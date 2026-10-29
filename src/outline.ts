// Workflowy-style outline: every row is a note by default, becomes a task via
// a marker (`- `, `[]`, `/task`) or Ctrl+Enter, and can nest one level under
// the item above it with Tab. The frontend owns ordering; after every change
// the whole list is sent to the backend with `set_items`.

import { invoke } from "@tauri-apps/api/core";

export interface Item {
  id: number;
  text: string;
  kind: "note" | "task";
  done: boolean;
  parent: number | null;
  created_at: number;
}

const $ = <T extends HTMLElement>(sel: string) => document.querySelector(sel) as T;

let items: Item[] = [];
/** Set once the backend list has been read. Until then nothing is ever saved,
 *  so a transient startup failure can never overwrite the stored notes. */
let loaded = false;
let saveTimer: number | undefined;
let dirty = false;
let pendingFocus: { id: number | "new"; caret: number | "end" } | null = null;

// ---------- model helpers ----------

/** Mirror of the backend's marker parser so typing a marker converts instantly. */
export function parseMarker(raw: string): { text: string; kind: "note" | "task" | null } {
  let t = raw;
  let kind: "note" | "task" | null = null;
  for (const m of ["[ ]", "[]", "- "]) {
    if (t.startsWith(m)) {
      t = t.slice(m.length).replace(/^\s+/, "");
      kind = "task";
      break;
    }
  }
  if (/\/task\b/.test(t)) {
    t = t.replace(/\s*\/task\b/g, "");
    kind = "task";
  }
  if (/\/note\b/.test(t)) {
    t = t.replace(/\s*\/note\b/g, "");
    kind = "note";
  }
  return { text: t, kind };
}

const idx = (id: number) => items.findIndex((i) => i.id === id);
const byId = (id: number) => items.find((i) => i.id === id);

/** Index of the last row in `parentId`'s block (the parent itself if childless). */
function blockEnd(parentId: number): number {
  let end = idx(parentId);
  for (let i = end + 1; i < items.length && items[i].parent === parentId; i++) end = i;
  return end;
}

function newItem(text: string, parent: number | null, kind: "note" | "task" = "note"): Item {
  return { id: Date.now() * 1000 + Math.floor(Math.random() * 1000), text, kind, done: false, parent, created_at: Math.floor(Date.now() / 1000) };
}

function remove(id: number) {
  // Promote children so nothing is orphaned.
  for (const i of items) if (i.parent === id) i.parent = null;
  items = items.filter((i) => i.id !== id);
}

// ---------- persistence ----------

function scheduleSave() {
  dirty = true;
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(flushSave, 250);
}

async function flushSave() {
  window.clearTimeout(saveTimer);
  if (!dirty || !loaded) return;
  dirty = false;
  try {
    await invoke("set_items", { items });
  } catch (err) {
    console.error("set_items failed", err);
    dirty = true;
  }
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export async function loadItems() {
  let fetched: Item[] | null = null;
  for (let attempt = 0; attempt < 5 && fetched === null; attempt++) {
    try {
      fetched = await invoke<Item[]>("list_items");
    } catch (err) {
      console.error(`list_items failed (attempt ${attempt + 1})`, err);
      await sleep(400 * (attempt + 1));
    }
  }
  if (fetched === null) {
    $("#outline").innerHTML = `<div class="error">Couldn't load notes. <a id="retry-items">Retry</a></div>`;
    $("#retry-items").addEventListener("click", () => loadItems());
    return;
  }
  if (dirty) {
    // Keep unsaved local text edits for rows the backend still has.
    for (const f of fetched) {
      const local = byId(f.id);
      if (local) Object.assign(f, { text: local.text, kind: local.kind, done: local.done });
    }
  }
  items = fetched;
  loaded = true;
  render();
}

// ---------- caret helpers ----------

function caretOffset(el: HTMLElement): number {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0) return 0;
  const r = sel.getRangeAt(0).cloneRange();
  r.selectNodeContents(el);
  r.setEnd(sel.getRangeAt(0).endContainer, sel.getRangeAt(0).endOffset);
  return r.toString().length;
}

