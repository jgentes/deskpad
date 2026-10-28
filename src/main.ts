import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";

interface Task {
  id: number;
  text: string;
  done: boolean;
  created_at: number;
}

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
  always_on_top: boolean;
  reserve_space: boolean;
  width: number;
  lookahead_hours: number;
}

const $ = <T extends HTMLElement>(sel: string) => document.querySelector(sel) as T;

let settings: Settings;
let meetings: Meeting[] = [];

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
  if (m.all_day) return { label: "all day", cls: "" };
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

function linkify(loc: string): string {
  const esc = loc.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
  return esc.replace(/https?:\/\/\S+/g, (u) => `<a href="${u}" data-ext>${u.replace(/^https?:\/\//, "")}</a>`);
}

function renderMeetings() {
  const el = $("#meetings");
  const now = Math.floor(Date.now() / 1000);
  if (!settings?.ics_url) {
    el.innerHTML = `<div class="empty">No calendar connected. <a id="link-settings">Add an .ics feed</a> to see upcoming meetings.</div>`;
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
    const when = m.all_day ? "All day" : `${fmtTime(m.start)} – ${fmtTime(m.end)}`;
    card.innerHTML = `
      <div class="m-time"><span>${when}</span><span class="m-rel">${rel.label}</span></div>
      <div class="m-title" title="${m.title.replace(/"/g, "&quot;")}"></div>
      ${m.location ? `<div class="m-loc">${linkify(m.location)}</div>` : ""}
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
  if (!settings?.ics_url) {
    meetings = [];
    renderMeetings();
    return;
  }
  const btn = $("#btn-refresh");
  btn.classList.add("spin");
  try {
    meetings = await invoke<Meeting[]>("get_meetings", { force });
    renderMeetings();
  } catch (err) {
    $("#meetings").innerHTML = `<div class="error">Couldn't load calendar: ${String(err)}</div>`;
  } finally {
    btn.classList.remove("spin");
  }
}

// ---------- tasks ----------

async function loadTasks() {
  const tasks = await invoke<Task[]>("list_tasks");
  const el = $("#tasks");
  const open = tasks.filter((t) => !t.done).length;
  $("#task-count").textContent = open ? String(open) : "";
  if (tasks.length === 0) {
    el.innerHTML = `<div class="empty">No tasks yet. Press <kbd>Ctrl</kbd>+<kbd>Space</kbd> anywhere to capture one.</div>`;
    return;
  }
  const frag = document.createDocumentFragment();
  const ordered = [...tasks.filter((t) => !t.done), ...tasks.filter((t) => t.done)];
  for (const t of ordered) {
    const row = document.createElement("div");
    row.className = `task ${t.done ? "done" : ""}`;
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = t.done;
    cb.addEventListener("change", () => invoke("toggle_task", { id: t.id }));
    const text = document.createElement("div");
    text.className = "t-text";
    text.textContent = t.text;
    text.title = "Double-click to edit";
    text.addEventListener("dblclick", () => editTask(row, text, t));
    const del = document.createElement("button");
    del.className = "t-del";
    del.textContent = "✕";
    del.title = "Delete";
    del.addEventListener("click", () => invoke("delete_task", { id: t.id }));
    row.append(cb, text, del);
    frag.appendChild(row);
  }
  el.replaceChildren(frag);
}

function editTask(row: HTMLElement, textEl: HTMLElement, t: Task) {
  const input = document.createElement("input");
  input.className = "t-edit";
  input.value = t.text;
  row.replaceChild(input, textEl);
  input.focus();
  input.select();
  const finish = async (save: boolean) => {
    if (save && input.value.trim() && input.value.trim() !== t.text) {
      await invoke("update_task", { id: t.id, text: input.value });
    } else {
      loadTasks();
    }
  };
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") finish(true);
    if (e.key === "Escape") finish(false);
  });
  input.addEventListener("blur", () => finish(true));
}

// ---------- settings ----------

function openSettings() {
  $<HTMLInputElement>("#s-ics").value = settings.ics_url;
  $<HTMLInputElement>("#s-lookahead").value = String(settings.lookahead_hours);
  $<HTMLInputElement>("#s-width").value = String(settings.width);
  $<HTMLInputElement>("#s-reserve").checked = settings.reserve_space;
  $<HTMLInputElement>("#s-top").checked = settings.always_on_top;
  $("#settings").classList.remove("hidden");
  $<HTMLInputElement>("#s-ics").focus();
}

async function saveSettings(e: Event) {
  e.preventDefault();
  const next: Settings = {
    ics_url: $<HTMLInputElement>("#s-ics").value.trim(),
    lookahead_hours: Math.max(1, Number($<HTMLInputElement>("#s-lookahead").value) || 36),
    width: Math.min(600, Math.max(240, Number($<HTMLInputElement>("#s-width").value) || 340)),
    reserve_space: $<HTMLInputElement>("#s-reserve").checked,
    always_on_top: $<HTMLInputElement>("#s-top").checked,
  };
  await invoke("set_settings", { settings: next });
  settings = next;
  $("#settings").classList.add("hidden");
  loadMeetings(true);
}

// ---------- boot ----------

window.addEventListener("DOMContentLoaded", async () => {
  tickClock();
  setInterval(tickClock, 1000);

  settings = await invoke<Settings>("get_settings");
  await Promise.all([loadTasks(), loadMeetings()]);

  // Re-render relative times every 30s, refetch the feed every 5 min.
  setInterval(renderMeetings, 30_000);
  setInterval(() => loadMeetings(), 5 * 60_000);

  await listen("tasks-changed", loadTasks);

  $("#task-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const input = $<HTMLInputElement>("#task-input");
    if (!input.value.trim()) return;
    await invoke("add_task", { text: input.value });
    input.value = "";
  });

  $("#btn-capture").addEventListener("click", () => invoke("show_capture"));
  $("#btn-settings").addEventListener("click", openSettings);
  $("#btn-refresh").addEventListener("click", () => loadMeetings(true));
  $("#btn-clear-done").addEventListener("click", () => invoke("clear_done"));
  $("#settings-form").addEventListener("submit", saveSettings);
  $("#btn-settings-cancel").addEventListener("click", () => $("#settings").classList.add("hidden"));
  $("#btn-quit").addEventListener("click", () => invoke("quit"));
  $("#settings").addEventListener("click", (e) => {
    if (e.target === $("#settings")) $("#settings").classList.add("hidden");
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") $("#settings").classList.add("hidden");
  });

  // Re-dock when the display layout changes (resolution / monitor swap).
  window.addEventListener("resize", () => invoke("redock"));
});
