use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// One outline row. `kind` is "note" or "task"; `parent` nests it one level
/// under another top-level item. Order in the vector is display order, with
/// children stored directly after their parent.
#[derive(Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub text: String,
    #[serde(default = "legacy_kind")]
    pub kind: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub parent: Option<u64>,
    pub created_at: i64,
}

/// Files written before notes existed only held tasks.
fn legacy_kind() -> String {
    "task".into()
}

impl Item {
    /// Build an item from raw text, honouring the task markers: a leading
    /// `- `, `[]` / `[ ]`, or `/task` anywhere in the text turns it into a task
    /// (the marker is stripped). `/note` forces a note.
    pub fn from_text(text: &str, parent: Option<u64>) -> Option<Self> {
        let (text, kind) = parse_marker(text);
        if text.is_empty() {
            return None;
        }
        let now = chrono::Utc::now();
        Some(Self {
            id: now.timestamp_millis() as u64,
            text,
            kind: kind.into(),
            done: false,
            parent,
            created_at: now.timestamp(),
        })
    }
}

/// Returns (clean text, kind).
pub fn parse_marker(text: &str) -> (String, &'static str) {
    let t = text.trim();
    let mut kind = "note";
    let mut t = t.to_string();
    for m in ["[ ]", "[]", "- ", "-"] {
        if let Some(rest) = t.strip_prefix(m) {
            if m == "-" && !rest.starts_with(' ') && !rest.is_empty() {
                continue;
            }
            t = rest.trim_start().to_string();
            kind = "task";
            break;
        }
    }
    if t.contains("/task") {
        t = t.replace("/task", "");
        kind = "task";
    }
    if t.contains("/note") {
        t = t.replace("/note", "");
        kind = "note";
    }
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    (t, kind)
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
    pub show_all_day: bool,
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
            show_all_day: true,
        }
    }
}

pub struct Store {
    pub dir: PathBuf,
    pub items: Mutex<Vec<Item>>,
    pub settings: Mutex<Settings>,
}

impl Store {
    pub fn load(dir: PathBuf) -> Self {
        fs::create_dir_all(&dir).ok();
        let items = fs::read_to_string(dir.join("tasks.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let settings = fs::read_to_string(dir.join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { dir, items: Mutex::new(items), settings: Mutex::new(settings) }
    }

    pub fn save_items(&self) {
        let items = self.items.lock().unwrap();
        if let Ok(json) = serde_json::to_string_pretty(&*items) {
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

#[cfg(test)]
mod tests {
    use super::parse_marker;

    #[test]
    fn markers() {
        assert_eq!(parse_marker("buy milk"), ("buy milk".into(), "note"));
        assert_eq!(parse_marker("- buy milk"), ("buy milk".into(), "task"));
        assert_eq!(parse_marker("[] buy milk"), ("buy milk".into(), "task"));
        assert_eq!(parse_marker("[ ] buy milk"), ("buy milk".into(), "task"));
        assert_eq!(parse_marker("buy milk /task"), ("buy milk".into(), "task"));
        assert_eq!(parse_marker("call /task mom"), ("call mom".into(), "task"));
        assert_eq!(parse_marker("-dash word"), ("-dash word".into(), "note"));
        assert_eq!(parse_marker("- x /note"), ("x".into(), "note"));
        assert_eq!(parse_marker("   "), ("".into(), "note"));
    }
}
