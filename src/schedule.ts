// Natural-language scheduling for outline rows and the capture popup.
//
// `parseSchedule("meet with tom next friday at 10am")` finds the date/time
// phrase, removes it from the text and returns the cleaned title plus the
// event window (unix seconds, local time). A time without a date means today
// (or tomorrow if that time has already passed); a date without a time is an
// all-day event. Nothing date-like returns null, so plain notes are untouched.
// The Rust side never parses language: it only receives the result.

export interface Parsed {
  title: string;
  start: number;
  end: number;
  allDay: boolean;
}

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

/** Default length of a timed event when the text doesn't say ("for 1 hour"). */
export const DEFAULT_DURATION_MIN = 30;

const DAY_RE = "sun(?:day)?|mon(?:day)?|tue(?:s|sday)?|wed(?:nesday)?|thu(?:r|rs|rsday)?|fri(?:day)?|sat(?:urday)?";
const MONTH_RE =
  "jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|jun(?:e)?|jul(?:y)?|aug(?:ust)?|sep(?:t|tember)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?";
const DAY_PREFIX = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const MONTH_PREFIX = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const WORD_TIMES: Record<string, number> = { morning: 9, noon: 12, midday: 12, afternoon: 14, evening: 18, tonight: 19 };

interface Span {
  start: number;
  end: number;
}

type DateSpec =
  | { kind: "offset"; days: number }
  | { kind: "weekday"; day: number; next: boolean }
  | { kind: "ymd"; year: number | null; month: number; day: number };

/** Local midnight of today plus `days`, as a Date. */
function dayAt(now: Date, days: number): Date {
  const d = new Date(now);
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() + days);
  return d;
}

/** "at 3" / "3:30" with no am/pm: 1–6 is afternoon, 7–11 is morning, 13+ is 24h. */
function guessHour(h: number): number {
  return h >= 1 && h <= 6 ? h + 12 : h;
}

function stripSpans(text: string, spans: Span[]): string {
  let out = text;
  for (const s of [...spans].sort((a, b) => b.start - a.start)) out = out.slice(0, s.start) + " " + out.slice(s.end);
  return out
    .replace(/\s+/g, " ")
    .replace(/\s+([,.;:!?])/g, "$1")
    .replace(/^[\s,;:–-]+|[\s,;:–-]+$/g, "")
    .replace(/\b(?:at|on|for|from|in|the)\s*$/i, "")
    .replace(/^(?:at|on|for)\b\s*/i, "")
    .replace(/\s+/g, " ")
    .trim();
}

