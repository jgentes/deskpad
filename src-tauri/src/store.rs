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
        let items = load_json(&dir.join("tasks.json")).unwrap_or_default();
        let settings = load_json(&dir.join("settings.json")).unwrap_or_default();
        Self { dir, items: Mutex::new(items), settings: Mutex::new(settings) }
    }

    pub fn save_items(&self) {
        let items = self.items.lock().unwrap();
        if let Ok(json) = serde_json::to_string_pretty(&*items) {
            let path = self.dir.join("tasks.json");
            backup_daily(&path, &self.dir.join("backups"));
            write_atomic(&path, &json);
        }
    }

    pub fn save_settings(&self) {
        let settings = self.settings.lock().unwrap();
        if let Ok(json) = serde_json::to_string_pretty(&*settings) {
            write_atomic(&self.dir.join("settings.json"), &json);
        }
    }
}

/// Read and parse a JSON file. A file that exists but does not parse is moved
/// aside as `<name>.corrupt-<timestamp>` instead of being silently replaced by
/// defaults on the next save.
pub fn load_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(v) => Some(v),
        Err(err) => {
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let aside = path.with_file_name(format!(
                "{}.corrupt-{stamp}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("data.json")
            ));
            eprintln!("[store] {} failed to parse ({err}); moved to {}", path.display(), aside.display());
            fs::rename(path, &aside).ok();
            None
        }
    }
}

/// Write via a temp file and rename so a crash mid-write never leaves a
/// truncated file behind.
pub fn write_atomic(path: &std::path::Path, contents: &str) {
    let tmp = path.with_extension("json.tmp");
    if let Err(err) = fs::write(&tmp, contents).and_then(|_| fs::rename(&tmp, path)) {
        eprintln!("[store] failed to write {}: {err}", path.display());
    }
}

/// Keep the first version of the file seen each day in `backups/`, pruning to
/// the newest 30. Runs before every overwrite; cheap when today's copy exists.
pub fn backup_daily(path: &std::path::Path, backups: &std::path::Path) {
    if !path.exists() {
        return;
    }
    let stem = path.file_stem().and_then(|n| n.to_str()).unwrap_or("data");
    let today = chrono::Local::now().format("%Y-%m-%d");
    let dest = backups.join(format!("{stem}-{today}.json"));
    if dest.exists() {
        return;
    }
    fs::create_dir_all(backups).ok();
    fs::copy(path, &dest).ok();
    let mut old: Vec<_> = fs::read_dir(backups)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&format!("{stem}-"))))
                .collect()
        })
        .unwrap_or_default();
    old.sort();
    for p in old.iter().rev().skip(30) {
        fs::remove_file(p).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::{backup_daily, load_json, parse_marker, write_atomic};

    #[test]
    fn corrupt_file_is_set_aside_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("deskpad-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tasks.json");
        std::fs::write(&path, "{not json").unwrap();
        let loaded: Option<Vec<super::Item>> = load_json(&path);
        assert!(loaded.is_none());
        assert!(!path.exists(), "corrupt file should have been moved aside");
        let aside = std::fs::read_dir(&dir).unwrap().flatten().find(|e| e.file_name().to_string_lossy().starts_with("tasks.json.corrupt-"));
        assert!(aside.is_some());

        write_atomic(&path, "[]");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[]");
        backup_daily(&path, &dir.join("backups"));
        assert_eq!(std::fs::read_dir(dir.join("backups")).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

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
