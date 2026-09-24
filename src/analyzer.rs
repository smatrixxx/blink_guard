pub mod ear;
pub mod face_detector;
pub mod gpu;
pub mod landmarks;

pub use face_detector::{BBox, FaceDetector};
pub use landmarks::LandmarkModel;

use crate::camera::record::FrameData;
use crate::config::CalibrationData;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalibrationStep {
    WaitingForFace,
    EyesOpen,
    EyesClosed,
}

#[derive(Debug, Clone)]
pub enum BlinkEvent {
    Blink {
        total: u32,
        duration_ms: u64,
        blinks_per_min: f32,
    },
    StareAlert {
        seconds_without_blink: u64,
    },
    FaceLost,
    CalibrationProgress {
        step: CalibrationStep,
        progress: f32,
    },
    CalibrationDone {
        open_ear: f32,
        closed_ear: f32,
        threshold: f32,
    },
    Error(String),
}

pub enum AnalyzerCommand {
    Recalibrate,
}

pub struct AnnotatedFrame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

pub struct Analyzer {
    pub events: Receiver<BlinkEvent>,
    pub preview: Receiver<AnnotatedFrame>,
    command_tx: Sender<AnalyzerCommand>,
    running: Arc<AtomicBool>,
}

enum InternalState {
    WaitingForFace,
    CalibratingOpen {
        start: Instant,
        samples: Vec<f32>,
    },
    CalibratingClosed {
        start: Instant,
        open_ear: f32,
        samples: Vec<f32>,
    },
    Running {
        dynamic_baseline: f32,
        threshold_ratio: f32,
    },
}

