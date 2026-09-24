use crate::camera::controls::{
    fix_camera_flicker, optimize_camera_for_tracking, select_optimal_format, set_brightness,
    set_contrast,
};
use crossbeam_channel::{bounded, Receiver, Sender};
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

pub enum CameraCommand {
    SetBrightness(i64),
    SetContrast(i64),
}

pub struct FrameData {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

pub struct CameraApp {
    pub receiver: Receiver<FrameData>,
    pub command_tx: Sender<CameraCommand>,
    running: Arc<AtomicBool>,
}

impl CameraApp {
    pub fn new(camera_info: &nokhwa::utils::CameraInfo) -> Self {
        let (sender, receiver) = bounded::<FrameData>(2);
        let (cmd_sender, cmd_receiver) = bounded::<CameraCommand>(8);
        let running = Arc::new(AtomicBool::new(true));
        let running_thread = running.clone();

        let index = camera_info.index().clone();
        let dev_path = match &index {
            CameraIndex::Index(idx) => format!("/dev/video{idx}"),
            CameraIndex::String(s) => s.clone(),
        };

        thread::spawn(move || {
            fix_camera_flicker(&dev_path);

            let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::None);

            let mut camera = match Camera::new(index.clone(), requested) {
                Ok(c) => c,
                Err(_) => {
                    let fallback = RequestedFormat::new::<RgbFormat>(
                        RequestedFormatType::AbsoluteHighestFrameRate,
                    );
                    match Camera::new(index, fallback) {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("Camera opening error: {e}");
                            return;
                        }
                    }
                }
            };

            if let Some(optimal) = select_optimal_format(&mut camera) {
                let _ = camera.set_camera_format(optimal);
            }
            optimize_camera_for_tracking(&mut camera);

            if let Err(e) = camera.open_stream() {
                eprintln!("Thread opening error: {e}");
                return;
            }

            let mut rgb_buffer = Vec::new();

            while running_thread.load(Ordering::Relaxed) {
                while let Ok(cmd) = cmd_receiver.try_recv() {
                    match cmd {
                        CameraCommand::SetBrightness(val) => {
                            let _ = set_brightness(&mut camera, val);
                        }
                        CameraCommand::SetContrast(val) => {
                            let _ = set_contrast(&mut camera, val);
                        }
                    }
                }

                match camera.frame() {
                    Ok(frame) => {
                        let res = frame.resolution();
                        let buffer_size = (res.width() * res.height() * 3) as usize;
                        if rgb_buffer.len() != buffer_size {
                            rgb_buffer.resize(buffer_size, 0);
                        }

                        if frame
                            .decode_image_to_buffer::<RgbFormat>(&mut rgb_buffer)
                            .is_ok()
                        {
                            let data = FrameData {
                                width: res.width() as usize,
                                height: res.height() as usize,
                                pixels: rgb_buffer.clone(),
                            };

                            let _ = sender.try_send(data);
                        }
                    }
                    Err(e) => {
                        eprintln!("Frame record error: {e}");
                        thread::sleep(std::time::Duration::from_millis(300));
                    }
                }
            }
        });

        Self {
            receiver,
            command_tx: cmd_sender,
            running,
        }
    }
}

impl Drop for CameraApp {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
