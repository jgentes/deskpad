import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { clearCompleted, loadItems } from "./outline";

interface Meeting {
  id: string;
  title: string;
  start: number;
  end: number;
  all_day: boolean;
  location: string | null;
  url: string | null;
}

interface Settings {
  ics_url: string;
  google_client_id: string;
  google_client_secret: string;
  always_on_top: boolean;
  reserve_space: boolean;
  autostart: boolean;
  width: number;
  lookahead_hours: number;
}

interface MeetingsResult {
  source: "google" | "ics" | "none";
  meetings: Meeting[];
}

const $ = <T extends HTMLElement>(sel: string) => document.querySelector(sel) as T;

let settings: Settings;
let meetings: Meeting[] = [];
let googleConnected = false;

// ---------- clock ----------

function tickClock() {
  const now = new Date();
  $("#clock-time").textContent = now.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  $("#clock-date").textContent = now.toLocaleDateString([], {
    weekday: "long",
    month: "long",
    day: "numeric",
  });
}

// ---------- meetings ----------

function fmtTime(ts: number) {
  return new Date(ts * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

function relative(m: Meeting, now: number): { label: string; cls: string } {
  if (m.all_day) return { label: "", cls: "" };
  if (now >= m.start && now < m.end) {
    const left = Math.ceil((m.end - now) / 60);
    return { label: `now · ${left}m left`, cls: "now" };
  }
  if (now >= m.end) return { label: "ended", cls: "past" };
  const mins = Math.round((m.start - now) / 60);
  if (mins < 60) return { label: `in ${mins}m`, cls: "soon" };
  const hrs = Math.floor(mins / 60);
  if (hrs < 12) return { label: `in ${hrs}h ${mins % 60}m`, cls: "" };
  return { label: "", cls: "" };
}

function dayLabel(ts: number) {
  const d = new Date(ts * 1000);
  const today = new Date();
  const tomorrow = new Date();
  tomorrow.setDate(today.getDate() + 1);
  if (d.toDateString() === today.toDateString()) return "Today";
  if (d.toDateString() === tomorrow.toDateString()) return "Tomorrow";
  return d.toLocaleDateString([], { weekday: "long", month: "short", day: "numeric" });
}

function joinLabel(url: string): string {
  if (/meet\.google\.com/.test(url)) return "Join Meet";
  if (/zoom\.us/.test(url)) return "Join Zoom";
  if (/teams\.microsoft\.com|teams\.live\.com/.test(url)) return "Join Teams";
  if (/calendar\.google\.com/.test(url)) return "Open in Calendar";
  return "Join";
}

function renderMeetings() {
  const el = $("#meetings");
  const now = Math.floor(Date.now() / 1000);
  if (!settings?.ics_url && !googleConnected) {
    el.innerHTML = `<div class="empty">No calendar connected. <a id="link-settings">Connect Google Calendar or an .ics feed</a> to see upcoming meetings.</div>`;
    el.querySelector("#link-settings")?.addEventListener("click", openSettings);
    return;
  }
  const upcoming = meetings.filter((m) => m.end > now - 15 * 60);
  if (upcoming.length === 0) {
    el.innerHTML = `<div class="empty">Nothing scheduled in the next ${settings.lookahead_hours}h.</div>`;
    return;
  }
  const frag = document.createDocumentFragment();
  let lastDay = "";
  for (const m of upcoming) {
    const day = dayLabel(m.start);
    if (day !== lastDay) {
      const sep = document.createElement("div");
      sep.className = "day-sep";
      sep.textContent = day;
      frag.appendChild(sep);
      lastDay = day;
    }
    const rel = relative(m, now);
    const card = document.createElement("div");
    card.className = `meeting ${rel.cls}`;
    const when = m.all_day ? "all day" : fmtTime(m.start);
    const range = m.all_day ? "All day" : `${fmtTime(m.start)} – ${fmtTime(m.end)}`;
    const loc = m.location && !m.location.startsWith("http") ? `\n${m.location}` : "";
    card.title = `${range}${loc}`;
    card.innerHTML = `
      <span class="m-when">${when}</span>
      <span class="m-title"></span>
      <span class="m-rel">${rel.label}</span>
      ${m.url ? `<a class="m-join" href="${m.url.replace(/"/g, "&quot;")}" data-ext title="${joinLabel(m.url)}"><svg viewBox="0 0 24 24"><path d="M15 3h6v6M21 3l-9 9M10 5H5v14h14v-5" /></svg></a>` : ""}
    `;
    card.querySelector(".m-title")!.textContent = m.title;
    frag.appendChild(card);
  }
  el.replaceChildren(frag);
  el.querySelectorAll<HTMLAnchorElement>("a[data-ext]").forEach((a) =>
    a.addEventListener("click", (e) => {
      e.preventDefault();
      openUrl(a.href);
    }),
  );
}

async function loadMeetings(force = false) {
  if (!settings?.ics_url && !googleConnected) {
    meetings = [];
    renderMeetings();
    return;
  }
  const btn = $("#btn-refresh");
  btn.classList.add("spin");
  try {
    const result = await invoke<MeetingsResult>("get_meetings", { force });
    meetings = result.meetings;
    $("#meetings-panel h2").textContent = result.source === "google" ? "Up next · Google" : "Up next";
    renderMeetings();
  } catch (err) {
    $("#meetings").innerHTML = `<div class="error">Couldn't load calendar: ${String(err)}</div>`;
  } finally {
    btn.classList.remove("spin");
  }
}

