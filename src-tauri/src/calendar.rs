//! Minimal iCalendar (.ics) fetcher + parser with recurrence expansion.

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use rrule::RRuleSet;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Serialize)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub location: Option<String>,
    pub url: Option<String>,
}

struct Prop {
    name: String,
    params: HashMap<String, String>,
    value: String,
}

fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if (line.starts_with(' ') || line.starts_with('\t')) && !lines.is_empty() {
            lines.last_mut().unwrap().push_str(&line[1..]);
        } else {
            lines.push(line.to_string());
        }
    }
    lines
}

fn parse_prop(line: &str) -> Option<Prop> {
    // NAME;PARAM=VAL;PARAM2=VAL:value   (value may contain ':')
    let mut in_quotes = false;
    let mut colon = None;
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let head = &line[..colon];
    let value = line[colon + 1..].to_string();
    let mut parts = head.split(';');
    let name = parts.next()?.to_uppercase();
    let mut params = HashMap::new();
    for p in parts {
        if let Some((k, v)) = p.split_once('=') {
            params.insert(k.to_uppercase(), v.trim_matches('"').to_string());
        }
    }
    Some(Prop { name, params, value })
}

fn unescape(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

/// Parse a DTSTART/DTEND value into (utc instant, is_all_day).
fn parse_dt(p: &Prop) -> Option<(DateTime<Utc>, bool)> {
    let v = p.value.trim();
    if p.params.get("VALUE").map(|s| s.as_str()) == Some("DATE") || v.len() == 8 {
        let d = NaiveDate::parse_from_str(v, "%Y%m%d").ok()?;
        let local = Local.from_local_datetime(&d.and_hms_opt(0, 0, 0)?).single()?;
        return Some((local.with_timezone(&Utc), true));
    }
    if let Some(stripped) = v.strip_suffix('Z') {
        let ndt = NaiveDateTime::parse_from_str(stripped, "%Y%m%dT%H%M%S").ok()?;
        return Some((Utc.from_utc_datetime(&ndt), false));
    }
    let ndt = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok()?;
    if let Some(tzid) = p.params.get("TZID") {
        if let Ok(tz) = tzid.parse::<Tz>() {
            let dt = tz.from_local_datetime(&ndt).single()?;
            return Some((dt.with_timezone(&Utc), false));
        }
    }
    // Floating time: interpret as local.
    let dt = Local.from_local_datetime(&ndt).single()?;
    Some((dt.with_timezone(&Utc), false))
}

#[derive(Default)]
struct RawEvent {
    uid: String,
    summary: String,
    location: Option<String>,
    url: Option<String>,
    dtstart: Option<Prop>,
    dtend: Option<Prop>,
    duration: Option<String>,
    rrule: Option<String>,
    exdates: Vec<Prop>,
    recurrence_id: Option<Prop>,
    status: Option<String>,
}

fn parse_events(text: &str) -> Vec<RawEvent> {
    let mut events = Vec::new();
    let mut cur: Option<RawEvent> = None;
    let mut nested = 0; // skip VALARM etc. inside VEVENT
    for line in unfold(text) {
        let Some(p) = parse_prop(&line) else { continue };
        match (p.name.as_str(), p.value.as_str()) {
            ("BEGIN", "VEVENT") => cur = Some(RawEvent::default()),
            ("END", "VEVENT") => {
                if let Some(e) = cur.take() {
                    events.push(e);
                }
            }
            ("BEGIN", _) if cur.is_some() => nested += 1,
            ("END", _) if cur.is_some() && nested > 0 => nested -= 1,
            _ => {
                if nested > 0 {
                    continue;
                }
                if let Some(e) = cur.as_mut() {
                    match p.name.as_str() {
                        "UID" => e.uid = p.value.clone(),
                        "SUMMARY" => e.summary = unescape(&p.value),
                        "LOCATION" => {
                            e.location = Some(unescape(&p.value)).filter(|s| !s.is_empty())
                        }
                        "URL" => e.url = Some(p.value.clone()),
                        "DTSTART" => e.dtstart = Some(p),
                        "DTEND" => e.dtend = Some(p),
                        "DURATION" => e.duration = Some(p.value.clone()),
                        "RRULE" => e.rrule = Some(p.value.clone()),
                        "EXDATE" => e.exdates.push(p),
                        "RECURRENCE-ID" => e.recurrence_id = Some(p),
                        "STATUS" => e.status = Some(p.value.to_uppercase()),
                        _ => {}
                    }
                }
            }
        }
    }
    events
}

/// Parse an ISO 8601 duration like PT1H30M / P1D.
fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim_start_matches('+');
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let s = s.strip_prefix('P')?;
    let mut total = Duration::zero();
    let mut num = String::new();
    let mut in_time = false;
    for ch in s.chars() {
        match ch {
            'T' => in_time = true,
            d if d.is_ascii_digit() => num.push(d),
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total = total
                    + match (unit, in_time) {
                        ('W', _) => Duration::weeks(n),
                        ('D', _) => Duration::days(n),
                        ('H', true) => Duration::hours(n),
                        ('M', true) => Duration::minutes(n),
                        ('S', true) => Duration::seconds(n),
                        _ => return None,
                    };
            }
        }
    }
    Some(if neg { -total } else { total })
}

