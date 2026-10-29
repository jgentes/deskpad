import { invoke } from "@tauri-apps/api/core";

window.addEventListener("DOMContentLoaded", () => {
  document.querySelector("#tab-open")!.addEventListener("click", () => invoke("show_sidebar"));
});