export function parseSchedule(raw: string, now: Date = new Date()): Parsed | null {
  const text = raw;
  const lower = text.toLowerCase();
  const spans: Span[] = [];
  let time: { h: number; m: number } | null = null;
  let date: DateSpec | null = null;
  let relative: number | null = null; // minutes from now ("in 2 hours")
  let duration: number | null = null;

  const take = (m: RegExpExecArray) => spans.push({ start: m.index, end: m.index + m[0].length });

  // Recurring things aren't calendar events we can place.
  if (/\bevery\s+(?:day|week|month|year|\d|(?:other\s+)?(?:mon|tue|wed|thu|fri|sat|sun))/i.test(lower)) return null;

  // ---- duration: "for 30 min", "for 1.5 hours", "for 2h"
  {
    const m = /\bfor\s+(\d+(?:\.\d+)?)\s*(h|hr|hrs|hour|hours|m|min|mins|minute|minutes)\b/i.exec(lower);
    if (m) {
      const n = parseFloat(m[1]);
      duration = Math.round(/^h/.test(m[2]) ? n * 60 : n);
      take(m);
    }
  }

  // ---- time of day
  {
    // "10am", "10:30 pm", "at 7 p.m."
    let m = /(?:\bat\s+)?\b(\d{1,2})(?::(\d{2}))?\s*([ap])\.?m\.?(?![a-z])/i.exec(lower);
    if (m) {
      let h = parseInt(m[1], 10) % 12;
      if (m[3] === "p") h += 12;
      const mm = m[2] ? parseInt(m[2], 10) : 0;
      if (h <= 23 && mm <= 59) {
        time = { h, m: mm };
        take(m);
      }
    }
    // "at 3", "at 14:30" (no am/pm)
    if (!time) {
      m = /\bat\s+(\d{1,2})(?::(\d{2}))?(?![\w:/-])/i.exec(lower);
      if (m) {
        const h = parseInt(m[1], 10);
        const mm = m[2] ? parseInt(m[2], 10) : 0;
        if (h <= 23 && mm <= 59) {
          time = { h: m[2] && h > 12 ? h : guessHour(h), m: mm };
          take(m);
        }
      }
    }
    // bare "14:30" / "9:00"
    if (!time) {
      m = /(?<![\w:/-])(\d{1,2}):(\d{2})(?![\w:/-])/.exec(lower);
      if (m) {
        const h = parseInt(m[1], 10);
        const mm = parseInt(m[2], 10);
        if (h <= 23 && mm <= 59) {
          time = { h: h > 12 ? h : guessHour(h), m: mm };
          take(m);
        }
      }
    }
    // "noon", "tomorrow morning", "in the evening", "tonight"
    if (!time) {
      m = /(?:\b(?:in\s+the|this)\s+)?\b(morning|noon|midday|afternoon|evening|tonight)\b/i.exec(lower);
      if (m) {
        time = { h: WORD_TIMES[m[1]], m: 0 };
        if (m[1] === "tonight") date = { kind: "offset", days: 0 };
        take(m);
      }
    }
  }

  // ---- date
  {
    let m: RegExpExecArray | null;
    if (!date && (m = /\b(?:the\s+)?day\s+after\s+tomorrow\b/i.exec(lower))) {
      date = { kind: "offset", days: 2 };
      take(m);
    }
    if (!date && (m = /\b(today|tomorrow|tmrw|tmr)\b/i.exec(lower))) {
      date = { kind: "offset", days: m[1] === "today" ? 0 : 1 };
      take(m);
    }
    if (!date && (m = /\bin\s+(\d+|a|an)\s+(minute|minutes|min|mins|hour|hours|hr|hrs|day|days|week|weeks)\b/i.exec(lower))) {
      const n = /^\d/.test(m[1]) ? parseInt(m[1], 10) : 1;
      const u = m[2];
      if (/^(min|minute)/.test(u)) relative = n;
      else if (/^(h|hour)/.test(u)) relative = n * 60;
      else if (/^day/.test(u)) date = { kind: "offset", days: n };
      else date = { kind: "offset", days: n * 7 };
      take(m);
    }
    // "nov 5", "november 5th, 2026", "on dec 1"
    if (!date && !relative && (m = new RegExp(`(?:\\bon\\s+)?\\b(${MONTH_RE})\\.?\\s+(\\d{1,2})(?:st|nd|rd|th)?\\b(?:,?\\s+(\\d{4})\\b)?`, "i").exec(lower))) {
      const month = MONTH_PREFIX.indexOf(m[1].slice(0, 3));
      const day = parseInt(m[2], 10);
      if (month >= 0 && day >= 1 && day <= 31) {
        date = { kind: "ymd", year: m[3] ? parseInt(m[3], 10) : null, month, day };
        take(m);
      }
    }
    // "5 nov", "5th of november"
    if (!date && !relative && (m = new RegExp(`(?:\\bon\\s+)?\\b(\\d{1,2})(?:st|nd|rd|th)?\\s+(?:of\\s+)?(${MONTH_RE})\\b(?:,?\\s+(\\d{4})\\b)?`, "i").exec(lower))) {
      const month = MONTH_PREFIX.indexOf(m[2].slice(0, 3));
      const day = parseInt(m[1], 10);
      if (month >= 0 && day >= 1 && day <= 31) {
        date = { kind: "ymd", year: m[3] ? parseInt(m[3], 10) : null, month, day };
        take(m);
      }
    }
    // "2026-11-05"
    if (!date && !relative && (m = /\b(\d{4})-(\d{2})-(\d{2})\b/.exec(lower))) {
      date = { kind: "ymd", year: parseInt(m[1], 10), month: parseInt(m[2], 10) - 1, day: parseInt(m[3], 10) };
      take(m);
    }
    // "11/5", "on 11/5/2026" (US order). Bare fractions like "1/2 cup" are
    // only taken when "on" precedes them or a time is also present.
    if (!date && !relative && (m = /(?:\b(on)\s+)?\b(\d{1,2})\/(\d{1,2})(?:\/(\d{2,4}))?\b/.exec(lower))) {
      const month = parseInt(m[2], 10) - 1;
      const day = parseInt(m[3], 10);
      if ((m[1] || time) && month >= 0 && month <= 11 && day >= 1 && day <= 31) {
        let year: number | null = m[4] ? parseInt(m[4], 10) : null;
        if (year !== null && year < 100) year += 2000;
        date = { kind: "ymd", year, month, day };
        take(m);
      }
    }
    // Weekdays: "next friday", "this fri", "on monday", or a bare day name
    // when a time is given or it ends the text. "last/since/from friday"
    // refer to the past and are left alone.
    if (!date && !relative) {
      const re = new RegExp(`(?:\\b(next|this|on|last|since|from)\\s+)?\\b(${DAY_RE})\\b\\.?`, "gi");
      while ((m = re.exec(lower))) {
        const prefix = m[1]?.toLowerCase();
        if (prefix === "last" || prefix === "since" || prefix === "from") continue;
        const atEnd = m.index + m[0].length >= lower.trimEnd().length;
        if (!prefix && !time && !atEnd) continue;
        date = { kind: "weekday", day: DAY_PREFIX.indexOf(m[2].slice(0, 3).toLowerCase()), next: prefix === "next" };
        take(m);
        break;
      }
    }
  }

  if (!time && !date && relative === null) return null;

  const title = stripSpans(text, spans);
  if (!title) return null;

  // ---- resolve to an instant
  let start: Date;
  let allDay = false;
  if (relative !== null) {
    start = new Date(now.getTime() + relative * 60_000);
    start.setSeconds(0, 0);
  } else {
    let base: Date;
    if (!date) {
      base = dayAt(now, 0);
    } else if (date.kind === "offset") {
      base = dayAt(now, date.days);
    } else if (date.kind === "weekday") {
      const todayIdx = now.getDay();
      let days: number;
      if (date.next) {
        // The named day of next week (weeks start on Monday).
        const toNextMonday = (8 - todayIdx) % 7 || 7;
        days = toNextMonday + ((date.day - 1 + 7) % 7);
      } else {
        days = (date.day - todayIdx + 7) % 7;
      }
      base = dayAt(now, days);
    } else {
      const year = date.year ?? now.getFullYear();
      base = new Date(year, date.month, date.day);
      if (base.getMonth() !== date.month) return null; // e.g. Feb 30
      // A month/day without a year that already passed means next year.
      if (date.year === null && base.getTime() < dayAt(now, -1).getTime()) base = new Date(year + 1, date.month, date.day);
    }
    if (time) {
      start = new Date(base);
      start.setHours(time.h, time.m, 0, 0);
      // "at 3pm" with no date, said after 3pm: tomorrow.
      if (!date && start.getTime() < now.getTime() - 60_000) {
        start = dayAt(now, 1);
        start.setHours(time.h, time.m, 0, 0);
      }
      // A bare weekday that is today but whose time has passed: next week.
      if (date?.kind === "weekday" && !date.next && start.getTime() < now.getTime() - 60_000) {
        start = dayAt(now, 7);
        start.setHours(time.h, time.m, 0, 0);
      }
    } else {
      start = base;
      allDay = true;
    }
  }

  let end: Date;
  if (allDay) {
    end = new Date(start);
    end.setDate(end.getDate() + 1);
  } else {
    end = new Date(start.getTime() + (duration ?? DEFAULT_DURATION_MIN) * 60_000);
  }
  return { title, start: Math.floor(start.getTime() / 1000), end: Math.floor(end.getTime() / 1000), allDay };
}

