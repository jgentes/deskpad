use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub text: String,
    pub done: bool,
    pub created_at: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub ics_url: String,
    pub google_client_id: String,
    pub google_client_secret: String,
    pub always_on_top: bool,
    pub reserve_space: bool,
    pub autostart: bool,
    pub width: u32,
    pub lookahead_hours: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ics_url: String::new(),
            google_client_id: String::new(),
            google_client_secret: String::new(),
            always_on_top: false,
            reserve_space: true,
            autostart: false,
            width: 340,
            lookahead_hours: 36,
        }
    }
}

pub struct Store {
    pub dir: PathBuf,
    pub tasks: Mutex<Vec<Task>>,
    pub settings: Mutex<Settings>,
}

impl Store {
    pub fn load(dir: PathBuf) -> Self {
        fs::create_dir_all(&dir).ok();
        let tasks = fs::read_to_string(dir.join("tasks.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let settings = fs::read_to_string(dir.join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { dir, tasks: Mutex::new(tasks), settings: Mutex::new(settings) }
    }

    pub fn save_tasks(&self) {
        let tasks = self.tasks.lock().unwrap();
        if let Ok(json) = serde_json::to_string_pretty(&*tasks) {
            fs::write(self.dir.join("tasks.json"), json).ok();
        }
    }

    pub fn save_settings(&self) {
        let settings = self.settings.lock().unwrap();
        if let Ok(json) = serde_json::to_string_pretty(&*settings) {
            fs::write(self.dir.join("settings.json"), json).ok();
        }
    }
}
