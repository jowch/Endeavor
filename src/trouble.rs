//! Why a turn ended without its answer, in Endeavor's words, from what Claude
//! Code's adapter reports: an error kind (`errorKind` in the error's data) and
//! its message. A usage limit is waiting, not failing: its reset time is read
//! from the message, and the line above the composer counts down to it.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::agent::Agent;

/// What stopped a turn, as the chat tells it.
#[derive(Clone, Debug, PartialEq)]
pub enum Trouble {
    /// Claude's sign-in ran out.
    SignIn,
    /// The account's usage limit: the message waits until it resets.
    UsageLimit(Option<Reset>),
    /// Anthropic's servers are overloaded.
    Busy,
    /// Too many requests in a short time (not the account's usage limit).
    RateLimited,
    /// The connection to Claude dropped.
    Connection,
    /// Any other error from Claude's side.
    Server,
}

/// Claude Code's own openings for a message about the account's usage, as the
/// SDK's `USAGE_LIMIT_ERROR_PREFIXES` lists them, and the older CLI's.
const USAGE_LIMIT: [&str; 9] = [
    "You've hit your",
    "You've reached your",
    "You're out of usage credits",
    "You're out of extra usage",
    "Your org is out of usage",
    "Your seat type doesn't include",
    "Your usage allocation has been disabled",
    "Your group's usage limit",
    "Claude AI usage limit reached",
];

const CONNECTION: [&str; 10] = [
    "unable to connect",
    "connection error",
    "connection reset",
    "connection closed",
    "econnreset",
    "etimedout",
    "socket hang up",
    "fetch failed",
    "network error",
    "request timed out",
];

/// The error's own words: the adapter's "Internal error: " and Claude Code's
/// "API Error: " come off.
pub fn error_text(message: &str) -> &str {
    let text = message.trim();
    let text = text.strip_prefix("Internal error: ").unwrap_or(text);
    text.strip_prefix("API Error: ").unwrap_or(text)
}

pub fn classify(kind: Option<&str>, message: &str) -> Trouble {
    let text = error_text(message);
    let lower = text.to_lowercase();
    if USAGE_LIMIT.iter().any(|p| text.starts_with(p)) || lower.contains("usage limit reached") {
        return Trouble::UsageLimit(parse_reset(text));
    }
    match kind {
        Some("authentication_failed" | "oauth_org_not_allowed") => return Trouble::SignIn,
        Some("overloaded") => return Trouble::Busy,
        Some("rate_limit") => return Trouble::RateLimited,
        Some("transport_lost" | "worker_shutdown") => return Trouble::Connection,
        _ => {}
    }
    if text.contains("Please run /login") || lower.contains("oauth token has expired") {
        return Trouble::SignIn;
    }
    if lower.contains("overloaded") || text.starts_with("529") {
        return Trouble::Busy;
    }
    if lower.contains("rate_limit_error") || text.starts_with("429") {
        return Trouble::RateLimited;
    }
    if CONNECTION.iter().any(|w| lower.contains(w)) {
        return Trouble::Connection;
    }
    Trouble::Server
}

impl Trouble {
    /// The card's reason, for a turn that got no reply.
    pub fn reason(&self, agent: Agent) -> &'static str {
        match self {
            Trouble::Busy => match agent {
                Agent::Claude => "Anthropic's servers are busy right now.",
                Agent::Codex => "OpenAI's servers are busy right now.",
                Agent::Antigravity => "Google's servers are busy right now.",
            },
            Trouble::RateLimited => crate::agent_text!(agent, "", " got too many requests in a short time."),
            Trouble::Connection => crate::agent_text!(agent, "The connection to ", " dropped before a reply came."),
            Trouble::SignIn | Trouble::UsageLimit(_) | Trouble::Server => crate::agent_text!(agent, "Something went wrong on ", "'s side."),
        }
    }
}