// ---------- settings ----------

function renderGoogleState() {
  $("#google-connected").classList.toggle("hidden", !googleConnected);
  $("#google-disconnected").classList.toggle("hidden", googleConnected);
}

function openSettings() {
  $<HTMLInputElement>("#s-ics").value = settings.ics_url;
  $<HTMLInputElement>("#s-gid").value = settings.google_client_id;
  $<HTMLInputElement>("#s-gsecret").value = settings.google_client_secret;
  $<HTMLInputElement>("#s-lookahead").value = String(settings.lookahead_hours);
  $<HTMLInputElement>("#s-width").value = String(settings.width);
  $<HTMLInputElement>("#s-reserve").checked = settings.reserve_space;
  $<HTMLInputElement>("#s-top").checked = settings.always_on_top;
  $<HTMLInputElement>("#s-autostart").checked = settings.autostart;
  $("#google-error").textContent = "";
  renderGoogleState();
  $("#settings").classList.remove("hidden");
}

function readSettingsForm(): Settings {
  return {
    ics_url: $<HTMLInputElement>("#s-ics").value.trim(),
    google_client_id: $<HTMLInputElement>("#s-gid").value.trim(),
    google_client_secret: $<HTMLInputElement>("#s-gsecret").value.trim(),
    lookahead_hours: Math.max(1, Number($<HTMLInputElement>("#s-lookahead").value) || 36),
    width: Math.min(600, Math.max(240, Number($<HTMLInputElement>("#s-width").value) || 340)),
    reserve_space: $<HTMLInputElement>("#s-reserve").checked,
    always_on_top: $<HTMLInputElement>("#s-top").checked,
    autostart: $<HTMLInputElement>("#s-autostart").checked,
  };
}

async function saveSettings(e: Event) {
  e.preventDefault();
  const next = readSettingsForm();
  try {
    await invoke("set_settings", { settings: next });
  } catch (err) {
    $("#google-error").textContent = String(err);
    return;
  }
  settings = next;
  $("#settings").classList.add("hidden");
  loadMeetings(true);
}

async function connectGoogle() {
  const btn = $<HTMLButtonElement>("#btn-google-connect");
  const errEl = $("#google-error");
  errEl.textContent = "";
  // Persist the client credentials first so the backend can use them.
  const next = readSettingsForm();
  if (!next.google_client_id) {
    errEl.textContent = "Enter the OAuth client ID first.";
    return;
  }
  await invoke("set_settings", { settings: next });
  settings = next;
  btn.disabled = true;
  btn.textContent = "Waiting for sign-in in your browser…";
  try {
    await invoke("google_connect");
    googleConnected = true;
    renderGoogleState();
    loadMeetings(true);
  } catch (err) {
    errEl.textContent = String(err);
  } finally {
    btn.disabled = false;
    btn.textContent = "Connect Google Calendar";
  }
}

async function disconnectGoogle() {
  await invoke("google_disconnect");
  googleConnected = false;
  renderGoogleState();
  loadMeetings(true);
}

// ---------- boot ----------

function wireUi() {
  $("#btn-capture").addEventListener("click", () => invoke("show_capture"));
  $("#btn-settings").addEventListener("click", openSettings);
  $("#btn-hide").addEventListener("click", () => invoke("hide_sidebar"));
  $("#btn-google-connect").addEventListener("click", connectGoogle);
  $("#btn-google-disconnect").addEventListener("click", disconnectGoogle);
  $("#btn-refresh").addEventListener("click", () => loadMeetings(true));
  $("#btn-clear-done").addEventListener("click", clearCompleted);
  $("#settings-form").addEventListener("submit", saveSettings);
  $("#btn-settings-cancel").addEventListener("click", () => $("#settings").classList.add("hidden"));
  $("#btn-quit").addEventListener("click", () => invoke("quit"));
  $("#settings").addEventListener("click", (e) => {
    if (e.target === $("#settings")) $("#settings").classList.add("hidden");
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") $("#settings").classList.add("hidden");
  });
}

window.addEventListener("error", (e) => invoke("log_js", { msg: `error: ${e.message} @${e.filename}:${e.lineno}` }).catch(() => {}));
window.addEventListener("unhandledrejection", (e) => invoke("log_js", { msg: `unhandled: ${String(e.reason)}` }).catch(() => {}));

window.addEventListener("DOMContentLoaded", async () => {
  // Wire every control first, synchronously, so a slow or failed backend call
  // can never leave the UI inert (or let the task form do a native submit).
  wireUi();
  tickClock();
  setInterval(tickClock, 1000);

  try {
    [settings, googleConnected] = await Promise.all([
      invoke<Settings>("get_settings"),
      invoke<boolean>("google_status"),
    ]);
  } catch (err) {
    console.error("failed to load settings", err);
    settings = {
      ics_url: "",
      google_client_id: "",
      google_client_secret: "",
      always_on_top: false,
      reserve_space: true,
      autostart: false,
      width: 340,
      lookahead_hours: 36,
    };
  }

  loadItems().catch((err) => console.error("loadItems failed", err));
  loadMeetings().catch((err) => console.error("loadMeetings failed", err));

  // Re-render relative times every 30s, refetch the feed every 5 min.
  setInterval(renderMeetings, 30_000);
  setInterval(() => loadMeetings(), 5 * 60_000);

  listen("items-changed", () => loadItems()).catch((err) => console.error("listen failed", err));
  listen("open-settings", () => openSettings()).catch((err) => console.error("listen failed", err));
});