/** Short label for a row badge: "Today 3:00 PM", "Tomorrow", "Fri 10:00 AM", "Nov 20 9:00 AM". */
export function fmtSchedule(start: number, allDay: boolean, now: Date = new Date()): string {
  const d = new Date(start * 1000);
  const days = Math.round((dayAt(d, 0).getTime() - dayAt(now, 0).getTime()) / 86_400_000);
  let day: string;
  if (days === 0) day = "Today";
  else if (days === 1) day = "Tomorrow";
  else if (days === -1) day = "Yesterday";
  else if (days > 1 && days < 7) day = d.toLocaleDateString([], { weekday: "short" });
  else day = d.toLocaleDateString([], { month: "short", day: "numeric", ...(d.getFullYear() !== now.getFullYear() ? { year: "numeric" } : {}) });
  return allDay ? day : `${day} ${d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`;
}

/** Long form for tooltips: "Friday, Nov 6 · 10:00 AM – 10:30 AM". */
export function fmtScheduleLong(start: number, end: number, allDay: boolean): string {
  const s = new Date(start * 1000);
  const day = s.toLocaleDateString([], { weekday: "long", month: "short", day: "numeric" });
  if (allDay) return `${day} · all day`;
  const t = (x: number) => new Date(x * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  return `${day} · ${t(start)} – ${t(end)}`;
}
