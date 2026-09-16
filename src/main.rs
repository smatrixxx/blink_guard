mod analyzer;
mod camera;
mod ui;

use camera::record::CameraApp;
use nokhwa::utils::ApiBackend;

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let devices = nokhwa::query(ApiBackend::Auto)?;
    let camera_info = devices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no camera found"))?;

    let camera = CameraApp::new(&camera_info);
    let frames = camera.receiver.clone();

    let analyzer = analyzer::Analyzer::new(
        frames,
        "models/face_detector.onnx",
        "models/face_landmarks.onnx",
    )?;

    eframe::run_native(
        "Blink Guard",
        eframe::NativeOptions::default(),
        Box::new(move |_cc| Box::new(ui::App::new(analyzer)) as Box<dyn eframe::App>),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))
}
