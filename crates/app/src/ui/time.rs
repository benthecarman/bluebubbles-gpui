//! Dates as Messages writes them, in local time with AM/PM.

use chrono::{DateTime, Datelike, Local, TimeZone};

pub fn local(ms: i64) -> DateTime<Local> {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(Local::now)
}

/// "9:41 AM".
pub fn clock<Tz: TimeZone>(at: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    at.format("%-I:%M %p").to_string()
}

/// Days between `at`'s date and `now`'s date (0 = same day).
fn days_ago<Tz: TimeZone>(at: &DateTime<Tz>, now: &DateTime<Tz>) -> i64 {
    (now.date_naive() - at.date_naive()).num_days()
}

/// The sidebar's time: "9:41 AM", "Yesterday", "Monday", "10/20/26".
pub fn sidebar<Tz: TimeZone>(at: &DateTime<Tz>, now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match days_ago(at, now) {
        i64::MIN..=0 => clock(at),
        1 => "Yesterday".into(),
        2..=6 => at.format("%A").to_string(),
        _ => format!("{}/{}/{:02}", at.month(), at.day(), at.year() % 100),
    }
}

/// A transcript separator, as a bold day and the rest: ("Today",
/// "9:41 AM"), ("Mon, Oct 13", "at 9:41 AM"), ("Oct 13, 2024", "at 9:41 AM").
pub fn separator<Tz: TimeZone>(at: &DateTime<Tz>, now: &DateTime<Tz>) -> (String, String)
where
    Tz::Offset: std::fmt::Display,
{
    let time = clock(at);
    match days_ago(at, now) {
        i64::MIN..=0 => ("Today".into(), time),
        1 => ("Yesterday".into(), time),
        2..=6 => (at.format("%A").to_string(), time),
        _ if at.year() == now.year() => (at.format("%a, %b %-d").to_string(), format!("at {time}")),
        _ => (at.format("%b %-d, %Y").to_string(), format!("at {time}")),
    }
}

/// "Read 9:41 AM", "Read Yesterday", "Read 10/20/26".
pub fn receipt<Tz: TimeZone>(verb: &str, at: &DateTime<Tz>, now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    format!("{verb} {}", sidebar(at, now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn sidebar_times() {
        let now = at("2026-10-27T15:00:00Z");
        assert_eq!(sidebar(&at("2026-10-27T09:41:00Z"), &now), "9:41 AM");
        assert_eq!(sidebar(&at("2026-10-27T14:05:00Z"), &now), "2:05 PM");
        assert_eq!(sidebar(&at("2026-10-26T23:00:00Z"), &now), "Yesterday");
        assert_eq!(sidebar(&at("2026-10-23T10:00:00Z"), &now), "Friday");
        assert_eq!(sidebar(&at("2026-10-20T10:00:00Z"), &now), "10/20/26");
    }

    #[test]
    fn separators() {
        let now = at("2026-10-27T15:00:00Z");
        let s = |t| separator(&at(t), &now);
        assert_eq!(
            s("2026-10-27T09:41:00Z"),
            ("Today".into(), "9:41 AM".into())
        );
        assert_eq!(
            s("2026-10-26T21:15:00Z"),
            ("Yesterday".into(), "9:15 PM".into())
        );
        assert_eq!(
            s("2026-10-13T09:41:00Z"),
            ("Tue, Oct 13".into(), "at 9:41 AM".into())
        );
        assert_eq!(
            s("2024-10-13T09:41:00Z"),
            ("Oct 13, 2024".into(), "at 9:41 AM".into())
        );
    }
}
