# Deskpad

A Windows 11 sidebar that docks to the right edge of your display, shows your
upcoming meetings, and keeps a Workflowy-style outline of notes and tasks. Capture one from anywhere with **Ctrl+Space**.

Built with [Tauri 2](https://tauri.app) (Rust backend, WebView2 frontend).

## Features

- **Docked sidebar** pinned to the right edge of the primary monitor. By default it
  registers as a Windows AppBar, so maximized windows stop at its edge instead of
  going underneath it (toggle in Settings).
- **Upcoming meetings** from Google Calendar (OAuth, all your checked calendars,
  with Meet/Zoom/Teams join links) or from any iCalendar (`.ics`) feed URL,
  including recurring events, exceptions, and time zones.
- **Tray icon**: left-click toggles the sidebar, right-click for capture,
  settings, and quit. Hiding the sidebar releases its reserved screen space.
- **Start on sign-in** toggle in Settings (adds a Run registry entry).
- **Notes & tasks outline** persisted locally (`%APPDATA%\com.jgentes.deskpad\tasks.json`).
  Every row is a note by default. Start it with `- ` or `[]`, or put `/task` anywhere
  in the text, to make it a task (`/note` converts back, as does **Ctrl+Enter**).
  **Tab** nests a row under the one above (one level), **Shift+Tab** un-nests,
  **Enter** splits or creates a row, **Backspace** on an empty row deletes it.
  Click a bullet to turn a note into a task; click a checkbox to complete it.
- **Global quick capture**: press **Ctrl+Space** in any app for a floating input.
  Type, hit Enter, and it's on your list. Esc or clicking away closes it.

## Run

```bash
npm install
npm run tauri dev
```

## Build an installer

```bash
npm run tauri build
```

Output lands in `src-tauri/target/release/bundle/` (MSI and NSIS installers).

## Connecting your calendar

### Google Calendar (recommended)

Deskpad uses the OAuth desktop-app flow with PKCE and a loopback redirect, so it
needs its own OAuth client. One-time setup in [Google Cloud Console](https://console.cloud.google.com/):

1. Create a project (or pick one) and enable the **Google Calendar API**
   (APIs & Services → Library).
2. Configure the OAuth consent screen (External, add yourself as a test user
   while the app is in testing).
3. APIs & Services → Credentials → **Create credentials → OAuth client ID**,
   application type **Desktop app**. Copy the client ID and client secret.
4. In Deskpad: Settings → paste both → **Connect Google Calendar**. Your browser
   opens for consent; Deskpad listens on `127.0.0.1` for the redirect.

Tokens live in `%APPDATA%\com.jgentes.deskpad\google_tokens.json` and refresh
automatically. The scope is read-only (`calendar.readonly`). Declined invites
and cancelled events are hidden.

### ICS feed (fallback)

Open Settings (gear icon) and paste an `.ics` URL:

- **Google Calendar**: Settings → pick the calendar → *Integrate calendar* →
  *Secret address in iCal format*.
- **Outlook / Microsoft 365**: Settings → Calendar → *Shared calendars* →
  *Publish a calendar* → copy the ICS link.

The feed is re-fetched every 5 minutes; the refresh button forces a fetch.

## Installing (release build)

```powershell
cd D:\deskpad; npm run tauri build
```

Produces two installers under `src-tauri	argeteleaseundle\`:

- `nsis\Deskpad_<version>_x64-setup.exe` — per-user installer, no admin prompt (recommended)
- `msi\Deskpad_<version>_x64_en-US.msi` — Windows Installer package

Bump `version` in `src-tauri/tauri.conf.json` before building a new release; the
installer upgrades in place. The installed app and the dev build share the same
data folder, so notes and settings carry over.

## Your data

Everything lives in `%APPDATA%\com.jgentes.deskpad\`:

| File | Contents |
|---|---|
| `tasks.json` | the notes/tasks outline |
| `settings.json` | settings, including the Google OAuth client |
| `google_tokens.json` | Google refresh token (delete to sign out) |
| `backups\tasks-YYYY-MM-DD.json` | first copy of the outline seen each day, newest 30 kept |

Writes are atomic (temp file + rename). A file that fails to parse is moved
aside as `*.corrupt-<timestamp>` rather than overwritten, and the UI never
saves the outline until it has successfully loaded it from disk.

## Layout

| Path | What |
| --- | --- |
| `src-tauri/src/lib.rs` | Tauri commands, window setup, global shortcut |
| `src-tauri/src/appbar.rs` | Win32 `SHAppBarMessage` docking |
| `src-tauri/src/calendar.rs` | ICS fetch, parse, and recurrence expansion |
| `src-tauri/src/google.rs` | Google OAuth (PKCE + loopback) and Calendar API |
| `src-tauri/src/store.rs` | Outline items + settings persistence, task-marker parsing |
| `src/main.ts`, `index.html` | Sidebar UI |
| `src/capture.ts`, `capture.html` | Ctrl+Space capture popup |
