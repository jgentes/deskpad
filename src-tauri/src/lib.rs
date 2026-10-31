mod appbar;
mod calendar;
mod google;
mod store;

use chrono::{Duration, Utc};
use google::Google;
use serde::Serialize;
use std::sync::Mutex;
use store::{ArchivedItem, Item, Settings, Store};
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

// ---------- Items (notes & tasks) ----------

#[tauri::command]
fn list_items(store: State<Store>) -> Vec<Item> {
    store.items.lock().unwrap().clone()
}

/// Replace the whole outline. The frontend owns ordering and nesting, so it
/// sends the full list after every edit.
#[tauri::command]
fn set_items(store: State<Store>, items: Vec<Item>) {
    *store.items.lock().unwrap() = items;
    store.save_items();
}

#[tauri::command]
fn list_archive(store: State<Store>) -> Vec<ArchivedItem> {
    store.archive.lock().unwrap().clone()
}

/// Move items (and their children) to the archive.
#[tauri::command]
fn archive_items(app: AppHandle, store: State<Store>, ids: Vec<u64>) {
    store.archive_items(&ids);
    store.save_items();
    store.save_archive();
    app.emit("items-changed", ()).ok();
}

#[tauri::command]
fn restore_items(app: AppHandle, store: State<Store>, ids: Vec<u64>) {
    store.restore_items(&ids);
    store.save_items();
    store.save_archive();
    app.emit("items-changed", ()).ok();
}

/// Permanently delete archived rows; an empty list clears the whole archive.
#[tauri::command]
fn delete_archived(store: State<Store>, ids: Vec<u64>) {
    store.archive.lock().unwrap().retain(|a| !ids.is_empty() && !ids.contains(&a.item.id));
    store.save_archive();
}

/// Add a top-level item from raw text (used by the capture popup). Task markers
/// in the text are honoured. Inserted at the top so a capture is visible at once.
#[tauri::command]
fn add_item(app: AppHandle, store: State<Store>, text: String) -> Option<Item> {
    let item = Item::from_text(&text, None)?;
    store.items.lock().unwrap().insert(0, item.clone());
    store.save_items();
    app.emit("items-changed", ()).ok();
    Some(item)
}

// ---------- Settings ----------

#[tauri::command]
fn get_settings(store: State<Store>) -> Settings {
    store.settings.lock().unwrap().clone()
}

fn apply_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    // A dev build must never register itself: the Run entry would point at
    // target\debug\deskpad.exe, which needs the Vite server and shows a console.
    // The installed release build re-applies the entry on every launch.
    if cfg!(debug_assertions) {
        eprintln!("[autostart] skipped in debug build (would be {enabled})");
        return Ok(());
    }
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

/// Whether events can be created (Google connected with the write scope).
#[tauri::command]
fn google_can_write(google: State<Google>) -> bool {
    google.connected() && google.can_write()
}

/// Create an event on the primary Google calendar. `start` is unix seconds.
#[tauri::command]
async fn create_event(
    store: State<'_, Store>,
    google: State<'_, Google>,
    title: String,
    start: i64,
    duration_min: i64,
) -> Result<calendar::Meeting, String> {
    if !google.connected() {
        return Err("Connect Google Calendar in Settings to add events.".into());
    }
    let (id, secret) = {
        let s = store.settings.lock().unwrap();
        (s.google_client_id.clone(), s.google_client_secret.clone())
    };
    google
        .create_event(&id, &secret, &title, start, start + duration_min.max(5) * 60)
        .await
}

// ---------- Meeting alert (full-screen overlay) ----------

#[derive(Clone, Serialize, serde::Deserialize)]
struct AlertAction {
    id: String,
    action: String,
    #[serde(default)]
    minutes: u32,
}

/// Cover the monitor the sidebar is on with the alert window and hand it the
/// meeting to display.
#[tauri::command]
fn show_alert(app: AppHandle, meeting: calendar::Meeting) {
    let Some(alert) = app.get_webview_window("alert") else { return };
    let monitor = app
        .get_webview_window("main")
        .and_then(|m| m.current_monitor().ok().flatten())
        .or_else(|| alert.primary_monitor().ok().flatten());
    if let Some(mon) = monitor {
        alert.set_position(*mon.position()).ok();
        alert.set_size(*mon.size()).ok();
    }
    alert.show().ok();
    alert.set_always_on_top(true).ok();
    alert.set_focus().ok();
    app.emit_to("alert", "alert-show", meeting).ok();
}

#[tauri::command]
fn hide_alert(app: AppHandle) {
    if let Some(alert) = app.get_webview_window("alert") {
        alert.hide().ok();
    }
}