fn calculate_median(mut vals: Vec<f32>) -> f32 {
    if vals.is_empty() {
        return 0.25;
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    vals[vals.len() / 2]
}

// 2D расчет EAR: верхнее и нижнее веко реально смыкаются в 0
fn compute_ear_2d(eye: &[(f32, f32, f32)]) -> f32 {
    let dist =
        |a: (f32, f32, f32), b: (f32, f32, f32)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    let vertical = dist(eye[1], eye[5]) + dist(eye[2], eye[4]);
    let horizontal = dist(eye[0], eye[3]);
    if horizontal < 1e-4 {
        return 0.0;
    }
    vertical / (2.0 * horizontal)
}

impl Analyzer {
    pub fn new(
        frames: Receiver<FrameData>,
        saved_calib: Option<CalibrationData>,
    ) -> anyhow::Result<Self> {
        let mut detector = FaceDetector::new()?;
        let mut landmarker = LandmarkModel::new()?;

        let (event_tx, event_rx) = bounded::<BlinkEvent>(16);
        let (preview_tx, preview_rx) = bounded::<AnnotatedFrame>(1);
        let (command_tx, command_rx) = bounded::<AnalyzerCommand>(8);

        let running = Arc::new(AtomicBool::new(true));
        let running_thread = running.clone();

        let mut cached_bbox: Option<BBox> = None;
        let mut frames_since_detect: u32 = 0;

        let mut blink_count: u32 = 0;
        let mut close_start: Option<Instant> = None;
        let mut last_blink_time = Instant::now();
        let mut blink_history: VecDeque<Instant> = VecDeque::new();
        let mut stare_alert_sent = false;

        const STARE_TIMEOUT: Duration = Duration::from_secs(12);
        const MIN_BLINK_MS: u128 = 40; // Достаточно для фиксации даже сверхбыстрых морганий
        const MAX_BLINK_MS: u128 = 400;

        const REDETECT_EVERY: u32 = 10;
        const BBOX_SMOOTHING: f32 = 0.70;

        let initial_state = if let Some(calib) = saved_calib {
            let ratio = if calib.open_ear > 1e-4 {
                calib.threshold / calib.open_ear
            } else {
                0.72
            };
            InternalState::Running {
                dynamic_baseline: calib.open_ear,
                threshold_ratio: ratio.clamp(0.60, 0.82),
            }
        } else {
            InternalState::WaitingForFace
        };

        thread::spawn(move || {
            let mut state = initial_state;

            while running_thread.load(Ordering::Relaxed) {
                while let Ok(cmd) = command_rx.try_recv() {
                    match cmd {
                        AnalyzerCommand::Recalibrate => {
                            state = InternalState::WaitingForFace;
                            cached_bbox = None;
                            close_start = None;
                            last_blink_time = Instant::now();
                            stare_alert_sent = false;
                        }
                    }
                }

                let frame = match frames.recv() {
                    Ok(f) => f,
                    Err(_) => break,
                };

                // ОБРАБАТЫВАЕМ КАЖДЫЙ КАДР (полные 30 FPS без пропуска!)

                let Some(mut rgb) = image::RgbImage::from_raw(
                    frame.width as u32,
                    frame.height as u32,
                    frame.pixels,
                ) else {
                    let _ = event_tx.try_send(BlinkEvent::Error("bad frame buffer".into()));
                    continue;
                };

                let need_redetect = cached_bbox.is_none() || frames_since_detect >= REDETECT_EVERY;

                let bbox = if need_redetect {
                    frames_since_detect = 0;

                    match detector.detect(&rgb) {
                        Ok(Some(new_bbox)) => {
                            let smoothed = if let Some(prev) = &cached_bbox {
                                BBox {
                                    x1: prev.x1 * BBOX_SMOOTHING
                                        + new_bbox.x1 * (1.0 - BBOX_SMOOTHING),
                                    y1: prev.y1 * BBOX_SMOOTHING
                                        + new_bbox.y1 * (1.0 - BBOX_SMOOTHING),
                                    x2: prev.x2 * BBOX_SMOOTHING
                                        + new_bbox.x2 * (1.0 - BBOX_SMOOTHING),
                                    y2: prev.y2 * BBOX_SMOOTHING
                                        + new_bbox.y2 * (1.0 - BBOX_SMOOTHING),
                                }
                            } else {
                                new_bbox
                            };

                            cached_bbox = Some(smoothed.clone());
                            smoothed
                        }
                        Ok(None) => {
                            cached_bbox = None;
                            close_start = None;
                            last_blink_time = Instant::now();
                            stare_alert_sent = false;
                            let _ = event_tx.try_send(BlinkEvent::FaceLost);

                            let rgba_out = image::DynamicImage::ImageRgb8(rgb).to_rgba8();
                            let _ = preview_tx.try_send(AnnotatedFrame {
                                width: rgba_out.width() as usize,
                                height: rgba_out.height() as usize,
                                pixels: rgba_out.into_raw(),
                            });
                            continue;
                        }
                        Err(e) => {
                            eprintln!("detector error: {e}");
                            continue;
                        }
                    }
                } else {
                    frames_since_detect += 1;
                    cached_bbox.clone().unwrap()
                };

                let all_points = match landmarker.predict(&rgb, &bbox) {
                    Ok(Some(p)) => p,
                    Ok(None) => {
                        cached_bbox = None;
                        close_start = None;
                        last_blink_time = Instant::now();
                        stare_alert_sent = false;
                        let _ = event_tx.try_send(BlinkEvent::FaceLost);

                        let rgba_out = image::DynamicImage::ImageRgb8(rgb).to_rgba8();
                        let _ = preview_tx.try_send(AnnotatedFrame {
                            width: rgba_out.width() as usize,
                            height: rgba_out.height() as usize,
                            pixels: rgba_out.into_raw(),
                        });
                        continue;
                    }
                    Err(e) => {
                        eprintln!("landmark error: {e}");
                        continue;
                    }
                };

                // ВЫЧИСЛЕНИЕ EAR БЕЗ СГЛАЖИВАНИЯ (мгновенная реакция век!)
                const RIGHT_EYE_IDX: [usize; 6] = [33, 160, 158, 133, 153, 144];
                const LEFT_EYE_IDX: [usize; 6] = [263, 387, 385, 362, 380, 373];

                fn pick_eye(all: &[(f32, f32, f32)], idx: &[usize; 6]) -> [(f32, f32, f32); 6] {
                    [
                        all[idx[0]],
                        all[idx[1]],
                        all[idx[2]],
                        all[idx[3]],
                        all[idx[4]],
                        all[idx[5]],
                    ]
                }

                let left_eye = pick_eye(&all_points, &LEFT_EYE_IDX);
                let right_eye = pick_eye(&all_points, &RIGHT_EYE_IDX);
                let ear_left = compute_ear_2d(&left_eye);
                let ear_right = compute_ear_2d(&right_eye);

                // Усреднение для устойчивости к шуму
                let ear = (ear_left + ear_right) / 2.0;

                // Отрисовка точек
                for (i, &(x, y, _z)) in all_points.iter().enumerate() {
                    let color = if LEFT_EYE_IDX.contains(&i) {
                        image::Rgb([255, 60, 60])
                    } else if RIGHT_EYE_IDX.contains(&i) {
                        image::Rgb([60, 100, 255])
                    } else {
                        image::Rgb([0, 255, 0])
                    };
                    imageproc::drawing::draw_filled_circle_mut(
                        &mut rgb,
                        (x as i32, y as i32),
                        1,
                        color,
                    );
                }

                let now = Instant::now();

                match &mut state {
                    InternalState::WaitingForFace => {
                        let _ = event_tx.try_send(BlinkEvent::CalibrationProgress {
                            step: CalibrationStep::WaitingForFace,
                            progress: 0.0,
                        });
                        state = InternalState::CalibratingOpen {
                            start: now,
                            samples: Vec::with_capacity(90),
                        };
                    }

                    InternalState::CalibratingOpen { start, samples } => {
                        samples.push(ear);
                        let elapsed = start.elapsed().as_secs_f32();
                        let target = 3.0;

                        let _ = event_tx.try_send(BlinkEvent::CalibrationProgress {
                            step: CalibrationStep::EyesOpen,
                            progress: (elapsed / target).min(1.0),
                        });

                        if elapsed >= target && !samples.is_empty() {
                            let open_ear = calculate_median(std::mem::take(samples));
                            state = InternalState::CalibratingClosed {
                                start: now,
                                open_ear,
                                samples: Vec::with_capacity(60),
                            };
                        }
                    }

                    InternalState::CalibratingClosed {
                        start,
                        open_ear,
                        samples,
                    } => {
                        samples.push(ear);
                        let elapsed = start.elapsed().as_secs_f32();
                        let target = 2.0;

                        let _ = event_tx.try_send(BlinkEvent::CalibrationProgress {
                            step: CalibrationStep::EyesClosed,
                            progress: (elapsed / target).min(1.0),
                        });

                        if elapsed >= target && !samples.is_empty() {
                            let closed_ear = calculate_median(std::mem::take(samples));
                            let open = *open_ear;

                            let (valid_open, valid_closed) = if open > closed_ear + 0.05 {
                                (open, closed_ear)
                            } else {
                                (0.28, 0.12)
                            };

                            // Порог ставится на 40% расстояния от закрытого к открытому
                            let initial_threshold =
                                valid_closed + (valid_open - valid_closed) * 0.40;
                            let threshold_ratio =
                                (initial_threshold / valid_open).clamp(0.60, 0.80);

                            let _ = event_tx.try_send(BlinkEvent::CalibrationDone {
                                open_ear: valid_open,
                                closed_ear: valid_closed,
                                threshold: initial_threshold,
                            });

                            last_blink_time = now;
                            state = InternalState::Running {
                                dynamic_baseline: valid_open,
                                threshold_ratio,
                            };
                        }
                    }

                    InternalState::Running {
                        dynamic_baseline,
                        threshold_ratio,
                    } => {
                        let threshold = *dynamic_baseline * (*threshold_ratio);

                        if ear < threshold {
                            if close_start.is_none() {
                                close_start = Some(now);
                            } else if let Some(start) = close_start {
                                // Если закрыт дольше 400 мс — человек наклонил голову или щурится
                                if now.duration_since(start).as_millis() > 400 {
                                    close_start = None;
                                    *dynamic_baseline += (ear - *dynamic_baseline) * 0.15;
                                    *dynamic_baseline = dynamic_baseline.clamp(0.16, 0.42);
                                }
                            }
                        } else {
                            if let Some(start) = close_start.take() {
                                let duration_ms = now.duration_since(start).as_millis();

                                if (MIN_BLINK_MS..=MAX_BLINK_MS).contains(&duration_ms) {
                                    blink_count += 1;
                                    last_blink_time = now;
                                    stare_alert_sent = false;

                                    blink_history.push_back(now);
                                    while let Some(&t) = blink_history.front() {
                                        if now.duration_since(t) > Duration::from_secs(60) {
                                            blink_history.pop_front();
                                        } else {
                                            break;
                                        }
                                    }

                                    let bpm = blink_history.len() as f32;

                                    let _ = event_tx.try_send(BlinkEvent::Blink {
                                        total: blink_count,
                                        duration_ms: duration_ms as u64,
                                        blinks_per_min: bpm,
                                    });
                                }
                            }

                            // Плавная адаптация базы при открытых глазах
                            *dynamic_baseline += (ear - *dynamic_baseline) * 0.04;
                            *dynamic_baseline = dynamic_baseline.clamp(0.18, 0.42);
                        }

                        let time_since_blink = now.duration_since(last_blink_time);
                        if time_since_blink >= STARE_TIMEOUT && !stare_alert_sent {
                            stare_alert_sent = true;
                            let _ = event_tx.try_send(BlinkEvent::StareAlert {
                                seconds_without_blink: time_since_blink.as_secs(),
                            });
                        }

                        while let Some(&t) = blink_history.front() {
                            if now.duration_since(t) > Duration::from_secs(60) {
                                blink_history.pop_front();
                            } else {
                                break;
                            }
                        }
                    }
                }

                let rgba_out = image::DynamicImage::ImageRgb8(rgb).to_rgba8();
                let _ = preview_tx.try_send(AnnotatedFrame {
                    width: rgba_out.width() as usize,
                    height: rgba_out.height() as usize,
                    pixels: rgba_out.into_raw(),
                });
            }
        });

        Ok(Self {
            events: event_rx,
            preview: preview_rx,
            command_tx,
            running,
        })
    }

    pub fn recalibrate(&self) {
        let _ = self.command_tx.try_send(AnalyzerCommand::Recalibrate);
    }
}

impl Drop for Analyzer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
