pub mod ear;
pub mod face_detector;
pub mod gpu;
pub mod landmarks;

pub use ear::compute_ear;
pub use face_detector::{BBox, FaceDetector};
pub use landmarks::LandmarkModel;

use crate::camera::record::FrameData;
use crossbeam_channel::{Receiver, bounded};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

#[derive(Debug, Clone)]
pub enum BlinkEvent {
    Blink { total: u32 },
    LowRateWarning { blinks_per_min: f32 },
    Error(String),
}

pub struct AnnotatedFrame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

pub struct Analyzer {
    pub events: Receiver<BlinkEvent>,
    pub preview: Receiver<AnnotatedFrame>,
    running: Arc<AtomicBool>,
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
        let running = Arc::new(AtomicBool::new(true));
        let running_thread = running.clone();

        let mut smoothed_points: Option<Vec<(f32, f32)>> = None;
        let mut cached_bbox: Option<BBox> = None;
        let mut frames_since_detect: u32 = 0;
        let mut eye_closed = false;
        let mut blink_count: u32 = 0;
        let mut consecutive_low: u32 = 0;
        let mut ear_baseline: f32 = 0.25;

        // === Настройки трекинга ===
        const REDETECT_EVERY: u32 = 6; // как часто принудительно передетектить
        const MAX_JUMP: f32 = 28.0; // максимальный скачок точек
        const SMOOTHING: f32 = 0.42; // сглаживание точек
        const BBOX_SMOOTHING: f32 = 0.68; // сглаживание bbox (выше = инертнее)
        const BASELINE_ADAPT_RATE: f32 = 0.005; // как быстро база подстраивается к новым макс. значениям
        const BLINK_RATIO: f32 = 0.80; // порог = 68% от базового уровня
        const EAR_PLAUSIBLE_MAX: f32 = 0.38; // выше — трекинг съехал, не доверяем
        const CONSEC_FRAMES: u32 = 2; // сколько кадров подряд ниже порога, чтобы засчитать моргание

        thread::spawn(move || {
            let mut frame_counter: u32 = 0;

            while running_thread.load(Ordering::Relaxed) {
                let frame = match frames.recv() {
                    Ok(f) => f,
                    Err(_) => break,
                };

                frame_counter = frame_counter.wrapping_add(1);
                if frame_counter % 2 != 0 {
                    continue;
                }

                let Some(rgba) = image::RgbaImage::from_raw(
                    frame.width as u32,
                    frame.height as u32,
                    frame.pixels,
                ) else {
                    let _ = event_tx.try_send(BlinkEvent::Error("bad frame buffer".into()));
                    continue;
                };
                let mut rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();

                // === Умный трекинг bbox ===
                let need_redetect = cached_bbox.is_none() || frames_since_detect >= REDETECT_EVERY;

                let bbox = if need_redetect {
                    frames_since_detect = 0;

                    match detector.detect(&rgb) {
                        Ok(Some(new_bbox)) => {
                            // Сглаживаем новый bbox с предыдущим
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
                            smoothed_points = None;
                            let rgba_out = image::DynamicImage::ImageRgb8(rgb.clone()).to_rgba8();
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

                // === Лендмарки ===
                let all_points = match landmarker.predict(&rgb, &bbox) {
                    Ok(Some(p)) => p,
                    Ok(None) => {
                        cached_bbox = None;
                        smoothed_points = None;
                        let rgba_out = image::DynamicImage::ImageRgb8(rgb.clone()).to_rgba8();
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
                            // Резкий скачок — берём новые точки как есть
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

                // === Глаза и EAR ===
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
                let ear = (compute_ear(&left_eye) + compute_ear(&right_eye)) / 2.0;

                for (i, &(x, y)) in all_points.iter().enumerate() {
                    let color = if LEFT_EYE_IDX.contains(&i) {
                        image::Rgb([255, 0, 0]) // красный — предполагаемый левый глаз
                    } else if RIGHT_EYE_IDX.contains(&i) {
                        image::Rgb([0, 0, 255]) // синий — предполагаемый правый глаз
                    } else {
                        image::Rgb([0, 255, 0]) // зелёный — все остальные точки лица
                    };
                    imageproc::drawing::draw_filled_circle_mut(
                        &mut rgb,
                        (x as i32, y as i32),
                        1,
                        color,
                    );
                }

                // === Адаптивная база EAR ===
                if ear > ear_baseline && ear < EAR_PLAUSIBLE_MAX {
                    ear_baseline = ear_baseline + (ear - ear_baseline) * BASELINE_ADAPT_RATE;
                } else if ear <= ear_baseline {
                    ear_baseline = ear_baseline * 0.999;
                }

                let threshold = ear_baseline * BLINK_RATIO;

                // === Детекция моргания (hysteresis + consecutive frames) ===
                if ear < threshold {
                    consecutive_low += 1;
                    if consecutive_low >= CONSEC_FRAMES && !eye_closed {
                        eye_closed = true;
                        blink_count += 1;
                        let _ = event_tx.try_send(BlinkEvent::Blink { total: blink_count });
                    }
                } else {
                    consecutive_low = 0;
                    eye_closed = false;
                }

                eprintln!("EAR: {ear:.4} baseline: {ear_baseline:.4} threshold: {threshold:.4}");

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
            running,
        })
    }
}

impl Drop for Analyzer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
