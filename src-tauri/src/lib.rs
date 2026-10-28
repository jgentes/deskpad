mod appbar;
mod calendar;
mod google;
mod store;

use chrono::{Duration, Utc};
use google::Google;
use serde::Serialize;
use std::sync::Mutex;
use store::{Settings, Store, Task};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_opener::OpenerExt;

const CAPTURE_SHORTCUT: &str = "ctrl+space";

/// Cached ICS text so the UI can refresh often without hammering the feed.
struct CalendarCache {
    text: String,
    fetched_at: i64,
    url: String,
}

#[derive(Default)]
struct CalCacheState(Mutex<Option<CalendarCache>>);

/// Whether the sidebar is currently hidden via the tray.
#[derive(Default)]
struct SidebarHidden(Mutex<bool>);

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

fn apply_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(|e| e.to_string())
}

#[tauri::command]
fn set_settings(app: AppHandle, store: State<Store>, settings: Settings) -> Result<(), String> {
    let layout_changed;
    let autostart_changed;
    {
        let mut s = store.settings.lock().unwrap();
        layout_changed = s.width != settings.width || s.reserve_space != settings.reserve_space;
        autostart_changed = s.autostart != settings.autostart;
        if s.ics_url != settings.ics_url {
            *app.state::<CalCacheState>().0.lock().unwrap() = None;
        }
        *s = settings.clone();
    }
    store.save_settings();
    if let Some(win) = app.get_webview_window("main") {
        win.set_always_on_top(settings.always_on_top).ok();
        if layout_changed && !*app.state::<SidebarHidden>().0.lock().unwrap() {
            dock(&win, &settings);
        }
    }
    if autostart_changed {
        apply_autostart(&app, settings.autostart)?;
    }
    app.emit("settings-changed", ()).ok();
    Ok(())
}

// ---------- Calendar ----------

#[derive(Serialize)]
struct MeetingsResult {
    source: &'static str,
    meetings: Vec<calendar::Meeting>,
}

#[tauri::command]
async fn get_meetings(
    store: State<'_, Store>,
    cache: State<'_, CalCacheState>,
    google: State<'_, Google>,
    force: bool,
) -> Result<MeetingsResult, String> {
    let settings = store.settings.lock().unwrap().clone();
    let now = Utc::now();
    let to = now + Duration::hours(settings.lookahead_hours as i64);

    if google.connected() {
        let meetings = google
            .meetings(&settings.google_client_id, &settings.google_client_secret, now, to)
            .await?;
        return Ok(MeetingsResult { source: "google", meetings });
    }

    let url = settings.ics_url.trim().to_string();
    if url.is_empty() {
        return Ok(MeetingsResult { source: "none", meetings: vec![] });
    }
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
    Ok(MeetingsResult { source: "ics", meetings: calendar::expand(&text, now, to) })
}

#[tauri::command]
fn google_status(google: State<Google>) -> bool {
    google.connected()
}

#[tauri::command]
async fn google_connect(
    app: AppHandle,
    store: State<'_, Store>,
    google: State<'_, Google>,
) -> Result<(), String> {
    let (id, secret) = {
        let s = store.settings.lock().unwrap();
        (s.google_client_id.clone(), s.google_client_secret.clone())
    };
    let opener = app.opener();
    google
        .connect(&id, &secret, |url| {
            opener.open_url(url, None::<&str>).ok();
        })
        .await?;
    app.emit("settings-changed", ()).ok();
    Ok(())
}

#[tauri::command]
fn google_disconnect(app: AppHandle, google: State<Google>) {
    google.disconnect();
    app.emit("settings-changed", ()).ok();
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

fn set_sidebar_visible(app: &AppHandle, visible: bool) {
    let Some(win) = app.get_webview_window("main") else { return };
    *app.state::<SidebarHidden>().0.lock().unwrap() = !visible;
    if visible {
        let s = app.state::<Store>().settings.lock().unwrap().clone();
        win.show().ok();
        dock(&win, &s);
    } else {
        undock(&win);
        win.hide().ok();
    }
}

fn toggle_sidebar(app: &AppHandle) {
    let hidden = *app.state::<SidebarHidden>().0.lock().unwrap();
    set_sidebar_visible(app, hidden);
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
fn hide_sidebar(app: AppHandle) {
    set_sidebar_visible(&app, false);
}

#[tauri::command]
fn redock(app: AppHandle, store: State<Store>) {
    if *app.state::<SidebarHidden>().0.lock().unwrap() {
        return;
    }
    if let Some(win) = app.get_webview_window("main") {
        let s = store.settings.lock().unwrap().clone();
        dock(&win, &s);
    }
}

#[tauri::command]
fn open_settings(app: AppHandle) {
    set_sidebar_visible(&app, true);
    app.emit_to("main", "open-settings", ()).ok();
}

#[tauri::command]
fn quit(app: AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        undock(&win);
    }
    app.exit(0);
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "Show / hide sidebar", true, None::<&str>)?;
    let capture = MenuItem::with_id(app, "capture", "Capture task\tCtrl+Space", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit Deskpad", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&toggle, &capture, &settings, &PredefinedMenuItem::separator(app)?, &quit_item],
    )?;
    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Deskpad")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => toggle_sidebar(app),
            "capture" => toggle_capture(app),
            "settings" => open_settings(app.clone()),
            "quit" => quit(app.clone()),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                toggle_sidebar(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
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
        .manage(SidebarHidden::default())
        .setup(|app| {
            let dir = app.path().app_data_dir().expect("app data dir");
            let store = Store::load(dir.clone());
            let settings = store.settings.lock().unwrap().clone();
            app.manage(store);
            app.manage(Google::load(&dir));

            if let Some(win) = app.get_webview_window("main") {
                win.set_always_on_top(settings.always_on_top).ok();
                dock(&win, &settings);
                win.show().ok();
            }

            build_tray(app.handle())?;

            // Keep the registry entry in sync with the saved preference.
            apply_autostart(app.handle(), settings.autostart).ok();

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
                WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                    // Closing the sidebar hides it to the tray instead of quitting.
                    api.prevent_close();
                    set_sidebar_visible(window.app_handle(), false);
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
            google_status,
            google_connect,
            google_disconnect,
            hide_capture,
            show_capture,
            hide_sidebar,
            redock,
            open_settings,
            quit
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