/// When a usage limit resets, as Claude Code's message says it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reset {
    /// A local clock time ("3pm", "3:30pm"): the next time it comes round. With
    /// a date ("Oct 3, 3pm") when it's more than a day away.
    At { date: Option<(u32, u32, Option<i32>)>, hour: u32, minute: u32 },
    /// Unix seconds, as the older CLI wrote it ("Claude AI usage limit reached|1759258800").
    Unix(i64),
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The reset time in a usage-limit message: "… · resets 3pm (America/Los_Angeles)",
/// "… · resets Oct 3, 3pm (…)", "Your limit will reset at 3pm (…).", or "…|1759258800".
pub fn parse_reset(text: &str) -> Option<Reset> {
    if let Some((_, secs)) = text.rsplit_once('|')
        && let Ok(secs) = secs.trim().parse::<i64>()
    {
        return Some(Reset::Unix(secs));
    }
    let start = ["resets at ", "resets ", "reset at "].iter().find_map(|w| text.find(w).map(|i| i + w.len()))?;
    let rest = &text[start..];
    let end = [" (", " ·", ". "].iter().filter_map(|w| rest.find(w)).min().unwrap_or(rest.len());
    let when = rest[..end].trim().trim_end_matches('.');
    let (date, clock) = match when.rsplit_once(", ") {
        Some((date, clock)) => (Some(parse_date(date)?), clock),
        None => (None, when),
    };
    let (hour, minute) = parse_clock(clock)?;
    Some(Reset::At { date, hour, minute })
}

/// "Oct 3" or "Oct 3, 2027" (the year comes before the last comma's clock).
fn parse_date(text: &str) -> Option<(u32, u32, Option<i32>)> {
    let (month_day, year) = match text.split_once(", ") {
        Some((md, y)) => (md, Some(y.trim().parse().ok()?)),
        None => (text, None),
    };
    let (month, day) = month_day.trim().split_once(' ')?;
    let month = MONTHS.iter().position(|m| *m == month)? as u32 + 1;
    Some((month, day.trim().parse().ok()?, year))
}

/// "3pm", "3:30pm", "12am", "3 PM".
fn parse_clock(text: &str) -> Option<(u32, u32)> {
    let lower = text.trim().to_lowercase();
    let (digits, pm) = if let Some(d) = lower.strip_suffix("pm") {
        (d.trim(), true)
    } else {
        (lower.strip_suffix("am")?.trim(), false)
    };
    let (h, m) = digits.split_once(':').unwrap_or((digits, "0"));
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    if !(1..=12).contains(&h) || m > 59 {
        return None;
    }
    Some((h % 12 + if pm { 12 } else { 0 }, m))
}

/// The moment a reset comes, from `now` and the local time zone's offset from
/// UTC in seconds: a clock time without a date is the next one after `now`.
pub fn resolve(reset: Reset, now: SystemTime, offset: i64) -> Option<SystemTime> {
    let secs = match reset {
        Reset::Unix(secs) => secs,
        Reset::At { date, hour, minute } => {
            let now_local = now.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64 + offset;
            let day = now_local.div_euclid(86400);
            let clock = i64::from(hour) * 3600 + i64::from(minute) * 60;
            let local = match date {
                None => {
                    let today = day * 86400 + clock;
                    if today > now_local { today } else { today + 86400 }
                }
                Some((month, d, year)) => {
                    let (this_year, ..) = civil(day);
                    let at = |y: i64| days_from_civil(y, i64::from(month), i64::from(d)) * 86400 + clock;
                    match year {
                        Some(y) => at(i64::from(y)),
                        // Past already this year: next year's (a reset in early January, seen in December).
                        None if at(this_year) + 86400 < now_local => at(this_year + 1),
                        None => at(this_year),
                    }
                }
            };
            local - offset
        }
    };
    Some(UNIX_EPOCH + Duration::from_secs(u64::try_from(secs).ok()?))
}

/// How long until a reset, as the line says it: "1 h 12 min", "12 min", "40 s",
/// "2 d 4 h". Minutes round up, so it never says "0 min".
pub fn countdown(left: Duration) -> String {
    let secs = left.as_secs();
    if secs < 60 {
        return format!("{} s", secs.max(1));
    }
    let minutes = secs.div_ceil(60);
    let (d, h, m) = (minutes / 1440, minutes % 1440 / 60, minutes % 60);
    match (d, h, m) {
        (0, 0, m) => format!("{m} min"),
        (0, h, 0) => format!("{h} h"),
        (0, h, m) => format!("{h} h {m} min"),
        (d, 0, _) => format!("{d} d"),
        (d, h, _) => format!("{d} d {h} h"),
    }
}

