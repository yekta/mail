//! The times a snooze, a reminder or a send-later can be for: the usual choices, and what a person
//! types ("tomorrow 9am", "in 2 hours", "fri", "next mon 2pm", "dec 3"), in the user's time zone.

use chrono::{DateTime, Datelike, Days, Duration, Months, NaiveDate, TimeZone, Timelike, Weekday};

use crate::api::TimeChoice;

/// When a day is given without a time.
const MORNING: u32 = 8;
const EVENING: u32 = 18;

fn at<Tz: TimeZone>(zone: &Tz, date: NaiveDate, hour: u32, minute: u32) -> Option<DateTime<Tz>> {
    let local = date.and_hms_opt(hour, minute, 0)?;
    // A time that a clock change skips is taken an hour later.
    zone.from_local_datetime(&local)
        .earliest()
        .or_else(|| zone.from_local_datetime(&(local + Duration::hours(1))).earliest())
}

/// "Wed 8:00 AM" within a week, "Tue, Oct 20, 8:00 AM" this year, "Jan 5, 2027, 8:00 AM" after.
pub fn label<Tz: TimeZone>(when: &DateTime<Tz>, now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let days = (when.date_naive() - now.date_naive()).num_days();
    if (0..7).contains(&days) {
        return when.format("%a %-I:%M %p").to_string();
    }
    if when.year() == now.year() {
        return when.format("%a, %b %-d, %-I:%M %p").to_string();
    }
    when.format("%b %-d, %Y, %-I:%M %p").to_string()
}

fn choice<Tz: TimeZone>(id: &str, name: &str, when: &DateTime<Tz>, now: &DateTime<Tz>) -> TimeChoice
where
    Tz::Offset: std::fmt::Display,
{
    TimeChoice { id: id.into(), name: name.into(), label: label(when, now), until: when.timestamp_millis() }
}

fn next_monday(today: NaiveDate) -> NaiveDate {
    today + Days::new(7 - today.weekday().num_days_from_monday() as u64)
}

/// The first `weekday` after today.
fn coming(today: NaiveDate, weekday: Weekday) -> NaiveDate {
    let ahead = (weekday.num_days_from_monday() + 7 - today.weekday().num_days_from_monday()) % 7;
    today + Days::new(if ahead == 0 { 7 } else { ahead as u64 })
}

/// The usual choices, those that make sense now.
pub fn choices<Tz: TimeZone>(now: &DateTime<Tz>) -> Vec<TimeChoice>
where
    Tz::Offset: std::fmt::Display,
{
    let zone = now.timezone();
    let today = now.date_naive();
    let mut list = Vec::new();
    let later = now.clone() + Duration::hours(3);
    if later.date_naive() == today
        && later.hour() < 21
        && let Some(later) = at(&zone, today, later.hour(), 0)
    {
        list.push(choice("later_today", "Later today", &later, now));
    }
    if now.hour() + 1 < EVENING
        && let Some(evening) = at(&zone, today, EVENING, 0)
    {
        list.push(choice("this_evening", "This evening", &evening, now));
    }
    let days = [
        ("tomorrow", "Tomorrow", Some(today + Days::new(1))),
        (
            "this_weekend",
            "This weekend",
            (today.weekday().num_days_from_monday() < 5).then(|| coming(today, Weekday::Sat)),
        ),
        ("next_week", "Next week", Some(next_monday(today))),
        ("next_month", "Next month", today.checked_add_months(Months::new(1))),
    ];
    for (id, name, date) in days {
        let Some(when) = date.and_then(|date| at(&zone, date, MORNING, 0)) else { continue };
        list.push(choice(id, name, &when, now));
    }
    list
}

/// The usual choices whose name starts like `text`, then what `text` says when it says a time.
pub fn parse<Tz: TimeZone>(text: &str, now: &DateTime<Tz>) -> Vec<TimeChoice>
where
    Tz::Offset: std::fmt::Display,
{
    let text = normalize(text);
    if text.is_empty() {
        return choices(now);
    }
    let mut found: Vec<TimeChoice> =
        choices(now).into_iter().filter(|choice| choice.name.to_lowercase().starts_with(&text)).collect();
    if let Some(when) = when(&text, now)
        && !found.iter().any(|choice| choice.until == when.timestamp_millis())
    {
        let mut letters = text.chars();
        let name: String =
            letters.next().map(|first| first.to_uppercase().chain(letters).collect()).unwrap_or_default();
        found.push(choice("custom", &name, &when, now));
    }
    found
}

