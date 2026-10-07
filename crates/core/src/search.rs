//! A search as typed, with Gmail's operators: the words become an FTS5 match, the rest conditions
//! on the messages.

use chrono::{DateTime, NaiveDate, TimeZone};

#[derive(Debug, Default, PartialEq)]
pub struct Query {
    /// The FTS5 match for the words, when there are any to look for.
    pub text: Option<String>,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    Unread(bool),
    Starred(bool),
    Attachment,
    /// A mailbox, as the lists name them: `inbox`, `sent`, `archive`, ...
    In(String),
    /// A label's name, lowercase, as written: `label:clients` or `label:"big clients"`.
    Label(String),
    /// Unix milliseconds.
    Before(i64),
    After(i64),
    /// An FTS5 match the messages must not have, when no other words come before it.
    Without(String),
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.text.is_none() && self.conditions.is_empty()
    }

    /// Whether trash and spam are searched: only when asked for.
    pub fn includes_trash(&self) -> bool {
        self.conditions.iter().any(|condition| {
            matches!(condition, Condition::In(mailbox) if ["trash", "spam", "anywhere"].contains(&mailbox.as_str()))
        })
    }
}

struct Token {
    negated: bool,
    key: Option<String>,
    value: String,
    quoted: bool,
}

fn tokens(input: &str) -> Vec<Token> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    let read_quoted = |index: &mut usize| -> String {
        *index += 1;
        let start = *index;
        while *index < chars.len() && chars[*index] != '"' {
            *index += 1;
        }
        let text: String = chars[start..*index].iter().collect();
        *index += 1;
        text
    };
    while index < chars.len() {
        if chars[index].is_whitespace() {
            index += 1;
            continue;
        }
        let negated = chars[index] == '-' && chars.get(index + 1).is_some_and(|next| !next.is_whitespace());
        if negated {
            index += 1;
        }
        if chars[index] == '"' {
            tokens.push(Token { negated, key: None, value: read_quoted(&mut index), quoted: true });
            continue;
        }
        let start = index;
        while index < chars.len() && !chars[index].is_whitespace() && chars[index] != ':' {
            index += 1;
        }
        let word: String = chars[start..index].iter().collect();
        if chars.get(index) != Some(&':') {
            tokens.push(Token { negated, key: None, value: word, quoted: false });
            continue;
        }
        index += 1;
        let key = Some(word.to_lowercase());
        if chars.get(index) == Some(&'"') {
            tokens.push(Token { negated, key, value: read_quoted(&mut index), quoted: true });
            continue;
        }
        let start = index;
        while index < chars.len() && !chars[index].is_whitespace() {
            index += 1;
        }
        tokens.push(Token { negated, key, value: chars[start..index].iter().collect(), quoted: false });
    }
    tokens
}

/// One FTS5 phrase: a prefix unless quoted, in one column when given.
fn phrase(value: &str, quoted: bool, column: Option<&str>) -> Option<String> {
    let value = value.replace(['"', '*'], " ");
    if !value.chars().any(char::is_alphanumeric) {
        return None;
    }
    let star = if quoted { "" } else { "*" };
    let column = column.map(|column| format!("{column}:")).unwrap_or_default();
    Some(format!("{column}\"{}\"{star}", value.trim()))
}

fn day<Tz: TimeZone>(text: &str, zone: &Tz) -> Option<i64> {
    let date =
        NaiveDate::parse_from_str(text, "%Y/%m/%d").or_else(|_| NaiveDate::parse_from_str(text, "%Y-%m-%d")).ok()?;
    let start = date.and_hms_opt(0, 0, 0)?;
    Some(zone.from_local_datetime(&start).earliest()?.timestamp_millis())
}

/// `2d`, `3w`, `1m`, `1y`, in milliseconds. An age too long to count is as long as can be.
fn age(text: &str) -> Option<i64> {
    let unit = text.chars().last()?;
    let count: i64 = text[..text.len() - unit.len_utf8()].parse().ok()?;
    let days = match unit {
        'd' => 1,
        'w' => 7,
        'm' => 30,
        'y' => 365,
        _ => return None,
    };
    Some(count.saturating_mul(days).saturating_mul(86_400_000))
}

pub fn parse<Tz: TimeZone>(input: &str, now: &DateTime<Tz>) -> Query {
    let zone = now.timezone();
    let now_ms = now.timestamp_millis();
    let mut groups: Vec<Vec<String>> = Vec::new();
    let mut negatives: Vec<String> = Vec::new();
    let mut conditions = Vec::new();
    let mut or_next = false;
    for token in tokens(input) {
        let Token { negated, key, value, quoted } = token;
        if key.is_none() && !quoted && !negated && value == "OR" {
            or_next = !groups.is_empty();
            continue;
        }
        let column = match key.as_deref() {
            None => None,
            Some("from") => Some("sender"),
            Some("to" | "cc" | "bcc") => Some("recipients"),
            Some("subject") => Some("subject"),
            Some(key) => {
                let value = value.to_lowercase();
                let condition = match (key, value.as_str()) {
                    ("is", "unread") => Some(Condition::Unread(!negated)),
                    ("is", "read") => Some(Condition::Unread(negated)),
                    ("is", "starred") => Some(Condition::Starred(!negated)),
                    ("has", "attachment" | "attachments") if !negated => Some(Condition::Attachment),
                    ("in", _) if !negated => Some(Condition::In(value.clone())),
                    ("label", _) if !negated => Some(Condition::Label(value.clone())),
                    ("before", _) if !negated => day(&value, &zone).map(Condition::Before),
                    ("after", _) if !negated => day(&value, &zone).map(Condition::After),
                    ("older_than", _) if !negated => {
                        age(&value).map(|age| Condition::Before(now_ms.saturating_sub(age)))
                    }
                    ("newer_than", _) if !negated => {
                        age(&value).map(|age| Condition::After(now_ms.saturating_sub(age)))
                    }
                    _ => None,
                };
                if let Some(condition) = condition {
                    conditions.push(condition);
                    continue;
                }
                // Not an operator after all: look for the words as written.
                let Some(phrase) = phrase(&format!("{key} {value}"), quoted, None) else { continue };
                push_phrase(&mut groups, &mut negatives, &mut or_next, phrase, negated);
                continue;
            }
        };
        let Some(phrase) = phrase(&value, quoted, column) else { continue };
        push_phrase(&mut groups, &mut negatives, &mut or_next, phrase, negated);
    }
    let positive: Vec<String> = groups
        .iter()
        .map(|group| match group.len() {
            1 => group[0].clone(),
            _ => format!("({})", group.join(" OR ")),
        })
        .collect();
    let text = match positive.is_empty() {
        true => {
            conditions.extend(negatives.into_iter().map(Condition::Without));
            None
        }
        false => {
            let without: String = negatives.iter().map(|negative| format!(" NOT {negative}")).collect();
            match without.is_empty() {
                true => Some(positive.join(" AND ")),
                false => Some(format!("({}){without}", positive.join(" AND "))),
            }
        }
    };
    Query { text, conditions }
}