/// Called by the alert window; hides it and tells the sidebar what happened.
#[tauri::command]
fn alert_action(app: AppHandle, action: AlertAction) {
    hide_alert(app.clone());
    app.emit_to("main", "alert-action", action).ok();
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

/// After launch, check a few times whether the shell moved us off the right edge
/// (a stale AppBar strip from a process the installer just killed) and re-dock.
/// `appbar::imp::subclass_proc` handles later notifications; this covers the case
/// where the shell silently drops the stale strip without notifying anyone.
fn schedule_redock_checks(app: AppHandle) {
    #[cfg(windows)]
    {
        std::thread::spawn(move || {
            for delay_ms in [1000u64, 2500, 5000, 10000] {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                let app2 = app.clone();
                app.run_on_main_thread(move || {
                    if *app2.state::<SidebarHidden>().0.lock().unwrap() {
                        return;
                    }
                    if let Some(win) = app2.get_webview_window("main") {
                        if let Ok(hwnd) = win.hwnd() {
                            let hwnd = windows::Win32::Foundation::HWND(hwnd.0);
                            if unsafe { appbar::imp::redock_if_displaced(hwnd) } {
                                eprintln!("[dock] re-docked after displacement");
                                place_tab(&app2);
                            }
                        }
                    }
                })
                .ok();
            }
        });
    }
    #[cfg(not(windows))]
    {
        let _ = app;
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

/// Hide the docked sidebar without going through tao (see `appbar::imp::hide`).
fn hide_native(win: &WebviewWindow) {
    #[cfg(windows)]
    {
        if let Ok(hwnd) = win.hwnd() {
            unsafe { appbar::imp::hide(windows::Win32::Foundation::HWND(hwnd.0)) }
            return;
        }
    }
    win.hide().ok();
}

/// Logical size of the edge tab shown while the sidebar is collapsed.
const TAB_W: f64 = 18.0;
const TAB_H: f64 = 72.0;

/// Pin the collapse tab to the right edge of the sidebar's monitor, vertically centered.
fn place_tab(app: &AppHandle) {
    let (Some(tab), Some(main)) = (app.get_webview_window("tab"), app.get_webview_window("main"))
    else {
        return;
    };
    #[cfg(windows)]
    {
        if let Ok(hwnd) = main.hwnd() {
            let (full, _) = unsafe { appbar::imp::monitor_rects(windows::Win32::Foundation::HWND(hwnd.0)) };
            let scale = tab.scale_factor().unwrap_or(1.0);
            let w = (TAB_W * scale).round() as i32;
            let h = (TAB_H * scale).round() as i32;
            tab.set_size(tauri::PhysicalSize::new(w as u32, h as u32)).ok();
            tab.set_position(tauri::PhysicalPosition::new(
                full.right - w,
                full.top + (full.bottom - full.top - h) / 2,
            ))
            .ok();
        }
    }
    #[cfg(not(windows))]
    {
        let _ = main;
    }
}

fn set_sidebar_visible(app: &AppHandle, visible: bool) {
    let Some(win) = app.get_webview_window("main") else { return };
    *app.state::<SidebarHidden>().0.lock().unwrap() = !visible;
    let tab = app.get_webview_window("tab");
    if visible {
        if let Some(t) = &tab {
            t.hide().ok();
        }
        let s = app.state::<Store>().settings.lock().unwrap().clone();
        // dock() shows the window itself (SWP_SHOWWINDOW); do not call win.show() here.
        dock(&win, &s);
    } else {
        undock(&win);
        hide_native(&win);
        place_tab(app);
        if let Some(t) = &tab {
            t.show().ok();
            t.set_always_on_top(true).ok();
        }
    }
}

#[tauri::command]
fn show_sidebar(app: AppHandle) {
    set_sidebar_visible(&app, true);
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

/// Relay webview errors to stderr so they show up in `tauri dev` output.
#[tauri::command]
fn log_js(window: tauri::Window, msg: String) {
    eprintln!("[js:{}] {msg}", window.label());
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

/// Same location Tauri's `app_data_dir()` resolves to on Windows
/// (`%APPDATA%\<identifier>`), computed without an AppHandle so the store can
/// be managed before any window exists.
fn app_data_dir(identifier: &str) -> std::path::PathBuf {
    std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(identifier)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Load persistent state *before* the Builder runs: windows declared in
    // tauri.conf.json are created before the setup hook, and the webview can
    // invoke commands in that gap. State registered inside setup is then not
    // yet managed and every call fails, which once made the UI look wiped.
    let context = tauri::generate_context!();
    let dir = app_data_dir(&context.config().identifier);
    let store = Store::load(dir.clone());
    let google = Google::load(&dir);

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
        .manage(store)
        .manage(google)
        .setup(|app| {
            if let Ok(resolved) = app.path().app_data_dir() {
                let used = &app.state::<Store>().dir;
                if &resolved != used {
                    eprintln!("[store] warning: using {} but Tauri resolves {}", used.display(), resolved.display());
                }
            }
            let settings = app.state::<Store>().settings.lock().unwrap().clone();

            if let Some(win) = app.get_webview_window("main") {
                win.set_always_on_top(settings.always_on_top).ok();
                dock(&win, &settings);
                win.show().ok();
                schedule_redock_checks(app.handle().clone());
            }

            build_tray(app.handle())?;

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
            list_items,
            set_items,
            add_item,
            list_archive,
            archive_items,
            restore_items,
            delete_archived,
            get_settings,
            set_settings,
            get_meetings,
            google_status,
            google_can_write,
            create_event,
            show_alert,
            hide_alert,
            alert_action,
            google_connect,
            google_disconnect,
            hide_capture,
            show_capture,
            hide_sidebar,
            show_sidebar,
            redock,
            open_settings,
            log_js,
            quit
        ])
        .run(context)
        .expect("error while running tauri application");
}
