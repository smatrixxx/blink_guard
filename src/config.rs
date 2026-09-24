use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationData {
    pub open_ear: f32,
    pub closed_ear: f32,
    pub threshold: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub sound_enabled: bool,
    pub notifications_enabled: bool,
    pub stare_timeout_secs: u64,
    pub start_in_tray: bool,
    pub brightness: i64,
    pub contrast: i64,
    pub calibration: Option<CalibrationData>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            sound_enabled: true,
            notifications_enabled: true,
            stare_timeout_secs: 12,
            start_in_tray: true,
            brightness: 128,
            contrast: 32,
            calibration: None,
        }
    }
}

impl AppConfig {
    fn get_path() -> Option<PathBuf> {
        ProjectDirs::from("", "", "blinkguard").map(|dirs| {
            let config_dir = dirs.config_dir();
            let _ = fs::create_dir_all(config_dir);
            config_dir.join("config.json")
        })
    }

    pub fn load() -> Self {
        if let Some(path) = Self::get_path() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(cfg) = serde_json::from_str::<Self>(&content) {
                    return cfg;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        if let Some(path) = Self::get_path() {
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = fs::write(path, json);
            }
        }
    }
}