/// A moment as the line names it: "at 3:00 PM" today, else "on Oct 3 at 3:00 PM".
pub fn when_text(at: SystemTime, now: SystemTime, offset: i64) -> String {
    let local = |t: SystemTime| t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64) + offset;
    let (at_local, now_local) = (local(at), local(now));
    let (h, m) = (at_local.rem_euclid(86400) / 3600, at_local.rem_euclid(3600) / 60);
    let (h12, half) = (if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" });
    let clock = format!("{h12}:{m:02} {half}");
    let day = at_local.div_euclid(86400);
    if day == now_local.div_euclid(86400) {
        return format!("at {clock}");
    }
    let (_, month, d) = civil(day);
    format!("on {} {d} at {clock}", MONTHS[(month - 1) as usize])
}

/// The usage-limit line above the composer. `until`: when it resets, if the
/// message said; `waiting`: a message waits to go then.
pub fn usage_line(until: Option<SystemTime>, now: SystemTime, offset: i64, waiting: bool, agent: Agent) -> String {
    let name = agent.name();
    let then = if waiting { "your message sends then".to_owned() } else { format!("{name} can answer again then") };
    match until {
        Some(at) => {
            let left = at.duration_since(now).unwrap_or_default();
            format!("You've reached your {name} usage limit. It resets in {}, {}, and {then}.", countdown(left), when_text(at, now, offset))
        }
        None => format!("You've reached your {name} usage limit. It resets later, and {then}."),
    }
}

/// The local time zone's offset from UTC now, in seconds.
#[cfg(unix)]
pub fn local_offset() -> i64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r only writes the `tm` it's given.
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() { 0 } else { tm.tm_gmtoff as i64 }
}

