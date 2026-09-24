mod analyzer;
mod camera;
mod config;
mod notifier;
mod ui;

use camera::record::CameraApp;
use config::AppConfig;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{ApiBackend, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;

fn find_working_camera() -> anyhow::Result<nokhwa::utils::CameraInfo> {
    let devices = nokhwa::query(ApiBackend::Auto)?;
    if devices.is_empty() {
        anyhow::bail!("В системе не найдено ни одной камеры!");
    }

    for dev in &devices {
        let index = dev.index().clone();
        let formats_to_try = [
            RequestedFormatType::None,
            RequestedFormatType::AbsoluteHighestFrameRate,
        ];

        for fmt in formats_to_try {
            let requested = RequestedFormat::new::<RgbFormat>(fmt);
            if let Ok(mut cam) = Camera::new(index.clone(), requested) {
                if cam.open_stream().is_ok() {
                    let _ = cam.stop_stream();
                    return Ok(dev.clone());
                }
            }
        }
    }

    anyhow::bail!("Не удалось запустить видео ни с одной камеры.");
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let config = AppConfig::load();

    let camera_info = find_working_camera()?;
    let camera = CameraApp::new(&camera_info);
    let frames = camera.receiver.clone();
    let camera_tx = camera.command_tx.clone();

    let analyzer = analyzer::Analyzer::new(frames, config.calibration.clone())?;

    let start_visible = config.calibration.is_none() || !config.start_in_tray;

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Blink Guard")
            .with_inner_size([420.0, 360.0])
            .with_min_inner_size([360.0, 280.0])
            .with_visible(start_visible),
        ..Default::default()
    };

    eframe::run_native(
        "Blink Guard",
        native_options,
        Box::new(move |_cc| {
            Box::new(ui::App::new(analyzer, camera_tx, config)) as Box<dyn eframe::App>
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))
}
