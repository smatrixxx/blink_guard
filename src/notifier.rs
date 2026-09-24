use notify_rust::Notification;
use rodio::source::{SineWave, Source};
use rodio::{OutputStream, OutputStreamHandle, Sink};
use std::time::Duration;

pub struct Notifier {
    _stream: Option<OutputStream>,
    stream_handle: Option<OutputStreamHandle>,
}

impl Notifier {
    pub fn new() -> Self {
        match OutputStream::try_default() {
            Ok((stream, handle)) => Self {
                _stream: Some(stream),
                stream_handle: Some(handle),
            },
            Err(e) => {
                eprintln!("Audio device unavailable: {e}");
                Self {
                    _stream: None,
                    stream_handle: None,
                }
            }
        }
    }

    pub fn play_chime(&self) {
        if let Some(handle) = &self.stream_handle {
            if let Ok(sink) = Sink::try_new(handle) {
                let tone1 = SineWave::new(880.0)
                    .take_duration(Duration::from_millis(70))
                    .amplify(0.12);
                let tone2 = SineWave::new(1108.7)
                    .take_duration(Duration::from_millis(150))
                    .amplify(0.12);

                sink.append(tone1);
                sink.append(tone2);
                sink.detach();
            }
        }
    }

    pub fn send_stare_alert(seconds: u64) {
        std::thread::spawn(move || {
            let _ = Notification::new()
                .appname("Blink Guard")
                .summary("Пора поморгать")
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
                    "Частота всего {bpm:.1} морганий/мин (норма: 12-20).\nНе забывайте моргать чаще."
                ))
                .icon("dialog-warning")
                .timeout(4000)
                .show();
        });
    }
}
