//! Times as the new-session screen shows them ("2 h ago", "yesterday", "Aug 12").

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// An ISO 8601 timestamp (`2026-09-26T14:03:11.123Z`, or with a `+02:00` offset).
pub fn parse_iso8601(text: &str) -> Option<SystemTime> {
    let (date, time) = text.split_once('T')?;
    let mut ymd = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (ymd.next()??, ymd.next()??, ymd.next()??);
    let (clock, offset) = match time.find(['Z', '+', '-']) {
        Some(i) => (&time[..i], &time[i..]),
        None => (time, "Z"),
    };
    let mut hms = clock.splitn(3, ':');
    let h: i64 = hms.next()?.parse().ok()?;
    let min: i64 = hms.next()?.parse().ok()?;
    let s: f64 = hms.next().unwrap_or("0").parse().ok()?;
    let offset_secs = match offset {
        "Z" => 0,
        o => {
            let sign = if o.starts_with('-') { -1 } else { 1 };
            let (oh, om) = o[1..].split_once(':').unwrap_or((&o[1..], "0"));
            sign * (oh.parse::<i64>().ok()? * 3600 + om.parse::<i64>().ok()? * 60)
        }
    };
    let secs = days_from_civil(y, m, d) * 86400 + h * 3600 + min * 60 + s as i64 - offset_secs;
    Some(UNIX_EPOCH + Duration::from_secs(u64::try_from(secs).ok()?))
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (m, d)
}

/// How long ago `then` was, in a few words, with days in local time.
pub fn ago(then: SystemTime) -> String {
    ago_at(then, SystemTime::now(), local_offset())
}

/// The local time zone's offset from UTC, in seconds.
#[cfg(unix)]
fn local_offset() -> i64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() { 0 } else { tm.tm_gmtoff as i64 }
}

#[cfg(windows)]
fn local_offset() -> i64 {
    chrono::Local::now().offset().local_minus_utc().into()
}

/// A moment (Unix seconds) as a local clock time, "18:40", with the weekday
/// ("Sat 09:10") when it isn't today.
pub fn clock(at: u64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    clock_at(at, now, local_offset())
}

fn clock_at(at: u64, now: u64, offset: i64) -> String {
    let local = at as i64 + offset;
    let (h, m) = (local.rem_euclid(86400) / 3600, local.rem_euclid(3600) / 60);
    let day = local.div_euclid(86400);
    if day == (now as i64 + offset).div_euclid(86400) {
        return format!("{h}:{m:02}");
    }
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    format!("{} {h}:{m:02}", WEEKDAYS[day.rem_euclid(7) as usize])
}

/// A moment as a day and clock time, in local time: "today at 9:14",
/// "yesterday at 16:40", "Monday at 11:02" within the week, else "Aug 12 at 9:14".
pub fn day_at(then: SystemTime) -> String {
    day_at_with(then, SystemTime::now(), local_offset())
}

fn day_at_with(then: SystemTime, now: SystemTime, offset: i64) -> String {
    let local = |t: SystemTime| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 + offset;
    let (then_local, now_local) = (local(then), local(now));
    let (h, m) = (then_local.rem_euclid(86400) / 3600, then_local.rem_euclid(3600) / 60);
    let day = then_local.div_euclid(86400);
    let day_name = match now_local.div_euclid(86400) - day {
        0 => "today".to_owned(),
        1 => "yesterday".to_owned(),
        2..=6 => {
            const WEEKDAYS: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];
            WEEKDAYS[day.rem_euclid(7) as usize].to_owned()
        }
        _ => {
            const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            let (month, d) = civil_from_days(day);
            format!("{} {d}", MONTHS[(month - 1) as usize])
        }
    };
    format!("{day_name} at {h}:{m:02}")
}

fn ago_at(then: SystemTime, now: SystemTime, offset: i64) -> String {
    let secs = now.duration_since(then).unwrap_or_default().as_secs() as i64;
    let then_secs = then.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 + offset;
    let now_secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 + offset;
    let days = now_secs.div_euclid(86400) - then_secs.div_euclid(86400);
    match secs {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if days == 0 => format!("{} h ago", s / 3600),
        _ if days == 1 => "yesterday".into(),
        _ if days < 7 => format!("{days} days ago"),
        _ if days < 14 => "last week".into(),
        _ => {
            const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            let (m, d) = civil_from_days(then_secs.div_euclid(86400));
            format!("{} {d}", MONTHS[(m - 1) as usize])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> SystemTime {
        parse_iso8601(text).unwrap()
    }

    #[test]
    fn parses_utc_and_offsets() {
        assert_eq!(at("1970-01-02T00:00:00Z"), UNIX_EPOCH + Duration::from_secs(86400));
        assert_eq!(at("2026-09-26T12:00:00.500Z"), at("2026-09-26T14:00:00+02:00"));
        assert_eq!(at("2026-09-26T12:00:00Z").duration_since(UNIX_EPOCH).unwrap().as_secs(), 1790424000);
        assert_eq!(parse_iso8601("yesterday"), None);
    }

    #[test]
    fn clock_times_say_the_day_when_it_isnt_today() {
        let secs = |t: &str| at(t).duration_since(UNIX_EPOCH).unwrap().as_secs();
        let now = secs("2026-09-26T15:00:00Z");
        assert_eq!(clock_at(secs("2026-09-26T18:40:00Z"), now, 0), "18:40");
        assert_eq!(clock_at(secs("2026-09-27T09:05:00Z"), now, 0), "Sun 9:05");
        assert_eq!(clock_at(secs("2026-09-26T22:30:00Z"), now, 7200), "Sun 0:30");
    }

    #[test]
    fn day_and_time_say_today_yesterday_the_weekday_or_the_date() {
        let now = at("2026-09-26T15:00:00Z");
        let day = |then| day_at_with(at(then), now, 0);
        assert_eq!(day("2026-09-26T09:14:00Z"), "today at 9:14");
        assert_eq!(day("2026-09-25T16:40:00Z"), "yesterday at 16:40");
        assert_eq!(day("2026-09-21T11:02:00Z"), "Monday at 11:02");
        assert_eq!(day("2026-09-12T08:05:00Z"), "Sep 12 at 8:05");
        assert_eq!(day_at_with(at("2026-09-25T23:30:00Z"), now, 7200), "today at 1:30");
    }

    #[test]
    fn says_how_long_ago() {
        let now = at("2026-09-26T15:00:00Z");
        let ago = |then| ago_at(then, now, 0);
        assert_eq!(ago(at("2026-09-26T14:59:30Z")), "just now");
        assert_eq!(ago(at("2026-09-26T14:10:00Z")), "50 min ago");
        assert_eq!(ago(at("2026-09-26T13:00:00Z")), "2 h ago");
        assert_eq!(ago(at("2026-09-25T23:00:00Z")), "yesterday");
        assert_eq!(ago(at("2026-09-22T09:00:00Z")), "4 days ago");
        assert_eq!(ago(at("2026-09-17T09:00:00Z")), "last week");
        assert_eq!(ago(at("2026-08-12T09:00:00Z")), "Aug 12");
        // 00:30 in UTC+2 is already the next day there.
        assert_eq!(ago_at(at("2026-09-25T20:00:00Z"), at("2026-09-25T22:30:00Z"), 7200), "yesterday");
        assert_eq!(ago_at(at("2026-09-25T20:00:00Z"), at("2026-09-25T22:30:00Z"), 0), "2 h ago");
    }
}