function setCaret(el: HTMLElement, pos: number | "end") {
  el.focus();
  const sel = window.getSelection();
  if (!sel) return;
  const range = document.createRange();
  const node = el.firstChild;
  if (!node) {
    range.setStart(el, 0);
  } else {
    const len = node.textContent?.length ?? 0;
    range.setStart(node, pos === "end" ? len : Math.min(pos, len));
  }
  range.collapse(true);
  sel.removeAllRanges();
  sel.addRange(range);
}

/** Is the caret on the first/last visual line of a wrapped row? */
function caretOnEdgeLine(el: HTMLElement, edge: "first" | "last"): boolean {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0 || !el.firstChild) return true;
  const rects = sel.getRangeAt(0).getClientRects();
  const r = rects[0] ?? el.getBoundingClientRect();
  const box = el.getBoundingClientRect();
  const line = parseFloat(getComputedStyle(el).lineHeight) || 20;
  return edge === "first" ? r.top - box.top < line * 0.6 : box.bottom - r.bottom < line * 0.6;
}

// ---------- rendering ----------

function render() {
  const el = $("#outline");
  const frag = document.createDocumentFragment();
  for (const it of items) frag.appendChild(renderRow(it));
  frag.appendChild(renderNewRow());
  el.replaceChildren(frag);

  const open = items.filter((i) => i.kind === "task" && !i.done).length;
  const done = items.filter((i) => i.kind === "task" && i.done).length;
  $("#task-count").textContent = open ? String(open) : "";
  const clearBtn = $<HTMLButtonElement>("#btn-clear-done");
  clearBtn.hidden = done === 0;
  clearBtn.textContent = done === 1 ? "clear 1 completed" : `clear ${done} completed`;

  if (pendingFocus) {
    const target = pendingFocus.id === "new" ? el.querySelector<HTMLElement>(".row.new .txt") : el.querySelector<HTMLElement>(`.row[data-id="${pendingFocus.id}"] .txt`);
    if (target) setCaret(target, pendingFocus.caret);
    pendingFocus = null;
  }
}

function rowClass(it: Item) {
  return `row ${it.kind}${it.done ? " done" : ""}${it.parent !== null ? " child" : ""}`;
}

function renderRow(it: Item): HTMLElement {
  const row = document.createElement("div");
  row.className = rowClass(it);
  row.dataset.id = String(it.id);

  const mark = document.createElement("button");
  mark.className = "mark";
  mark.tabIndex = -1;
  mark.title = it.kind === "task" ? (it.done ? "Mark not done" : "Mark done") : "Turn into a task";
  mark.innerHTML = it.kind === "task" ? `<svg viewBox="0 0 24 24"><path d="M5 12l5 5L20 7" /></svg>` : "";
  mark.addEventListener("mousedown", (e) => e.preventDefault());
  mark.addEventListener("click", () => {
    if (it.kind === "task") it.done = !it.done;
    else it.kind = "task";
    row.className = rowClass(it);
    mark.title = it.done ? "Mark not done" : "Mark done";
    mark.innerHTML = `<svg viewBox="0 0 24 24"><path d="M5 12l5 5L20 7" /></svg>`;
    scheduleSave();
    render();
  });

  const txt = document.createElement("div");
  txt.className = "txt";
  txt.contentEditable = "plaintext-only";
  txt.spellcheck = true;
  txt.textContent = it.text;
  txt.addEventListener("input", () => onInput(it, txt));
  txt.addEventListener("keydown", (e) => onKey(e, it, txt));
  txt.addEventListener("blur", () => flushSave());

  const del = document.createElement("button");
  del.className = "del";
  del.tabIndex = -1;
  del.title = "Delete";
  del.innerHTML = `<svg viewBox="0 0 24 24"><path d="M6 6l12 12M18 6L6 18" /></svg>`;
  del.addEventListener("mousedown", (e) => e.preventDefault());
  del.addEventListener("click", () => {
    remove(it.id);
    scheduleSave();
    render();
  });

  row.append(mark, txt, del);
  return row;
}

