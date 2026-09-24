use crate::analyzer::{Analyzer, BlinkEvent, CalibrationStep};
use crate::camera::record::CameraCommand;
use crate::config::{AppConfig, CalibrationData};
use crate::notifier::Notifier;
use crate::ui::monitor::render_monitor;
use crate::ui::settings::render_settings;
use crossbeam_channel::Sender;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::time::Instant;

#[derive(PartialEq)]
pub enum ActiveView {
    Monitor,
    Settings,
}

pub struct App {
    pub analyzer: Analyzer,
    pub camera_tx: Sender<CameraCommand>,
    pub notifier: Notifier,
    pub config: AppConfig,
    pub texture: Option<TextureHandle>,
    pub frame_count: u32,
    pub fps: f32,
    pub fps_window_start: Instant,
    pub blink_total: u32,
    pub blinks_per_min: f32,
    pub face_in_frame: bool,
    pub stare_warning: Option<u64>,
    pub calibration_status: Option<(CalibrationStep, f32)>,
    pub calibration_saved: bool,
    pub active_view: ActiveView,
}

impl App {
    pub fn new(analyzer: Analyzer, camera_tx: Sender<CameraCommand>, config: AppConfig) -> Self {
        let has_calib = config.calibration.is_some();
        Self {
            analyzer,
            camera_tx,
            notifier: Notifier::new(),
            config,
            texture: None,
            frame_count: 0,
            fps: 0.0,
            fps_window_start: Instant::now(),
            blink_total: 0,
            blinks_per_min: 0.0,
            face_in_frame: false,
            stare_warning: None,
            calibration_status: None,
            calibration_saved: has_calib,
            active_view: ActiveView::Monitor,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(sys_theme) = _frame.info().system_theme {
            let is_dark = sys_theme == eframe::Theme::Dark;
            if ctx.style().visuals.dark_mode != is_dark {
                ctx.set_visuals(if is_dark {
                    egui::Visuals::dark()
                } else {
                    egui::Visuals::light()
                });
            }
        }

        let mut got_new_frame = false;

        if let Ok(annotated) = self.analyzer.preview.try_recv() {
            got_new_frame = true;
            self.frame_count += 1;

            let color_image = ColorImage::from_rgba_unmultiplied(
                [annotated.width, annotated.height],
                &annotated.pixels,
            );
            match &mut self.texture {
                Some(tex) => tex.set(color_image, TextureOptions::default()),
                None => {
                    self.texture = Some(ctx.load_texture(
                        "camera_preview",
                        color_image,
                        TextureOptions::default(),
                    ))
                }
            }
        }

        let elapsed = self.fps_window_start.elapsed();
        if elapsed.as_secs_f32() >= 1.0 {
            self.fps = self.frame_count as f32 / elapsed.as_secs_f32();
            self.frame_count = 0;
            self.fps_window_start = Instant::now();
        }

        while let Ok(ev) = self.analyzer.events.try_recv() {
            match ev {
                BlinkEvent::Blink {
                    total,
                    blinks_per_min,
                    ..
                } => {
                    self.face_in_frame = true;
                    self.blink_total = total;
                    self.blinks_per_min = blinks_per_min;
                    self.stare_warning = None;
                }
                BlinkEvent::StareAlert {
                    seconds_without_blink,
                } => {
                    if self.face_in_frame {
                        self.stare_warning = Some(seconds_without_blink);

                        if self.config.sound_enabled {
                            self.notifier.play_chime();
                        }
                        if self.config.notifications_enabled {
                            Notifier::send_stare_alert(seconds_without_blink);
                        }
                    }
                }
                BlinkEvent::FaceLost => {
                    self.face_in_frame = false;
                    self.stare_warning = None;
                }
                BlinkEvent::CalibrationProgress { step, progress } => {
                    if step != CalibrationStep::WaitingForFace {
                        self.face_in_frame = true;
                    }
                    self.calibration_status = Some((step, progress));
                }
                BlinkEvent::CalibrationDone {
                    open_ear,
                    closed_ear,
                    threshold,
                } => {
                    self.face_in_frame = true;
                    self.calibration_status = None;
                    self.calibration_saved = true;

                    self.config.calibration = Some(CalibrationData {
                        open_ear,
                        closed_ear,
                        threshold,
                    });
                    self.config.save();
                }
                _ => {}
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| match self.active_view {
            ActiveView::Monitor => render_monitor(self, ui),
            ActiveView::Settings => render_settings(self, ui),
        });

        if got_new_frame {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}
