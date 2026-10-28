# Deskpad

A Windows 11 sidebar that docks to the right edge of your display, shows your
upcoming meetings, and lets you capture tasks from anywhere with **Ctrl+Space**.

Built with [Tauri 2](https://tauri.app) (Rust backend, WebView2 frontend).

## Features

- **Docked sidebar** pinned to the right edge of the primary monitor. By default it
  registers as a Windows AppBar, so maximized windows stop at its edge instead of
  going underneath it (toggle in Settings).
- **Upcoming meetings** from any iCalendar (`.ics`) feed URL, including recurring
  events, exceptions, and time zones. Google Calendar and Outlook both publish one.
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
| `src-tauri/src/store.rs` | Tasks + settings persistence |
| `src/main.ts`, `index.html` | Sidebar UI |
| `src/capture.ts`, `capture.html` | Ctrl+Space capture popup |
