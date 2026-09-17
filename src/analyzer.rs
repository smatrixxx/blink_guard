pub mod ear;
pub mod face_detector;
pub mod gpu;
pub mod landmarks;

pub use ear::compute_ear;
pub use face_detector::{BBox, FaceDetector};
pub use landmarks::LandmarkModel;

use crate::camera::record::FrameData;
use crossbeam_channel::{Receiver, Sender, bounded};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
    FaceLost, // Человек отошел от камеры / лицо не найдено
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
        blink_delta: f32,      // Индивидуальная амплитуда моргания (open - closed)
        dynamic_baseline: f32, // Динамический уровень открытого глаза для текущей позы
    },
}

fn calculate_median(mut vals: Vec<f32>) -> f32 {
    if vals.is_empty() {
        return 0.25;
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    vals[vals.len() / 2]
}

impl Analyzer {
    pub fn new(
        frames: Receiver<FrameData>,
        detector_path: &str,
        landmark_path: &str,
    ) -> anyhow::Result<Self> {
        let mut detector = FaceDetector::new(detector_path)?;
        let mut landmarker = LandmarkModel::new(landmark_path)?;

        let (event_tx, event_rx) = bounded::<BlinkEvent>(16);
        let (preview_tx, preview_rx) = bounded::<AnnotatedFrame>(1);
        let (command_tx, command_rx) = bounded::<AnalyzerCommand>(8);

        let running = Arc::new(AtomicBool::new(true));
        let running_thread = running.clone();

        let mut smoothed_points: Option<Vec<(f32, f32)>> = None;
        let mut cached_bbox: Option<BBox> = None;
        let mut frames_since_detect: u32 = 0;

        // Переменные аналитики времени и морганий
        let mut blink_count: u32 = 0;
        let mut close_start: Option<Instant> = None;
        let mut last_blink_time = Instant::now();
        let mut blink_history: VecDeque<Instant> = VecDeque::new();
        let mut stare_alert_sent = false;

        const STARE_TIMEOUT: Duration = Duration::from_secs(12); // Порог замирания без моргания
        const MIN_BLINK_MS: u128 = 65; // Меньше 65 мс — шум детектора
        const MAX_BLINK_MS: u128 = 380; // Больше 380 мс — прищур или дремота

        // Настройки трекинга
        const REDETECT_EVERY: u32 = 6;
        const MAX_JUMP: f32 = 28.0;
        const SMOOTHING: f32 = 0.40;
        const BBOX_SMOOTHING: f32 = 0.68;

        thread::spawn(move || {
            let mut frame_counter: u32 = 0;
            let mut state = InternalState::WaitingForFace;

            while running_thread.load(Ordering::Relaxed) {
                // Обработка внешних команд (например, кнопка «Калибровка» в UI)
                while let Ok(cmd) = command_rx.try_recv() {
                    match cmd {
                        AnalyzerCommand::Recalibrate => {
                            state = InternalState::WaitingForFace;
                            smoothed_points = None;
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

                frame_counter = frame_counter.wrapping_add(1);
                if frame_counter % 2 != 0 {
                    continue; // Пропуск каждого второго кадра (~15 FPS)
                }

                let Some(mut rgb) = image::RgbImage::from_raw(
                    frame.width as u32,
                    frame.height as u32,
                    frame.pixels,
                ) else {
                    let _ = event_tx.try_send(BlinkEvent::Error("bad frame buffer".into()));
                    continue;
                };

                // === Умный трекинг лица ===
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
                            // ЛИЦО НЕ НАЙДЕНО: сбрасываем таймеры, чтобы не слать ложных алертов в пустую комнату
                            cached_bbox = None;
                            smoothed_points = None;
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

                // === Лендмарки лица ===
                let all_points = match landmarker.predict(&rgb, &bbox) {
                    Ok(Some(p)) => p,
                    Ok(None) => {
                        // СЕТКА СЪЕХАЛА / ЛИЦО ПОТЕРЯНО: сбрасываем таймеры
                        cached_bbox = None;
                        smoothed_points = None;
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

                // === Сглаживание точек ===
                let all_points = match &smoothed_points {
                    Some(prev) => {
                        let max_jump = prev
                            .iter()
                            .zip(all_points.iter())
                            .map(|(a, b)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt())
                            .fold(0.0_f32, f32::max);

                        if max_jump > MAX_JUMP {
                            all_points
                        } else {
                            all_points
                                .iter()
                                .zip(prev.iter())
                                .map(|(new, old)| {
                                    (
                                        old.0 + (new.0 - old.0) * SMOOTHING,
                                        old.1 + (new.1 - old.1) * SMOOTHING,
                                    )
                                })
                                .collect()
                        }
                    }
                    None => all_points,
                };

                smoothed_points = Some(all_points.clone());

                // === Вычисление EAR для обоих глаз ===
                const RIGHT_EYE_IDX: [usize; 6] = [33, 160, 158, 133, 153, 144];
                const LEFT_EYE_IDX: [usize; 6] = [263, 387, 385, 362, 380, 373];

                fn pick_eye(all: &[(f32, f32)], idx: &[usize; 6]) -> [(f32, f32); 6] {
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
                let ear_left = compute_ear(&left_eye);
                let ear_right = compute_ear(&right_eye);

                // ЗАЩИТА ОТ ПОВОРОТА (Yaw): при повороте головы один глаз оптически сплющивается,
                // но второй открыт. Пока хотя бы один открыт — человек не моргнул.
                let ear = ear_left.max(ear_right);

                // Отрисовка контуров глаз
                for (i, &(x, y)) in all_points.iter().enumerate() {
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

                // === Калибровка и работа трекера ===
                match &mut state {
                    InternalState::WaitingForFace => {
                        let _ = event_tx.try_send(BlinkEvent::CalibrationProgress {
                            step: CalibrationStep::WaitingForFace,
                            progress: 0.0,
                        });
                        state = InternalState::CalibratingOpen {
                            start: now,
                            samples: Vec::with_capacity(60),
                        };
                    }

                    InternalState::CalibratingOpen { start, samples } => {
                        samples.push(ear);
                        let elapsed = start.elapsed().as_secs_f32();
                        let target = 3.0; // 3 секунды держим открытыми

                        let _ = event_tx.try_send(BlinkEvent::CalibrationProgress {
                            step: CalibrationStep::EyesOpen,
                            progress: (elapsed / target).min(1.0),
                        });

                        if elapsed >= target && !samples.is_empty() {
                            let open_ear = calculate_median(std::mem::take(samples));
                            state = InternalState::CalibratingClosed {
                                start: now,
                                open_ear,
                                samples: Vec::with_capacity(40),
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
                        let target = 2.0; // 2 секунды держим закрытыми

                        let _ = event_tx.try_send(BlinkEvent::CalibrationProgress {
                            step: CalibrationStep::EyesClosed,
                            progress: (elapsed / target).min(1.0),
                        });

                        if elapsed >= target && !samples.is_empty() {
                            let closed_ear = calculate_median(std::mem::take(samples));
                            let open = *open_ear;

                            let (valid_open, valid_closed) = if open > closed_ear + 0.04 {
                                (open, closed_ear)
                            } else {
                                (0.30, 0.14) // безопасный fallback
                            };

                            let blink_delta = (valid_open - valid_closed).max(0.06);
                            let dynamic_baseline = valid_open;
                            let initial_threshold = dynamic_baseline - (blink_delta * 0.50);

                            let _ = event_tx.try_send(BlinkEvent::CalibrationDone {
                                open_ear: valid_open,
                                closed_ear: valid_closed,
                                threshold: initial_threshold,
                            });

                            last_blink_time = now;
                            state = InternalState::Running {
                                blink_delta,
                                dynamic_baseline,
                            };
                        }
                    }

                    InternalState::Running {
                        blink_delta,
                        dynamic_baseline,
                    } => {
                        let threshold = *dynamic_baseline - (*blink_delta * 0.50);

                        // === ДЕТЕКЦИЯ МОРГАНИЯ ПО ВРЕМЕНИ (Instant) ===
                        if ear < threshold {
                            if close_start.is_none() {
                                close_start = Some(now);
                            }
                        } else {
                            if let Some(start) = close_start.take() {
                                let duration_ms = now.duration_since(start).as_millis();

                                // Валидация физиологической длительности (65 - 380 мс)
                                if (MIN_BLINK_MS..=MAX_BLINK_MS).contains(&duration_ms) {
                                    blink_count += 1;
                                    last_blink_time = now;
                                    stare_alert_sent = false;

                                    // Скользящая история морганий за 60 секунд (BPM)
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

                            // Адаптация базы под позу головы (обновляем только когда глаза открыты)
                            if ear > threshold {
                                *dynamic_baseline += (ear - *dynamic_baseline) * 0.05;
                                *dynamic_baseline = dynamic_baseline.clamp(0.18, 0.45);
                            }
                        }

                        // === ПРОВЕРКА НА ЗАМИРАНИЕ ВЗГЛЯДА (Stare Alert) ===
                        let time_since_blink = now.duration_since(last_blink_time);
                        if time_since_blink >= STARE_TIMEOUT && !stare_alert_sent {
                            stare_alert_sent = true;
                            let _ = event_tx.try_send(BlinkEvent::StareAlert {
                                seconds_without_blink: time_since_blink.as_secs(),
                            });
                        }

                        // Очистка старых записей из истории
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
