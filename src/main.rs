mod analyzer;
mod camera;
mod notifier;
mod ui;

use camera::record::CameraApp;
use nokhwa::Camera;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{ApiBackend, RequestedFormat, RequestedFormatType};

fn find_working_camera() -> anyhow::Result<nokhwa::utils::CameraInfo> {
    let devices = nokhwa::query(ApiBackend::Auto)?;
    if devices.is_empty() {
        anyhow::bail!("В системе не найдено ни одной камеры!");
    }

    println!("=== Обнаруженные устройства камеры ===");
    for dev in &devices {
        println!("  • {:?} (индекс: {:?})", dev.human_name(), dev.index());
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
                    println!(
                        "✓ Успешно подключено к: {:?} ({:?})",
                        dev.human_name(),
                        dev.index()
                    );
                    return Ok(dev.clone());
                }
            }
        }
        println!(
            "  × Устройство {:?} пропущено (служебный узел)",
            dev.index()
        );
    }

    anyhow::bail!("Не удалось запустить видео ни с одной камеры.");
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let camera_info = find_working_camera()?;
    let camera = CameraApp::new(&camera_info);
    let frames = camera.receiver.clone();

    let analyzer = analyzer::Analyzer::new(frames)?;

    eframe::run_native(
        "Blink Guard",
        eframe::NativeOptions::default(),
        Box::new(move |_cc| Box::new(ui::App::new(analyzer)) as Box<dyn eframe::App>),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))
}