function renderNewRow(): HTMLElement {
  const row = document.createElement("div");
  row.className = "row new note";
  const mark = document.createElement("span");
  mark.className = "mark";
  const txt = document.createElement("div");
  txt.className = "txt";
  txt.contentEditable = "plaintext-only";
  txt.dataset.placeholder = items.length ? "Add a note…" : "Add a note… start with - or [] for a task";
  txt.addEventListener("input", () => {
    const raw = txt.textContent ?? "";
    if (!raw) return;
    const { text, kind } = parseMarker(raw);
    const it = newItem(text, null, kind ?? "note");
    items.push(it);
    pendingFocus = { id: it.id, caret: "end" };
    scheduleSave();
    render();
  });
  txt.addEventListener("keydown", (e) => {
    if (e.key === "ArrowUp" && items.length) {
      e.preventDefault();
      pendingFocus = { id: items[items.length - 1].id, caret: "end" };
      render();
    } else if (e.key === "Enter") {
      e.preventDefault();
    } else if (e.key === "Escape") {
      txt.blur();
    }
  });
  row.append(mark, txt);
  return row;
}

// ---------- editing ----------

function onInput(it: Item, txt: HTMLElement) {
  const raw = txt.textContent ?? "";
  const { text, kind } = parseMarker(raw);
  if (kind && (kind !== it.kind || text !== raw)) {
    it.kind = kind;
    it.text = text;
    pendingFocus = { id: it.id, caret: "end" };
    scheduleSave();
    render();
    return;
  }
  it.text = raw;
  scheduleSave();
}

function onKey(e: KeyboardEvent, it: Item, txt: HTMLElement) {
  const i = idx(it.id);
  switch (e.key) {
    case "Enter": {
      e.preventDefault();
      if (e.ctrlKey) {
        it.kind = it.kind === "task" ? "note" : "task";
        pendingFocus = { id: it.id, caret: caretOffset(txt) };
        scheduleSave();
        render();
        return;
      }
      // Split at the caret: text after it moves to the new row.
      const at = caretOffset(txt);
      const after = it.text.slice(at);
      it.text = it.text.slice(0, at);
      const hasChildren = it.parent === null && items[i + 1]?.parent === it.id;
      const next = newItem(after, hasChildren ? it.id : it.parent);
      items.splice(i + 1, 0, next);
      pendingFocus = { id: next.id, caret: 0 };
      scheduleSave();
      render();
      return;
    }
    case "Tab": {
      e.preventDefault();
      if (e.shiftKey) {
        if (it.parent === null) return;
        const parent = it.parent;
        remove(it.id);
        it.parent = null;
        items.splice(blockEnd(parent) + 1, 0, it);
      } else {
        if (it.parent !== null) return;
        let p = -1;
        for (let k = i - 1; k >= 0; k--) if (items[k].parent === null) { p = k; break; }
        if (p < 0) return;
        const parentId = items[p].id;
        // Our own children become siblings under the new parent (one level max).
        for (const c of items) if (c.parent === it.id) c.parent = parentId;
        it.parent = parentId;
      }
      pendingFocus = { id: it.id, caret: caretOffset(txt) };
      scheduleSave();
      render();
      return;
    }
    case "Backspace": {
      if (txt.textContent === "" || (caretOffset(txt) === 0 && !it.text)) {
        e.preventDefault();
        const prev = items[i - 1];
        remove(it.id);
        pendingFocus = prev ? { id: prev.id, caret: "end" } : { id: "new", caret: 0 };
        scheduleSave();
        render();
      }
      return;
    }
    case "ArrowUp": {
      if (!caretOnEdgeLine(txt, "first")) return;
      const prev = items[i - 1];
      if (!prev) return;
      e.preventDefault();
      focusRow(prev.id);
      return;
    }
    case "ArrowDown": {
      if (!caretOnEdgeLine(txt, "last")) return;
      e.preventDefault();
      const next = items[i + 1];
      if (next) focusRow(next.id);
      else setCaret($(".row.new .txt"), 0);
      return;
    }
    case "Escape":
      txt.blur();
      return;
  }
}

function focusRow(id: number) {
  const el = document.querySelector<HTMLElement>(`.row[data-id="${id}"] .txt`);
  if (el) setCaret(el, "end");
}

// ---------- public ----------

export function clearCompleted() {
  for (const it of items.filter((i) => i.kind === "task" && i.done)) remove(it.id);
  scheduleSave();
  render();
}

export function focusNewRow() {
  setCaret($(".row.new .txt"), 0);
}
