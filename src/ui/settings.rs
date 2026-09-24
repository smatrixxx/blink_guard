use crate::camera::record::CameraCommand;
use crate::ui::app::{ActiveView, App};
use eframe::egui;

pub fn render_settings(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.heading("Настройки");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Назад").clicked() {
                app.active_view = ActiveView::Monitor;
            }
        });
    });
    ui.separator();

    ui.label("Параметры веб-камеры:");
    if ui
        .add(egui::Slider::new(&mut app.config.brightness, 0..=255).text("Яркость"))
        .changed()
    {
        let _ = app
            .camera_tx
            .try_send(CameraCommand::SetBrightness(app.config.brightness));
        app.config.save();
    }
    if ui
        .add(egui::Slider::new(&mut app.config.contrast, 0..=255).text("Контрастность"))
        .changed()
    {
        let _ = app
            .camera_tx
            .try_send(CameraCommand::SetContrast(app.config.contrast));
        app.config.save();
    }

    ui.add_space(8.0);
    ui.separator();
    ui.label("Оповещения:");
    if ui
        .checkbox(&mut app.config.sound_enabled, "Звуковой сигнал")
        .changed()
    {
        app.config.save();
    }
    if ui
        .checkbox(
            &mut app.config.notifications_enabled,
            "Системные уведомления",
        )
        .changed()
    {
        app.config.save();
    }
    if ui
        .add(
            egui::Slider::new(&mut app.config.stare_timeout_secs, 8..=30)
                .suffix(" сек")
                .text("Замирание"),
        )
        .changed()
    {
        app.config.save();
    }

    ui.add_space(8.0);
    ui.separator();
    if ui.button("Сбросить калибровку").clicked() {
        app.config.calibration = None;
        app.config.save();
        app.calibration_saved = false;
        app.analyzer.recalibrate();
        app.active_view = ActiveView::Monitor;
    }
}
