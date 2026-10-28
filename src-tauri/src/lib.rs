mod appbar;
mod calendar;
mod store;

use chrono::{Duration, Utc};
use std::sync::Mutex;
use store::{Settings, Store, Task};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

const CAPTURE_SHORTCUT: &str = "ctrl+space";

/// Cached calendar text so the UI can refresh often without hammering the feed.
struct CalendarCache {
    text: String,
    fetched_at: i64,
    url: String,
}

#[derive(Default)]
struct CalCacheState(Mutex<Option<CalendarCache>>);

// ---------- Tasks ----------

#[tauri::command]
fn list_tasks(store: State<Store>) -> Vec<Task> {
    store.tasks.lock().unwrap().clone()
}

#[tauri::command]
fn add_task(app: AppHandle, store: State<Store>, text: String) -> Option<Task> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return None;
    }
    let task = Task {
        id: Utc::now().timestamp_millis() as u64,
        text,
        done: false,
        created_at: Utc::now().timestamp(),
    };
    store.tasks.lock().unwrap().insert(0, task.clone());
    store.save_tasks();
    app.emit("tasks-changed", ()).ok();
    Some(task)
}

#[tauri::command]
fn toggle_task(app: AppHandle, store: State<Store>, id: u64) {
    {
        let mut tasks = store.tasks.lock().unwrap();
        if let Some(t) = tasks.iter_mut().find(|t| t.id == id) {
            t.done = !t.done;
        }
    }
    store.save_tasks();
    app.emit("tasks-changed", ()).ok();
}

#[tauri::command]
fn update_task(app: AppHandle, store: State<Store>, id: u64, text: String) {
    {
        let mut tasks = store.tasks.lock().unwrap();
        if let Some(t) = tasks.iter_mut().find(|t| t.id == id) {
            t.text = text.trim().to_string();
        }
    }
    store.save_tasks();
    app.emit("tasks-changed", ()).ok();
}

#[tauri::command]
fn delete_task(app: AppHandle, store: State<Store>, id: u64) {
    store.tasks.lock().unwrap().retain(|t| t.id != id);
    store.save_tasks();
    app.emit("tasks-changed", ()).ok();
}

#[tauri::command]
fn clear_done(app: AppHandle, store: State<Store>) {
    store.tasks.lock().unwrap().retain(|t| !t.done);
    store.save_tasks();
    app.emit("tasks-changed", ()).ok();
}

// ---------- Settings ----------

#[tauri::command]
fn get_settings(store: State<Store>) -> Settings {
    store.settings.lock().unwrap().clone()
}

#[tauri::command]
fn set_settings(app: AppHandle, store: State<Store>, settings: Settings) {
    let width_changed;
    {
        let mut s = store.settings.lock().unwrap();
        width_changed = s.width != settings.width || s.reserve_space != settings.reserve_space;
        if s.ics_url != settings.ics_url {
            // Invalidate the calendar cache.
            *app.state::<CalCacheState>().0.lock().unwrap() = None;
        }
        *s = settings.clone();
    }
    store.save_settings();
    if let Some(win) = app.get_webview_window("main") {
        win.set_always_on_top(settings.always_on_top).ok();
        if width_changed {
            dock(&win, &settings);
        }
    }
    app.emit("settings-changed", ()).ok();
}

// ---------- Calendar ----------

#[tauri::command]
async fn get_meetings(
    store: State<'_, Store>,
    cache: State<'_, CalCacheState>,
    force: bool,
) -> Result<Vec<calendar::Meeting>, String> {
    let (url, lookahead) = {
        let s = store.settings.lock().unwrap();
        (s.ics_url.clone(), s.lookahead_hours as i64)
    };
    if url.trim().is_empty() {
        return Ok(vec![]);
    }
    let now = Utc::now();
    let cached = {
        let c = cache.0.lock().unwrap();
        c.as_ref()
            .filter(|c| c.url == url && !force && now.timestamp() - c.fetched_at < 5 * 60)
            .map(|c| c.text.clone())
    };
    let text = match cached {
        Some(t) => t,
        None => {
            let t = calendar::fetch(&url).await?;
            *cache.0.lock().unwrap() = Some(CalendarCache {
                text: t.clone(),
                fetched_at: now.timestamp(),
                url: url.clone(),
            });
            t
        }
    };
    Ok(calendar::expand(&text, now, now + Duration::hours(lookahead)))
}

// ---------- Windows ----------

fn dock(win: &WebviewWindow, settings: &Settings) {
    #[cfg(windows)]
    {
        if let Ok(hwnd) = win.hwnd() {
            let scale = win.scale_factor().unwrap_or(1.0);
            let width_px = (settings.width as f64 * scale).round() as i32;
            let hwnd = windows::Win32::Foundation::HWND(hwnd.0);
            unsafe {
                appbar::imp::dock_right(hwnd, width_px, settings.reserve_space);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (win, settings);
    }
}

fn undock(win: &WebviewWindow) {
    #[cfg(windows)]
    {
        if let Ok(hwnd) = win.hwnd() {
            unsafe { appbar::imp::undock(windows::Win32::Foundation::HWND(hwnd.0)) }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = win;
    }
}

fn toggle_capture(app: &AppHandle) {
    let Some(win) = app.get_webview_window("capture") else { return };
    if win.is_visible().unwrap_or(false) && win.is_focused().unwrap_or(false) {
        win.hide().ok();
        return;
    }
    win.center().ok();
    win.show().ok();
    win.set_focus().ok();
    app.emit_to("capture", "capture-shown", ()).ok();
}

#[tauri::command]
fn hide_capture(app: AppHandle) {
    if let Some(win) = app.get_webview_window("capture") {
        win.hide().ok();
    }
}

#[tauri::command]
fn show_capture(app: AppHandle) {
    toggle_capture(&app);
}

#[tauri::command]
fn redock(app: AppHandle, store: State<Store>) {
    if let Some(win) = app.get_webview_window("main") {
        let s = store.settings.lock().unwrap().clone();
        dock(&win, &s);
    }
}

#[tauri::command]
fn quit(app: AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        undock(&win);
    }
    app.exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    let target = Shortcut::new(Some(Modifiers::CONTROL), Code::Space);
                    if event.state() == ShortcutState::Pressed && *shortcut == target {
                        toggle_capture(app);
                    }
                })
                .build(),
        )
        .manage(CalCacheState::default())
        .setup(|app| {
            let dir = app.path().app_data_dir().expect("app data dir");
            let store = Store::load(dir);
            let settings = store.settings.lock().unwrap().clone();
            app.manage(store);

            if let Some(win) = app.get_webview_window("main") {
                win.set_always_on_top(settings.always_on_top).ok();
                dock(&win, &settings);
                win.show().ok();
            }

            if let Err(e) = app.global_shortcut().register(CAPTURE_SHORTCUT) {
                eprintln!("failed to register {CAPTURE_SHORTCUT}: {e}");
                app.emit("shortcut-error", e.to_string()).ok();
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            use tauri::WindowEvent;
            match event {
                WindowEvent::Focused(false) if window.label() == "capture" => {
                    window.hide().ok();
                }
                WindowEvent::Destroyed if window.label() == "main" => {
                    if let Some(win) = window.app_handle().get_webview_window("main") {
                        undock(&win);
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_tasks,
            add_task,
            toggle_task,
            update_task,
            delete_task,
            clear_done,
            get_settings,
            set_settings,
            get_meetings,
            hide_capture,
            show_capture,
            redock,
            quit
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
