use crate::analyzer::{Analyzer, AnnotatedFrame, BlinkEvent};
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::time::Instant;

pub struct App {
    analyzer: Analyzer,
    texture: Option<TextureHandle>,
    last_event: Option<BlinkEvent>,
    frame_count: u32,
    fps: f32,
    fps_window_start: Instant,
    blink_total: u32,
}

impl App {
    pub fn new(analyzer: Analyzer) -> Self {
        Self {
            analyzer,
            texture: None,
            last_event: None,
            frame_count: 0,
            fps: 0.0,
            fps_window_start: Instant::now(),
            blink_total: 0,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Ok(annotated) = self.analyzer.preview.try_recv() {
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
            if let BlinkEvent::Blink { total } = ev {
                self.blink_total = total;
            }
            self.last_event = Some(ev);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label(format!("FPS: {:.1}", self.fps));
            ui.label(format!("Морганий: {}", self.blink_total));

            if let Some(tex) = &self.texture {
                ui.image(tex);
            }
            ui.label(format!("{:?}", self.last_event));
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}