fn normalize(text: &str) -> String {
    let text = text.to_lowercase().replace("a.m.", "am").replace("p.m.", "pm").replace(',', " ");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn weekday(word: &str) -> Option<Weekday> {
    Some(match word {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tues" | "tuesday" => Weekday::Tue,
        "wed" | "weds" | "wednesday" => Weekday::Wed,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

fn month(word: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let index = MONTHS.iter().position(|month| word.starts_with(month))?;
    let full = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    (full[index].starts_with(word) || word == "sept").then_some(index as u32 + 1)
}

/// "3", "3rd", "21st".
fn day_number(word: &str) -> Option<u32> {
    let digits = word.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &word[digits.len()..];
    if !["", "st", "nd", "rd", "th"].contains(&suffix) {
        return None;
    }
    digits.parse().ok().filter(|day| (1..=31).contains(day))
}

/// An hour said without am or pm: the one in waking hours.
fn waking(hour: u32) -> u32 {
    match hour {
        1..=6 => hour + 12,
        _ => hour,
    }
}

/// "9am", "9:30pm", "21:00", "9:30"; `meridiem` is a following "am" or "pm".
fn clock(word: &str, meridiem: Option<&str>) -> Option<(u32, u32)> {
    let (digits, suffix) = match word.find(|c: char| c.is_ascii_alphabetic()) {
        Some(index) => (&word[..index], Some(&word[index..])),
        None => (word, meridiem),
    };
    let (hour, minute) = match digits.split_once(':') {
        Some((hour, minute)) if minute.len() == 2 => (hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?),
        Some(_) => return None,
        None => (digits.parse::<u32>().ok()?, 0),
    };
    if minute > 59 || digits.is_empty() {
        return None;
    }
    let hour = match suffix {
        Some("am" | "a") if (1..=12).contains(&hour) => hour % 12,
        Some("pm" | "p") if (1..=12).contains(&hour) => hour % 12 + 12,
        Some(_) => return None,
        None if digits.contains(':') && hour <= 23 => match hour {
            1..=6 => waking(hour),
            _ => hour,
        },
        None if (1..=12).contains(&hour) => waking(hour),
        None => return None,
    };
    Some((hour, minute))
}

/// "12/3", "12/3/26", "2026-12-03": month first, unless the first number can't be a month.
fn numeric_date(word: &str, today: NaiveDate) -> Option<NaiveDate> {
    if let Ok(date) = NaiveDate::parse_from_str(word, "%Y-%m-%d") {
        return Some(date);
    }
    let parts: Vec<u32> = word.split('/').map(|part| part.parse().ok()).collect::<Option<_>>()?;
    let (first, second, year) = match parts.as_slice() {
        [first, second] => (*first, *second, None),
        [first, second, year] => (*first, *second, Some(if *year < 100 { year + 2000 } else { *year })),
        _ => return None,
    };
    let (month, day) = if first > 12 { (second, first) } else { (first, second) };
    dated(today, month, day, year)
}

/// A month and day, this year or, once it has passed, the next.
fn dated(today: NaiveDate, month: u32, day: u32, year: Option<u32>) -> Option<NaiveDate> {
    if let Some(year) = year {
        return NaiveDate::from_ymd_opt(year as i32, month, day);
    }
    let this_year = NaiveDate::from_ymd_opt(today.year(), month, day)?;
    match this_year < today {
        true => NaiveDate::from_ymd_opt(today.year() + 1, month, day),
        false => Some(this_year),
    }
}

enum Unit {
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
    Years,
}

fn unit(word: &str) -> Option<Unit> {
    Some(match word {
        "m" | "min" | "mins" | "minute" | "minutes" => Unit::Minutes,
        "h" | "hr" | "hrs" | "hour" | "hours" => Unit::Hours,
        "d" | "day" | "days" => Unit::Days,
        "w" | "wk" | "wks" | "week" | "weeks" => Unit::Weeks,
        "mo" | "mos" | "month" | "months" => Unit::Months,
        "y" | "yr" | "yrs" | "year" | "years" => Unit::Years,
        _ => return None,
    })
}

/// "3", "a" or "an" before a unit.
fn count(word: &str) -> Option<f64> {
    match word {
        "a" | "an" => Some(1.0),
        _ => word.parse().ok().filter(|count: &f64| *count > 0.0 && *count < 10_000.0),
    }
}

/// "3d", "2h", "1.5hours": a count with its unit in one word.
fn joined(word: &str) -> Option<(f64, Unit)> {
    let split = word.find(|c: char| c.is_ascii_alphabetic())?;
    Some((count(&word[..split])?, unit(&word[split..])?))
}

/// What the text says, if it is a time in the future.
fn when<Tz: TimeZone>(text: &str, now: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let zone = now.timezone();
    let today = now.date_naive();
    let words: Vec<&str> = text.split(' ').collect();
    let mut date: Option<NaiveDate> = None;
    let mut time: Option<(u32, u32)> = None;
    let mut instant: Option<DateTime<Tz>> = None;
    let mut index = 0;
    while index < words.len() {
        let word = words[index];
        let next = words.get(index + 1).copied();
        index += 1;
        let relative = match (joined(word), next.and_then(unit)) {
            (Some(found), _) => Some(found),
            (None, Some(unit)) => count(word).map(|count| {
                index += 1;
                (count, unit)
            }),
            _ => None,
        };
        if let Some((count, unit)) = relative {
            let whole = count.round() as u32;
            match unit {
                Unit::Minutes => instant = Some(now.clone() + Duration::seconds((count * 60.0) as i64)),
                Unit::Hours => instant = Some(now.clone() + Duration::seconds((count * 3600.0) as i64)),
                Unit::Days => date = Some(today + Days::new(whole as u64)),
                Unit::Weeks => date = Some(today + Days::new(whole as u64 * 7)),
                Unit::Months => date = today.checked_add_months(Months::new(whole)),
                Unit::Years => date = today.checked_add_months(Months::new(whole * 12)),
            }
            continue;
        }
        match word {
            "in" | "at" | "on" | "the" | "by" | "this" | "of" => {}
            "today" => date = Some(today),
            "tonight" => {
                date = Some(today);
                time = Some((20, 0));
            }
            "tomorrow" | "tmrw" | "tmr" | "tom" => date = Some(today + Days::new(1)),
            "morning" => time = Some((MORNING, 0)),
            "noon" | "midday" | "lunch" => time = Some((12, 0)),
            "afternoon" => time = Some((14, 0)),
            "evening" => time = Some((EVENING, 0)),
            "midnight" => time = Some((0, 0)),
            "weekend" => date = Some(coming(today, Weekday::Sat)),
            "next" => {
                let following = next?;
                index += 1;
                date = Some(match following {
                    "week" => next_monday(today),
                    "month" => today.checked_add_months(Months::new(1))?,
                    "weekend" => next_monday(today) + Days::new(5),
                    _ => next_monday(today) + Days::new(weekday(following)?.num_days_from_monday() as u64),
                });
            }
            "am" | "pm" => {}
            _ => {
                if let Some(day) = weekday(word) {
                    date = Some(coming(today, day));
                } else if let Some(month) = month(word) {
                    let day = next.and_then(day_number)?;
                    index += 1;
                    let year = words.get(index).and_then(|year| year.parse::<u32>().ok()).filter(|year| *year > 1900);
                    if year.is_some() {
                        index += 1;
                    }
                    date = Some(dated(today, month, day, year)?);
                } else if let Some(day) = day_number(word)
                    && let Some(month) = next.and_then(month)
                {
                    index += 1;
                    date = Some(dated(today, month, day, None)?);
                } else if let Some(found) = numeric_date(word, today) {
                    date = Some(found);
                } else {
                    let meridiem = next.filter(|next| *next == "am" || *next == "pm");
                    time = Some(clock(word, meridiem)?);
                }
            }
        }
    }
    let found = match (instant, date, time) {
        (Some(instant), None, None) => instant,
        (Some(_), ..) => return None,
        (None, None, None) => return None,
        (None, Some(date), None) if date == today => return None,
        (None, Some(date), None) => at(&zone, date, MORNING, 0)?,
        (None, None, Some((hour, minute))) => {
            let today_then = at(&zone, today, hour, minute)?;
            match today_then > *now {
                true => today_then,
                false => at(&zone, today + Days::new(1), hour, minute)?,
            }
        }
        (None, Some(date), Some((hour, minute))) => at(&zone, date, hour, minute)?,
    };
    (found > *now).then_some(found)
}

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;

    use super::*;

    /// Wednesday, October 7 2026, 12:00 two hours east of UTC.
    fn now() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(2 * 3600).unwrap().with_ymd_and_hms(2026, 10, 7, 12, 0, 0).unwrap()
    }

    fn local(text: &str) -> String {
        let choices = parse(text, &now());
        let Some(last) = choices.last() else { return "nothing".into() };
        let when = now().timezone().timestamp_millis_opt(last.until).unwrap();
        when.format("%a %Y-%m-%d %H:%M").to_string()
    }

    #[test]
    fn the_usual_choices_follow_the_time_of_day() {
        let names: Vec<String> = choices(&now()).into_iter().map(|choice| choice.name).collect();
        assert_eq!(names, ["Later today", "This evening", "Tomorrow", "This weekend", "Next week", "Next month"]);
        let list = choices(&now());
        assert_eq!(list[0].label, "Wed 3:00 PM");
        assert_eq!(list[2].label, "Thu 8:00 AM");
        assert_eq!(list[3].label, "Sat 8:00 AM");
        assert_eq!(list[4].label, "Mon 8:00 AM");
        assert_eq!(list[5].label, "Sat, Nov 7, 8:00 AM");

        let late = now().with_hour(22).unwrap();
        let names: Vec<String> = choices(&late).into_iter().map(|choice| choice.name).collect();
        assert_eq!(names, ["Tomorrow", "This weekend", "Next week", "Next month"]);
        let saturday = now() + Duration::days(3);
        assert!(choices(&saturday).iter().all(|choice| choice.id != "this_weekend"));
    }

    #[test]
    fn relative_times_count_from_now() {
        assert_eq!(local("in 2 hours"), "Wed 2026-10-07 14:00");
        assert_eq!(local("2h"), "Wed 2026-10-07 14:00");
        assert_eq!(local("30 min"), "Wed 2026-10-07 12:30");
        assert_eq!(local("in an hour"), "Wed 2026-10-07 13:00");
        assert_eq!(local("1.5h"), "Wed 2026-10-07 13:30");
        assert_eq!(local("3d"), "Sat 2026-10-10 08:00");
        assert_eq!(local("in 2 weeks"), "Wed 2026-10-21 08:00");
        assert_eq!(local("in 3 days at 5pm"), "Sat 2026-10-10 17:00");
        assert_eq!(local("1 month"), "Sat 2026-11-07 08:00");
    }

    #[test]
    fn days_and_times() {
        assert_eq!(local("tomorrow"), "Thu 2026-10-08 08:00");
        assert_eq!(local("tomorrow 9am"), "Thu 2026-10-08 09:00");
        assert_eq!(local("Tomorrow at 9:30 p.m."), "Thu 2026-10-08 21:30");
        assert_eq!(local("fri"), "Fri 2026-10-09 08:00");
        assert_eq!(local("wed"), "Wed 2026-10-14 08:00");
        assert_eq!(local("next mon 2pm"), "Mon 2026-10-12 14:00");
        assert_eq!(local("next fri"), "Fri 2026-10-16 08:00");
        assert_eq!(local("next week"), "Mon 2026-10-12 08:00");
        assert_eq!(local("weekend"), "Sat 2026-10-10 08:00");
        assert_eq!(local("monday morning"), "Mon 2026-10-12 08:00");
        assert_eq!(local("thursday afternoon"), "Thu 2026-10-08 14:00");
    }

    #[test]
    fn times_alone_are_the_next_one_to_come() {
        assert_eq!(local("noon"), "Thu 2026-10-08 12:00");
        assert_eq!(local("tonight"), "Wed 2026-10-07 20:00");
        assert_eq!(local("5"), "Wed 2026-10-07 17:00");
        assert_eq!(local("at 9"), "Thu 2026-10-08 09:00");
        assert_eq!(local("3:15"), "Wed 2026-10-07 15:15");
        assert_eq!(local("21:00"), "Wed 2026-10-07 21:00");
        assert_eq!(local("7 pm"), "Wed 2026-10-07 19:00");
        assert_eq!(local("midnight"), "Thu 2026-10-08 00:00");
        assert_eq!(local("this evening"), "Wed 2026-10-07 18:00");
    }

    #[test]
    fn dates_are_this_year_until_they_pass() {
        assert_eq!(local("dec 3"), "Thu 2026-12-03 08:00");
        assert_eq!(local("December 3rd at 4pm"), "Thu 2026-12-03 16:00");
        assert_eq!(local("3 dec"), "Thu 2026-12-03 08:00");
        assert_eq!(local("12/3"), "Thu 2026-12-03 08:00");
        assert_eq!(local("25/12"), "Fri 2026-12-25 08:00");
        assert_eq!(local("sep 1"), "Wed 2027-09-01 08:00");
        assert_eq!(local("2027-01-05 10:00"), "Tue 2027-01-05 10:00");
        assert_eq!(local("1/5/27"), "Tue 2027-01-05 08:00");
    }

    #[test]
    fn nonsense_and_the_past_say_nothing() {
        assert_eq!(local("banana"), "nothing");
        assert_eq!(local("today 9am"), "nothing");
        assert_eq!(local("2020-01-01"), "nothing");
        assert_eq!(local("13pm"), "nothing");
        assert_eq!(local("feb 30"), "nothing");
    }

    #[test]
    fn typing_a_choice_finds_it_first() {
        let found = parse("tom", &now());
        assert_eq!(found.len(), 1, "the parsed time is the same as the choice");
        assert_eq!(found[0].name, "Tomorrow");
        let next = parse("next", &now());
        assert_eq!(next.iter().map(|choice| choice.id.as_str()).collect::<Vec<_>>(), ["next_week", "next_month"]);
        assert_eq!(parse("fri 3pm", &now())[0].name, "Fri 3pm");
        assert_eq!(parse("", &now()).len(), 6);
    }
}
