use crossbeam_channel::{Receiver, bounded};
use nokhwa::Camera;
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{RequestedFormat, RequestedFormatType};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

pub struct FrameData {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

pub struct CameraApp {
    pub receiver: Receiver<FrameData>,
    running: Arc<AtomicBool>,
}

impl CameraApp {
    pub fn new(camera_info: &nokhwa::utils::CameraInfo) -> Self {
        let (sender, receiver) = bounded::<FrameData>(2);
        let running = Arc::new(AtomicBool::new(true));
        let running_thread = running.clone();

        let index = camera_info.index().clone();

        thread::spawn(move || {
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

            if let Err(e) = camera.open_stream() {
                eprintln!("Thread opening error: {e}");
                return;
            }

            let mut rgb_buffer = Vec::new();

            while running_thread.load(Ordering::Relaxed) {
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

        Self { receiver, running }
    }
}

impl Drop for CameraApp {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
