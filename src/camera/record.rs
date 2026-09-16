use crossbeam_channel::{Receiver, bounded};
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use nokhwa::Camera;
use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{CameraFormat, FrameFormat, RequestedFormat, RequestedFormatType, Resolution};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

// structure send frame
pub struct FrameData {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

pub struct CameraApp {
    pub receiver: Receiver<FrameData>,
    texture: Option<TextureHandle>,
    running: Arc<AtomicBool>,
}

impl CameraApp {
    pub fn new(camera_info: &nokhwa::utils::CameraInfo) -> Self {
        // channel with delay in 1-2 frames, to not save up delay
        let (sender, receiver) = bounded::<FrameData>(2);
        let running = Arc::new(AtomicBool::new(true));
        let running_thread = running.clone();

        let index = nokhwa::utils::CameraIndex::Index(0);

        // start record in separate system thread
        thread::spawn(move || {
            let requested = RequestedFormat::new::<RgbAFormat>(RequestedFormatType::Closest(
                CameraFormat::new(Resolution::new(640, 480), FrameFormat::MJPEG, 30),
            ));
            let mut camera = match Camera::new(index, requested) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Camera opening error: {e}");
                    return;
                }
            };

            if let Err(e) = camera.open_stream() {
                eprintln!("Thread opening error: {e}");
                return;
            }

            while running_thread.load(Ordering::Relaxed) {
                match camera.frame() {
                    Ok(frame) => {
                        let res = frame.resolution();
                        // декодируем кадр напрямую в сырой буфер Rgba
                        let mut rgba_buffer = vec![0u8; (res.width() * res.height() * 4) as usize];
                        if frame
                            .decode_image_to_buffer::<RgbAFormat>(&mut rgba_buffer)
                            .is_ok()
                        {
                            let data = FrameData {
                                width: res.width() as usize,
                                height: res.height() as usize,
                                pixels: rgba_buffer,
                            };

                            // if ui doesnt have enough time to catch frames, pull out old
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
            texture: None,
            running,
        }
    }
}

impl Drop for CameraApp {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
