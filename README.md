# Deskpad

A Windows 11 sidebar that docks to the right edge of your display, shows your
upcoming meetings, and lets you capture tasks from anywhere with **Ctrl+Space**.

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
- **Task list** persisted locally (`%APPDATA%\com.jgentes.deskpad\tasks.json`).
  Check off, edit (double-click), delete, and clear completed tasks.
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

## Layout

| Path | What |
| --- | --- |
| `src-tauri/src/lib.rs` | Tauri commands, window setup, global shortcut |
| `src-tauri/src/appbar.rs` | Win32 `SHAppBarMessage` docking |
| `src-tauri/src/calendar.rs` | ICS fetch, parse, and recurrence expansion |
| `src-tauri/src/google.rs` | Google OAuth (PKCE + loopback) and Calendar API |
| `src-tauri/src/store.rs` | Tasks + settings persistence |
| `src/main.ts`, `index.html` | Sidebar UI |
| `src/capture.ts`, `capture.html` | Ctrl+Space capture popup |