fn dt_line(name: &str, p: &Prop) -> String {
    let mut s = String::from(name);
    if let Some(tz) = p.params.get("TZID") {
        s.push_str(";TZID=");
        s.push_str(tz);
    }
    if p.params.get("VALUE").map(|s| s.as_str()) == Some("DATE") {
        s.push_str(";VALUE=DATE");
    }
    s.push(':');
    s.push_str(&p.value);
    s
}

pub fn expand(text: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Meeting> {
    let events = parse_events(text);

    // Overridden instances of recurring events: (uid, recurrence instant) -> skip in expansion.
    let mut overridden: Vec<(String, DateTime<Utc>)> = Vec::new();
    for e in &events {
        if let (Some(rid), false) = (&e.recurrence_id, e.uid.is_empty()) {
            if let Some((dt, _)) = parse_dt(rid) {
                overridden.push((e.uid.clone(), dt));
            }
        }
    }

    let mut out = Vec::new();
    for e in &events {
        if e.status.as_deref() == Some("CANCELLED") {
            continue;
        }
        let Some(dtstart) = &e.dtstart else { continue };
        let Some((start, all_day)) = parse_dt(dtstart) else { continue };
        let dur = match (&e.dtend, &e.duration) {
            (Some(end), _) => parse_dt(end).map(|(end, _)| end - start),
            (None, Some(d)) => parse_duration(d),
            _ => None,
        }
        .unwrap_or(if all_day { Duration::days(1) } else { Duration::minutes(30) });

        let mut push = |s: DateTime<Utc>, suffix: &str| {
            let end = s + dur;
            if end <= from || s >= to {
                return;
            }
            out.push(Meeting {
                id: format!("{}{}", e.uid, suffix),
                title: if e.summary.is_empty() { "(No title)".into() } else { e.summary.clone() },
                start: s.timestamp(),
                end: end.timestamp(),
                all_day,
                location: e.location.clone(),
                url: e.url.clone(),
            });
        };

        match &e.rrule {
            None => push(start, ""),
            Some(rule) => {
                let mut src = dt_line("DTSTART", dtstart);
                src.push_str("\nRRULE:");
                src.push_str(rule);
                for ex in &e.exdates {
                    src.push('\n');
                    src.push_str(&dt_line("EXDATE", ex));
                }
                let Ok(set) = src.parse::<RRuleSet>() else {
                    // Can't expand; fall back to the base instance.
                    push(start, "");
                    continue;
                };
                let tz = set.get_dt_start().timezone();
                let after = (from - dur).with_timezone(&tz);
                let before = to.with_timezone(&tz);
                let result = set.after(after).before(before).all(500);
                for d in result.dates {
                    let s = d.with_timezone(&Utc);
                    if overridden.iter().any(|(u, t)| *u == e.uid && *t == s) {
                        continue;
                    }
                    push(s, &format!("@{}", s.timestamp()));
                }
            }
        }
    }
    out.sort_by_key(|m| (m.start, m.end));
    out.dedup_by(|a, b| a.id == b.id);
    out
}

pub async fn fetch(url: &str) -> Result<String, String> {
    let url = url.trim().replacen("webcal://", "https://", 1);
    let client = reqwest::Client::builder()
        .user_agent("deskpad/0.1")
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Calendar returned HTTP {}", resp.status()));
    }
    resp.text().await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n\
BEGIN:VEVENT\r\nUID:single@test\r\nSUMMARY:One-off sync\r\n\
DTSTART;TZID=America/Los_Angeles:20261029T100000\r\nDTEND;TZID=America/Los_Angeles:20261029T103000\r\n\
LOCATION:https://meet.example.com/abc\r\nBEGIN:VALARM\r\nTRIGGER:-PT10M\r\nEND:VALARM\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:weekly@test\r\nSUMMARY:Weekly standup\\, team\r\n\
DTSTART;TZID=America/Los_Angeles:20260105T090000\r\nDURATION:PT15M\r\n\
RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR\r\nEXDATE;TZID=America/Los_Angeles:20261030T090000\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:weekly@test\r\nRECURRENCE-ID;TZID=America/Los_Angeles:20261028T090000\r\n\
SUMMARY:Standup (moved)\r\nDTSTART;TZID=America/Los_Angeles:20261028T140000\r\nDTEND;TZID=America/Los_Angeles:20261028T141500\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:allday@test\r\nSUMMARY:Offsite\r\nDTSTART;VALUE=DATE:20261029\r\nDTEND;VALUE=DATE:20261030\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:cancelled@test\r\nSUMMARY:Nope\r\nSTATUS:CANCELLED\r\nDTSTART:20261029T150000Z\r\nDTEND:20261029T160000Z\r\nEND:VEVENT\r\n\
END:VCALENDAR\r\n";

    #[test]
    fn expands_recurrence_exceptions_and_overrides() {
        let from = Utc.with_ymd_and_hms(2026, 10, 28, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap();
        let ms = expand(ICS, from, to);
        let titles: Vec<&str> = ms.iter().map(|m| m.title.as_str()).collect();

        // Window is Wed 28 .. Sat 31. The Wed standup is replaced by its RECURRENCE-ID
        // override and the Fri standup is EXDATE'd, so no plain "Weekly standup" remains.
        assert!(titles.contains(&"Standup (moved)"), "{titles:?}");
        assert!(titles.contains(&"One-off sync"), "{titles:?}");
        assert!(titles.contains(&"Offsite"), "{titles:?}");
        assert!(!titles.contains(&"Nope"), "cancelled event leaked: {titles:?}");
        assert_eq!(
            titles.iter().filter(|t| **t == "Weekly standup, team").count(),
            0,
            "Wed instance should be replaced by override and Fri excluded: {titles:?}"
        );

        let one = ms.iter().find(|m| m.id == "single@test").unwrap();
        // 10:00 PDT == 17:00 UTC
        assert_eq!(one.start, Utc.with_ymd_and_hms(2026, 10, 29, 17, 0, 0).unwrap().timestamp());
        assert_eq!(one.end - one.start, 30 * 60);
        assert_eq!(one.location.as_deref(), Some("https://meet.example.com/abc"));

        let off = ms.iter().find(|m| m.id == "allday@test").unwrap();
        assert!(off.all_day);
        assert_eq!(off.end - off.start, 86_400);

        // Sorted by start.
        assert!(ms.windows(2).all(|w| w[0].start <= w[1].start));
    }

    #[test]
    fn weekly_rule_produces_instances_inside_window() {
        let from = Utc.with_ymd_and_hms(2026, 11, 2, 0, 0, 0).unwrap(); // Mon
        let to = Utc.with_ymd_and_hms(2026, 11, 7, 0, 0, 0).unwrap();
        let ms = expand(ICS, from, to);
        let standups: Vec<_> = ms.iter().filter(|m| m.title == "Weekly standup, team").collect();
        assert_eq!(standups.len(), 3, "{:?}", ms.iter().map(|m| &m.title).collect::<Vec<_>>());
        assert_eq!(standups[0].end - standups[0].start, 15 * 60);
    }

    #[test]
    fn duration_parsing() {
        assert_eq!(parse_duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(parse_duration("P1D"), Some(Duration::days(1)));
        assert_eq!(parse_duration("-PT5M"), Some(Duration::minutes(-5)));
    }
}
