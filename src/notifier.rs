use notify_rust::Notification;
use rodio::source::{SineWave, Source};
use rodio::{DeviceSinkBuilder, MixerDeviceSink};
use std::time::Duration;

pub struct Notifier {
    sink: Option<MixerDeviceSink>,
}

impl Notifier {
    pub fn new() -> Self {
        match DeviceSinkBuilder::open_default_sink() {
            Ok(sink) => Self { sink: Some(sink) },
            Err(e) => {
                eprintln!("Аудиоустройство недоступно: {e}");
                Self { sink: None }
            }
        }
    }

    /// Проигрывает гармоничный мягкий аккорд (A5 -> C#6)
    pub fn play_chime(&self) {
        if let Some(sink) = &self.sink {
            let tone1 = SineWave::new(880.0)
                .take_duration(Duration::from_millis(70))
                .amplify(0.12);
            let tone2 = SineWave::new(1108.7)
                .take_duration(Duration::from_millis(150))
                .amplify(0.12);

            let mixer = sink.mixer();
            mixer.add(tone1);
            mixer.add(tone2);
        }
    }

    /// Всплывающее системное уведомление
    pub fn send_stare_alert(seconds: u64) {
        std::thread::spawn(move || {
            let _ = Notification::new()
                .appname("Blink Guard")
                .summary("Пора поморгать! 👀")
                .body(&format!(
                    "Вы не моргали уже {seconds} сек.\nСделайте пару морганий для увлажнения глаз."
                ))
                .icon("dialog-information")
                .timeout(4000)
                .show();
        });
    }

    pub fn send_low_rate_alert(bpm: f32) {
        std::thread::spawn(move || {
            let _ = Notification::new()
                .appname("Blink Guard")
                .summary("Низкая частота моргания")
                .body(&format!(
                    "Частота всего {bpm:.1} морганий/мин (норма: 12-20).\nНе забывайте моргать чаще!"
                ))
                .icon("dialog-warning")
                .timeout(4000)
                .show();
        });
    }
}
