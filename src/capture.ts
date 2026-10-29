import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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
    await invoke("add_item", { text });
    input.value = "";
    flash.textContent = "Saved ✓";
    flash.classList.add("show");
    setTimeout(() => flash.classList.remove("show"), 900);
    setTimeout(() => invoke("hide_capture"), 350);
  });

  input.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      input.value = "";
      invoke("hide_capture");
    }
  });
});
