# Deskpad — notes for AI agents and contributors

Deskpad is a Windows 11 sidebar built with **Tauri 2** (Rust backend, vanilla
TypeScript + Vite frontend). It docks to the right screen edge as a Win32
AppBar, shows today's Google Calendar events (with tomorrow's early ones), keeps a
Workflowy-style outline of notes and tasks, captures notes from anywhere with
Ctrl+Space, and throws a full-screen alert two minutes before a meeting.

## Layout

| Path | What |
| --- | --- |
| `src-tauri/src/lib.rs` | Tauri commands, window setup, tray, global shortcut, alert window |
| `src-tauri/src/appbar.rs` | Win32 `SHAppBarMessage` docking and native hide/show |
| `src-tauri/src/store.rs` | `Item`/`Settings` persistence, task-marker parsing, archive, atomic writes, backups |
| `src-tauri/src/calendar.rs` | The shared `Meeting` type (ICS support was removed; Google only) |
| `src-tauri/src/google.rs` | Google OAuth (PKCE + loopback), Calendar read, event creation |
| `src-tauri/nsis/installer.nsi` | Custom NSIS installer template (see below) |
| `src/main.ts` | Sidebar: clock, meetings, add-event form, alert scheduler, settings |
| `src/outline.ts` | Notes/tasks outliner and archive view |
| `src/capture.ts`, `src/tab.ts`, `src/alert.ts` | Capture popup, collapsed edge tab, alert overlay |
| `index.html`, `capture.html`, `tab.html`, `alert.html` | One page per window (Vite multi-page) |

Windows are declared in `src-tauri/tauri.conf.json`; every window label must
also be listed in `src-tauri/capabilities/default.json`.

## Working on it

- This is a **Windows** project in a PowerShell terminal. Give commands as
  `cd D:\deskpad; npm run tauri dev`, not Unix paths. Rust lives at
  `~/.cargo/bin`; in Git Bash export that onto `PATH` first.
- `npm run tauri dev` serves the frontend on port 1420 and rebuilds the Rust
  side on change. **Only one Deskpad can run at a time**: a second instance
  fights the first for the right edge and the Ctrl+Space shortcut. Quit the
  installed app from its tray icon before running dev, and vice versa.
- Checks: `cargo test` in `src-tauri` (marker parsing, store safety, archive), `npx tsc --noEmit -p .` and `npx vite build` at the root.
- UI changes can be exercised without the native app by opening
  `http://localhost:1420/` in a browser and installing a mock
  `window.__TAURI_INTERNALS__` whose `invoke` implements the commands the page
  calls. `alert.html` exposes `window.__alertShow(meeting)` for the same reason.
- Never send synthetic keyboard input to test the app; it lands in whatever
  window has focus. Synthetic mouse clicks on Deskpad's own window are fine.

## Data safety rules (keep these intact)

User data lives in `%APPDATA%\com.jgentes.deskpad\` (`tasks.json`,
`archive.json`, `settings.json`, `google_tokens.json`, `backups\`). The
installed app and the dev build share it.

1. **Register state before windows exist.** `Store` and `Google` are loaded in
   `run()` before `tauri::Builder` runs, because config windows are created
   before the `setup` hook and the webview can call commands in that gap.
2. **The frontend never saves before a successful load.** `outline.ts` sets
   `loaded` only after `list_items` succeeds; `flushSave` is a no-op before
   that. Startup loads retry instead of falling back to defaults.
3. **Writes are atomic** (`write_atomic`: temp file + rename), the outline gets
   a daily backup (`backup_daily`, newest 30 kept), and a file that fails to
   parse is moved aside as `*.corrupt-<timestamp>` rather than overwritten.
4. **Debug builds never touch autostart.** `apply_autostart` returns early
   under `cfg!(debug_assertions)`; otherwise the Run key ends up pointing at
   `target\debug\deskpad.exe`, which needs the Vite server.

If you add a new persisted file, use `load_json` and `write_atomic`.

## Installer: one click, no prompts

`npm run tauri build` produces a single NSIS installer,
`src-tauri/target/release/bundle/nsis/Deskpad_<version>_x64-setup.exe`
(`bundle.targets` is `["nsis"]`; the MSI target was dropped on purpose).

Double-clicking it must install **without any interaction**: no wizard pages,
no "create desktop shortcut" or "launch app" checkboxes, no "app is running"
dialog. It closes a running Deskpad, copies the new build, and relaunches it.

This is done with `bundle.windows.nsis.template = "nsis/installer.nsi"`, a
copy of Tauri's stock template with two changes, both marked `; Deskpad:`:

- `.onInit` unconditionally sets `$PassiveMode 1` (the stock template only
  does this for the `/P` flag). Passive mode skips every page via
  `SkipIfPassive`, and the `CheckIfAppIsRunning` macro kills the running app
  instead of prompting.
- `.onInstSuccess` always relaunches the app with `nsis_tauri_utils::RunAsUser`
  (the stock template requires `/R`).

`installMode` is `currentUser`, so no UAC prompt either.

The relaunch races the shell: the old Deskpad is killed and the new one starts
at once, while Explorer still holds the dead window's AppBar strip, so the new
window gets docked one sidebar-width inboard. `appbar.rs` handles this: the
AppBar registers a callback message and the window is subclassed, so on
`ABN_POSCHANGED`, `WM_SETTINGCHANGE` or `WM_DISPLAYCHANGE` it re-docks whenever
it is not flush with the monitor's right edge (`needs_redock`), and
`schedule_redock_checks` in `lib.rs` sweeps a few times after launch in case the
shell drops the stale strip silently. Keep both when touching docking.

**When bumping `@tauri-apps/cli`**, re-copy the template from the matching tag
(`crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi` at
`tauri-cli-v<version>`), re-apply the two marked changes, and keep the header
comment current. Then build and run the installer against a running Deskpad
and confirm: it exits within a few seconds, the only window is the progress
page, the old process is gone, and a new one is running.

Bump `version` in `src-tauri/tauri.conf.json` for each release.

## Conventions

- Commands return `Result<_, String>` with user-readable messages; the
  frontend shows them verbatim.
- Frontend code relays `console.error` and uncaught errors to the backend via
  `log_js`, which prints `[js:<label>]` in the dev terminal. Check there first
  when something looks inert.
- Google: read scope `calendar.readonly` plus write scope `calendar.events`.
  Tokens record the granted scope; `can_write()` gates event creation and a
  read-only grant produces a "disconnect and reconnect" message.
- Alert timing lives in `main.ts` (`ALERT_LEAD` = 120 s, `ALERT_GRACE` = 300 s).
- Calendar windows are chosen by the frontend: `get_meetings(from, to)` takes
  unix seconds, and `main.ts` always keeps today + tomorrow loaded (`meetings`,
  which drives alerts) plus the browsed day (`dayMeetings`) when it is another
  one. `tomorrow_until_hour` replaced the old rolling look-ahead.
- Fold state (which parents are collapsed) is a view preference kept in
  `localStorage` (`deskpad.collapsed`), never in `tasks.json`. The archive view
  always opens with every parent folded.