#[cfg(windows)]
pub fn local_offset() -> i64 {
    chrono::Local::now().offset().local_minus_utc().into()
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// (year, month, day) of a day count since 1970-01-01.
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACIFIC: i64 = -7 * 3600;

    /// A moment given as Pacific (UTC−7) wall-clock time.
    fn pacific(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs((days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 - PACIFIC) as u64)
    }

    #[test]
    fn errors_are_told_by_what_the_user_can_do() {
        let overloaded = r#"API Error: 529 {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
        assert_eq!(classify(Some("server_error"), overloaded), Trouble::Busy);
        assert_eq!(classify(Some("overloaded"), "Internal error: Claude is temporarily overloaded."), Trouble::Busy);
        assert_eq!(classify(Some("server_error"), r#"API Error: 500 {"type":"error","error":{"type":"api_error","message":"Internal server error"}}"#), Trouble::Server);
        assert_eq!(classify(Some("server_error"), "API Error: Unable to connect to API. Check your internet connection"), Trouble::Connection);
        assert_eq!(classify(Some("transport_lost"), "Internal error"), Trouble::Connection);
        assert_eq!(classify(Some("rate_limit"), r#"API Error: 429 {"type":"error","error":{"type":"rate_limit_error","message":"Number of requests has exceeded your rate limit"}}"#), Trouble::RateLimited);
        assert_eq!(classify(Some("authentication_failed"), "Invalid API key · Please run /login"), Trouble::SignIn);
        assert_eq!(classify(None, "Internal error: something odd"), Trouble::Server);
        assert_eq!(Trouble::Busy.reason(Agent::Claude), "Anthropic's servers are busy right now.");
        assert_eq!(Trouble::RateLimited.reason(Agent::Codex), "Codex got too many requests in a short time.");
        assert_eq!(Trouble::Connection.reason(Agent::Codex), "The connection to Codex dropped before a reply came.");
        assert_eq!(Trouble::Server.reason(Agent::Codex), "Something went wrong on Codex's side.");
    }

    #[test]
    fn a_usage_limit_waits_with_its_reset_time() {
        assert_eq!(
            classify(Some("rate_limit"), "Internal error: You've hit your session limit · resets 3pm (America/Los_Angeles)"),
            Trouble::UsageLimit(Some(Reset::At { date: None, hour: 15, minute: 0 }))
        );
        assert_eq!(
            classify(Some("rate_limit"), "You've hit your weekly limit · resets Oct 3, 3:30pm (America/Los_Angeles) · progress saved"),
            Trouble::UsageLimit(Some(Reset::At { date: Some((10, 3, None)), hour: 15, minute: 30 }))
        );
        assert_eq!(
            classify(None, "Claude usage limit reached. Your limit will reset at 3pm (America/Los_Angeles)."),
            Trouble::UsageLimit(Some(Reset::At { date: None, hour: 15, minute: 0 }))
        );
        assert_eq!(classify(None, "Claude AI usage limit reached|1759258800"), Trouble::UsageLimit(Some(Reset::Unix(1759258800))));
        assert_eq!(parse_reset("You've hit your session limit · resets 12am (Europe/London)"), Some(Reset::At { date: None, hour: 0, minute: 0 }));
        assert_eq!(parse_reset("You've hit your Opus limit · resets Jan 2, 2027, 9am (Asia/Tokyo)"), Some(Reset::At { date: Some((1, 2, Some(2027))), hour: 9, minute: 0 }));
        // No time to read: it still waits.
        assert_eq!(classify(None, "You're out of usage credits. Run /usage-credits to keep using Opus"), Trouble::UsageLimit(None));
        assert_eq!(classify(None, "You've hit your org's monthly spend limit · ask your admin"), Trouble::UsageLimit(None));
    }

    #[test]
    fn a_reset_is_the_next_time_its_clock_comes_round() {
        let now = pacific(2026, 9, 30, 13, 48);
        let at3 = Reset::At { date: None, hour: 15, minute: 0 };
        assert_eq!(resolve(at3, now, PACIFIC), Some(pacific(2026, 9, 30, 15, 0)));
        // 9am has gone today: tomorrow's.
        assert_eq!(resolve(Reset::At { date: None, hour: 9, minute: 0 }, now, PACIFIC), Some(pacific(2026, 10, 1, 9, 0)));
        assert_eq!(resolve(Reset::At { date: Some((10, 3, None)), hour: 15, minute: 30 }, now, PACIFIC), Some(pacific(2026, 10, 3, 15, 30)));
        // Early January, seen in December: next year's.
        let december = pacific(2026, 12, 30, 10, 0);
        assert_eq!(resolve(Reset::At { date: Some((1, 2, None)), hour: 9, minute: 0 }, december, PACIFIC), Some(pacific(2027, 1, 2, 9, 0)));
        assert_eq!(resolve(Reset::Unix(1759258800), now, PACIFIC), Some(UNIX_EPOCH + Duration::from_secs(1759258800)));
    }

    #[test]
    fn the_line_counts_down_by_the_minute_then_the_second() {
        assert_eq!(countdown(Duration::from_secs(72 * 60)), "1 h 12 min");
        assert_eq!(countdown(Duration::from_secs(71 * 60 + 30)), "1 h 12 min");
        assert_eq!(countdown(Duration::from_secs(2 * 3600)), "2 h");
        assert_eq!(countdown(Duration::from_secs(12 * 60)), "12 min");
        assert_eq!(countdown(Duration::from_secs(61)), "2 min");
        assert_eq!(countdown(Duration::from_secs(40)), "40 s");
        assert_eq!(countdown(Duration::from_millis(300)), "1 s");
        assert_eq!(countdown(Duration::from_secs(2 * 86400 + 4 * 3600 + 5)), "2 d 4 h");

        let now = pacific(2026, 9, 30, 13, 48);
        assert_eq!(
            usage_line(Some(pacific(2026, 9, 30, 15, 0)), now, PACIFIC, true, Agent::Claude),
            "You've reached your Claude usage limit. It resets in 1 h 12 min, at 3:00 PM, and your message sends then."
        );
        assert_eq!(
            usage_line(Some(pacific(2026, 10, 3, 9, 5)), now, PACIFIC, true, Agent::Claude),
            "You've reached your Claude usage limit. It resets in 2 d 19 h, on Oct 3 at 9:05 AM, and your message sends then."
        );
        assert_eq!(usage_line(None, now, PACIFIC, false, Agent::Claude), "You've reached your Claude usage limit. It resets later, and Claude can answer again then.");
        assert_eq!(usage_line(None, now, PACIFIC, false, Agent::Codex), "You've reached your Codex usage limit. It resets later, and Codex can answer again then.");
    }
}