fn push_phrase(
    groups: &mut Vec<Vec<String>>,
    negatives: &mut Vec<String>,
    or_next: &mut bool,
    phrase: String,
    negated: bool,
) {
    if negated {
        negatives.push(phrase);
        *or_next = false;
        return;
    }
    match (*or_next, groups.last_mut()) {
        (true, Some(group)) => group.push(phrase),
        _ => groups.push(vec![phrase]),
    }
    *or_next = false;
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, Utc};

    use super::*;

    fn now() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(3600).unwrap().with_ymd_and_hms(2026, 10, 7, 12, 0, 0).unwrap()
    }

    fn text(query: &str) -> Option<String> {
        parse(query, &now()).text
    }

    #[test]
    fn words_are_prefixes_and_quotes_are_exact() {
        assert_eq!(text("lunch fri").as_deref(), Some("\"lunch\"* AND \"fri\"*"));
        assert_eq!(text("\"see you soon\"").as_deref(), Some("\"see you soon\""));
        assert_eq!(text("\"; DROP \"\"x"), Some("\"; DROP\" AND \"x\"".into()));
        assert_eq!(text("--- * \""), None);
    }

    #[test]
    fn or_binds_tighter_than_and() {
        assert_eq!(text("budget q3 OR q4").as_deref(), Some("\"budget\"* AND (\"q3\"* OR \"q4\"*)"));
        assert_eq!(text("OR lunch").as_deref(), Some("\"lunch\"*"));
    }

    #[test]
    fn people_and_subjects_look_in_their_columns() {
        assert_eq!(text("from:ann@x.com").as_deref(), Some("sender:\"ann@x.com\"*"));
        assert_eq!(text("to:bob cc:cy").as_deref(), Some("recipients:\"bob\"* AND recipients:\"cy\"*"));
        assert_eq!(text("subject:\"q4 budget\"").as_deref(), Some("subject:\"q4 budget\""));
    }

    #[test]
    fn negated_words_are_taken_away() {
        assert_eq!(text("lunch -friday").as_deref(), Some("(\"lunch\"*) NOT \"friday\"*"));
        let alone = parse("-friday is:unread", &now());
        assert_eq!(alone.text, None);
        assert_eq!(alone.conditions, [Condition::Unread(true), Condition::Without("\"friday\"*".into())]);
    }

    #[test]
    fn operators_become_conditions() {
        let query = parse("is:unread is:starred has:attachment in:sent label:Clients -is:read", &now());
        assert_eq!(
            query.conditions,
            [
                Condition::Unread(true),
                Condition::Starred(true),
                Condition::Attachment,
                Condition::In("sent".into()),
                Condition::Label("clients".into()),
                Condition::Unread(true),
            ]
        );
        assert_eq!(query.text, None);
        assert!(!query.includes_trash());
        assert!(parse("in:trash", &now()).includes_trash());
    }

    #[test]
    fn dates_are_days_in_the_users_zone_and_ages_count_back() {
        let query = parse("after:2026/01/31 before:2026-02-02 newer_than:2d older_than:1w", &now());
        let midnight = |day: u32, month: u32| {
            FixedOffset::east_opt(3600).unwrap().with_ymd_and_hms(2026, month, day, 0, 0, 0).unwrap().timestamp_millis()
        };
        let now = now().timestamp_millis();
        assert_eq!(
            query.conditions,
            [
                Condition::After(midnight(31, 1)),
                Condition::Before(midnight(2, 2)),
                Condition::After(now - 2 * 86_400_000),
                Condition::Before(now - 7 * 86_400_000),
            ]
        );
    }

    #[test]
    fn ages_too_long_to_count_mean_everything_or_nothing() {
        let query = parse("newer_than:99999999999999999y older_than:9223372036854775807d", &now());
        let oldest = now().timestamp_millis() - i64::MAX;
        assert_eq!(query.conditions, [Condition::After(oldest), Condition::Before(oldest)]);
    }

    #[test]
    fn unknown_operators_are_words() {
        assert_eq!(text("re:lunch").as_deref(), Some("\"re lunch\"*"));
        assert_eq!(parse("before:soon", &Utc::now()).text.as_deref(), Some("\"before soon\"*"));
    }
}
