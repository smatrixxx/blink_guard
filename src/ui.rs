use crate::analyzer::{Analyzer, BlinkEvent, CalibrationStep};
use crate::notifier::Notifier;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::time::Instant;

pub struct App {
    analyzer: Analyzer,
    notifier: Notifier,
    texture: Option<TextureHandle>,
    frame_count: u32,
    fps: f32,
    fps_window_start: Instant,
    blink_total: u32,
    blinks_per_min: f32,
    face_in_frame: bool,
    stare_warning: Option<u64>,
    calibration_status: Option<(CalibrationStep, f32)>,
    calibration_result: Option<(f32, f32, f32)>,
    sound_enabled: bool,
    notifications_enabled: bool,
}

impl App {
    pub fn new(analyzer: Analyzer) -> Self {
        Self {
            analyzer,
            notifier: Notifier::new(),
            texture: None,
            frame_count: 0,
            fps: 0.0,
            fps_window_start: Instant::now(),
            blink_total: 0,
            blinks_per_min: 0.0,
            face_in_frame: false,
            stare_warning: None,
            calibration_status: None,
            calibration_result: None,
            sound_enabled: true,
            notifications_enabled: true,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
                    // Алерт выдаем только если лицо реально в кадре
                    if self.face_in_frame {
                        self.stare_warning = Some(seconds_without_blink);

                        if self.sound_enabled {
                            self.notifier.play_chime();
                        }
                        if self.notifications_enabled {
                            Notifier::send_stare_alert(seconds_without_blink);
                        }
                    }
                }
                BlinkEvent::FaceLost => {
                    self.face_in_frame = false;
                    self.stare_warning = None; // Снимаем алерт, если человек отошел
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
                    self.calibration_result = Some((open_ear, closed_ear, threshold));
                }
                _ => {}
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("FPS: {:.1}", self.fps));
                ui.separator();

                // Статус наличия человека перед камерой
                if self.face_in_frame {
                    ui.colored_label(egui::Color32::from_rgb(100, 220, 120), "● Человек в кадре");
                } else {
                    ui.colored_label(egui::Color32::from_rgb(200, 200, 100), "○ Нет лица");
                }

                ui.separator();
                ui.label(format!("Всего: {}", self.blink_total));
                ui.separator();

                let bpm_color = if self.blinks_per_min < 8.0 {
                    egui::Color32::from_rgb(255, 120, 100)
                } else {
                    egui::Color32::from_rgb(100, 220, 120)
                };
                ui.colored_label(
                    bpm_color,
                    format!("Частота: {:.0} / мин", self.blinks_per_min),
                );

                ui.separator();
                ui.checkbox(&mut self.sound_enabled, "🔔 Звук");
                ui.checkbox(&mut self.notifications_enabled, "💬 Уведомления");

                if ui.button("⟳ Калибровка").clicked() {
                    self.analyzer.recalibrate();
                }
            });

            // Предупреждение о долгом замирании
            if let Some(sec) = self.stare_warning {
                if self.face_in_frame {
                    ui.add_space(4.0);
                    egui::Frame::group(ui.style())
                        .fill(egui::Color32::from_rgb(90, 30, 30))
                        .show(ui, |ui| {
                            ui.colored_label(
                                egui::Color32::YELLOW,
                                format!(
                                    "⚠ Внимание: вы не моргали уже {} секунд! Поморгайте.",
                                    sec
                                ),
                            );
                        });
                }
            }

            // Калибровка
            if let Some((step, progress)) = self.calibration_status {
                ui.add_space(4.0);
                egui::Frame::group(ui.style()).show(ui, |ui| match step {
                    CalibrationStep::WaitingForFace => {
                        ui.colored_label(egui::Color32::YELLOW, "Ищем лицо в кадре...");
                    }
                    CalibrationStep::EyesOpen => {
                        ui.heading("Этап 1/2: Смотрите в экран естественно");
                        ui.add(egui::ProgressBar::new(progress).show_percentage());
                    }
                    CalibrationStep::EyesClosed => {
                        ui.heading("Этап 2/2: Закройте глаза");
                        ui.add(egui::ProgressBar::new(progress).show_percentage());
                    }
                });
            }

            if let Some(tex) = &self.texture {
                ui.add_space(4.0);
                ui.image(tex);
            }
        });

        if got_new_frame {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}
