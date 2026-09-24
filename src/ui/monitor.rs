use crate::analyzer::CalibrationStep;
use crate::ui::app::{ActiveView, App};
use eframe::egui;

pub fn render_monitor(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(format!("FPS: {:.0}", app.fps));
        ui.separator();

        if app.face_in_frame {
            ui.colored_label(egui::Color32::from_rgb(100, 200, 100), "[В кадре]");
        } else {
            ui.colored_label(egui::Color32::from_rgb(180, 180, 100), "[Нет лица]");
        }

        ui.separator();
        ui.label(format!("Всего: {}", app.blink_total));
        ui.separator();

        let bpm_color = if app.blinks_per_min < 8.0 {
            egui::Color32::from_rgb(230, 100, 80)
        } else {
            egui::Color32::from_rgb(100, 200, 100)
        };
        ui.colored_label(bpm_color, format!("{:.0}/мин", app.blinks_per_min));
    });

    if let Some((step, progress)) = app.calibration_status {
        ui.add_space(4.0);
        egui::Frame::group(ui.style()).show(ui, |ui| match step {
            CalibrationStep::WaitingForFace => {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 180, 50),
                    "Ищем лицо в кадре...",
                );
            }
            CalibrationStep::EyesOpen => {
                ui.label("Этап 1/2: Смотрите в экран прямо (3 сек)");
                ui.add(egui::ProgressBar::new(progress).show_percentage());
            }
            CalibrationStep::EyesClosed => {
                ui.label("Этап 2/2: Закройте глаза (2 сек)");
                ui.add(egui::ProgressBar::new(progress).show_percentage());
            }
        });
    } else if !app.calibration_saved {
        ui.colored_label(
            egui::Color32::from_rgb(220, 180, 50),
            "Требуется первичная калибровка",
        );
    }

    if let Some(sec) = app.stare_warning {
        if app.face_in_frame {
            ui.add_space(3.0);
            ui.colored_label(
                egui::Color32::from_rgb(230, 80, 80),
                format!("Внимание: вы не моргали уже {} секунд!", sec),
            );
        }
    }

    if let Some(tex) = &app.texture {
        ui.add_space(4.0);
        let aspect = 3.0 / 4.0;
        let width = ui.available_width();
        let height = (width * aspect).min(ui.available_height() - 36.0);
        ui.image((tex.id(), egui::vec2(width, height)));
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("Калибровка").clicked() {
            app.analyzer.recalibrate();
        }
        if ui.button("Настройки").clicked() {
            app.active_view = ActiveView::Settings;
        }
    });
}
