import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { parseMarker, parseSchedule } from "./schedule";

interface AddedItem {
  id: number;
}

window.addEventListener("DOMContentLoaded", async () => {
  const form = document.querySelector("#capture-form") as HTMLFormElement;
  const input = document.querySelector("#capture-input") as HTMLInputElement;
  const flash = document.querySelector("#capture-flash") as HTMLElement;

  const focus = () => {
    input.focus();
    input.select();
  };

  focus();
  listen("capture-shown", focus).catch((err) => console.error("listen failed", err));

  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    const text = input.value.trim();
    if (!text) {
      invoke("hide_capture");
      return;
    }
    // "meet tom friday at 10am" becomes a task on the calendar; a /note is
    // never scheduled.
    const marker = parseMarker(text);
    const sched = marker.kind === "note" ? null : parseSchedule(marker.text);
    const item = await invoke<AddedItem | null>("add_item", { text: sched ? sched.title : text, kind: sched ? "task" : null });
    input.value = "";
    let msg = "Saved ✓";
    let hold = 350;
    if (sched && item) {
      try {
        await invoke("schedule_item", { id: item.id, start: sched.start, end: sched.end, allDay: sched.allDay });
        msg = "Added to calendar ✓";
      } catch (err) {
        console.error("schedule_item failed", err);
        msg = `Saved, but not on the calendar: ${String(err)}`;
        hold = 4000;
      }
    }
    flash.textContent = msg;
    flash.classList.add("show");
    setTimeout(() => flash.classList.remove("show"), hold + 550);
    setTimeout(() => invoke("hide_capture"), hold);
  });

  input.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      input.value = "";
      invoke("hide_capture");
    }
  });
});
