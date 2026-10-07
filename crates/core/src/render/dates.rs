//! Dates as the list and the thread show them, in the device's time zone.

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};

fn local<Tz: TimeZone>(ms: i64, zone: &Tz) -> DateTime<Tz> {
    Utc.timestamp_millis_opt(ms).single().unwrap_or_default().with_timezone(zone)
}

/// "9:41 AM" today, "Yesterday", "Mar 3" this year, "Mar 3, 2024" before.
pub fn short<Tz: TimeZone>(ms: i64, now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let date = local(ms, &now.timezone());
    let today = now.date_naive();
    let day = date.date_naive();
    if day == today {
        return date.format("%-I:%M %p").to_string();
    }
    if day == today - Duration::days(1) {
        return "Yesterday".into();
    }
    if day.year() == today.year() {
        return date.format("%b %-d").to_string();
    }
    date.format("%b %-d, %Y").to_string()
}

/// "9:41 AM" today, "Mar 3, 9:41 AM" this year, "Mar 3, 2024, 9:41 AM" before.
pub fn long<Tz: TimeZone>(ms: i64, now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let date = local(ms, &now.timezone());
    let today = now.date_naive();
    if date.date_naive() == today {
        return date.format("%-I:%M %p").to_string();
    }
    if date.year() == today.year() {
        return date.format("%b %-d, %-I:%M %p").to_string();
    }
    date.format("%b %-d, %Y, %-I:%M %p").to_string()
}

/// How a reply introduces what it quotes: "Mar 3, 2024 at 9:41 AM".
pub fn quoted<Tz: TimeZone>(ms: i64, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    local(ms, zone).format("%b %-d, %Y at %-I:%M %p").to_string()
}

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;

    use super::*;

    fn at(text: &str) -> i64 {
        DateTime::parse_from_rfc3339(text).unwrap().timestamp_millis()
    }

    #[test]
    fn short_dates_follow_how_long_ago() {
        let zone = FixedOffset::east_opt(2 * 3600).unwrap();
        let now = zone.with_ymd_and_hms(2026, 10, 7, 12, 0, 0).unwrap();
        assert_eq!(short(at("2026-10-07T09:41:00+02:00"), &now), "9:41 AM");
        assert_eq!(short(at("2026-10-06T23:59:00+02:00"), &now), "Yesterday");
        // Late on the 5th in UTC is already the 6th here.
        assert_eq!(short(at("2026-10-05T23:30:00Z"), &now), "Yesterday");
        assert_eq!(short(at("2026-03-03T10:00:00+02:00"), &now), "Mar 3");
        assert_eq!(short(at("2024-03-03T10:00:00+02:00"), &now), "Mar 3, 2024");
    }

    #[test]
    fn long_dates_add_the_time() {
        let zone = FixedOffset::east_opt(0).unwrap();
        let now = zone.with_ymd_and_hms(2026, 10, 7, 12, 0, 0).unwrap();
        assert_eq!(long(at("2026-10-07T21:05:00Z"), &now), "9:05 PM");
        assert_eq!(long(at("2026-03-03T09:41:00Z"), &now), "Mar 3, 9:41 AM");
        assert_eq!(long(at("2025-03-03T09:41:00Z"), &now), "Mar 3, 2025, 9:41 AM");
    }
}
