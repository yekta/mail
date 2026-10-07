//! Search as in Gmail: words, "quoted phrases", -word and OR anywhere in a message, and operators
//! for its parts: from:, to:, subject:, has:attachment, is:unread, is:starred,
//! in:<inbox|sent|drafts|trash|spam|archive|snoozed|starred>, label:<name>, before: and after:
//! (YYYY/MM/DD), newer_than: and older_than: (2d, 3w, 1m, 1y). An operator it doesn't know is a word.
//! The search vector weighs the sender A, the recipients B and the subject C (see message_search).

use chrono::{DateTime, Duration, Months, NaiveDate, Utc};
use mail_protocol::role;
use sqlx::{PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

const LIMIT: i64 = 200;

#[derive(Debug, Default, PartialEq)]
pub struct Query {
    /// For websearch_to_tsquery, which reads the phrases, -word and OR.
    pub words: String,
    pub filters: Vec<Filter>,
}

#[derive(Debug, PartialEq)]
pub enum Filter {
    /// A to_tsquery expression, for from:, to: and subject:.
    Matches(String),
    HasAttachment,
    Unread,
    Starred,
    Snoozed,
    /// None of inbox, trash or spam.
    Archived,
    Role(&'static str),
    Label(String),
    Before(DateTime<Utc>),
    After(DateTime<Utc>),
}

pub fn parse(text: &str, now: DateTime<Utc>) -> Query {
    let mut query = Query::default();
    let mut words = Vec::new();
    for token in tokens(text) {
        let Some((key, value)) = token.split_once(':') else {
            words.push(token);
            continue;
        };
        let value = value.trim_matches('"');
        let filter = match key.to_ascii_lowercase().as_str() {
            "from" => lexemes(value, ":*A", " & ").map(Filter::Matches),
            "to" => lexemes(value, ":*B", " & ").map(Filter::Matches),
            "subject" => lexemes(value, ":C", " <-> ").map(Filter::Matches),
            "label" if !value.is_empty() => Some(Filter::Label(value.to_string())),
            "has" | "is" | "in" => mailbox(value),
            "before" => date(value).map(Filter::Before),
            "after" => date(value).map(Filter::After),
            "older_than" => ago(value, now).map(Filter::Before),
            "newer_than" => ago(value, now).map(Filter::After),
            _ => None,
        };
        match filter {
            Some(filter) => query.filters.push(filter),
            None => words.push(token),
        }
    }
    query.words = words.join(" ");
    query
}

pub async fn run(db: &PgPool, user_id: Uuid, text: &str) -> sqlx::Result<Vec<Uuid>> {
    let query = parse(text, Utc::now());
    if query == Query::default() {
        return Ok(Vec::new());
    }
    let mut sql = QueryBuilder::<Postgres>::new("SELECT id FROM messages WHERE NOT deleted AND user_id = ");
    sql.push_bind(user_id);
    if !query.words.is_empty() {
        sql.push(" AND search @@ websearch_to_tsquery('simple', ").push_bind(query.words).push(")");
    }
    for filter in query.filters {
        match filter {
            Filter::Matches(part) => sql.push(" AND search @@ to_tsquery('simple', ").push_bind(part).push(")"),
            Filter::HasAttachment => sql.push(" AND attachments <> '[]'::jsonb"),
            Filter::Unread => sql.push(" AND unread"),
            Filter::Starred => sql.push(" AND starred"),
            Filter::Snoozed => sql.push(" AND snoozed_until IS NOT NULL"),
            Filter::Archived => sql.push(" AND NOT labels && ARRAY['inbox', 'trash', 'spam']"),
            Filter::Role(role) => sql.push(" AND ").push_bind(role).push(" = ANY(labels)"),
            Filter::Label(name) => sql
                .push(" AND labels && ARRAY(SELECT label.id::text FROM labels AS label WHERE label.user_id = ")
                .push_bind(user_id)
                .push(" AND NOT label.deleted AND lower(label.name) = lower(")
                .push_bind(name)
                .push("))"),
            Filter::Before(date) => sql.push(" AND date < ").push_bind(date),
            Filter::After(date) => sql.push(" AND date >= ").push_bind(date),
        };
    }
    sql.push(" ORDER BY date DESC LIMIT ").push_bind(LIMIT);
    sql.build_query_scalar().fetch_all(db).await
}

/// Splits on spaces outside quotes; quotes stay, for websearch_to_tsquery's phrases.
fn tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in text.chars() {
        if character == '"' {
            quoted = !quoted;
        }
        if character.is_whitespace() && !quoted {
            tokens.extend((!current.is_empty()).then(|| std::mem::take(&mut current)));
            continue;
        }
        current.push(character);
    }
    tokens.extend((!current.is_empty()).then_some(current));
    tokens
}

/// The words of `value` as to_tsquery lexemes, each with `suffix` (a weight, maybe a prefix
/// match), joined by `joiner`. Addresses stay whole.
fn lexemes(value: &str, suffix: &str, joiner: &str) -> Option<String> {
    let words: Vec<String> = value
        .split(|character: char| !(character.is_alphanumeric() || "@._+-".contains(character)))
        .map(|word| word.trim_matches(['.', '-', '+', '_']).to_lowercase())
        .filter(|word| !word.is_empty())
        .map(|word| format!("'{word}'{suffix}"))
        .collect();
    (!words.is_empty()).then(|| words.join(joiner))
}

fn mailbox(value: &str) -> Option<Filter> {
    Some(match value.to_ascii_lowercase().as_str() {
        "attachment" => Filter::HasAttachment,
        "unread" => Filter::Unread,
        "starred" => Filter::Starred,
        "snoozed" => Filter::Snoozed,
        "archive" | "archived" => Filter::Archived,
        "inbox" => Filter::Role(role::INBOX),
        "sent" => Filter::Role(role::SENT),
        "drafts" | "draft" => Filter::Role(role::DRAFTS),
        "trash" => Filter::Role(role::TRASH),
        "spam" => Filter::Role(role::SPAM),
        _ => return None,
    })
}

fn date(value: &str) -> Option<DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(value, "%Y/%m/%d").or_else(|_| NaiveDate::parse_from_str(value, "%Y-%m-%d"));
    Some(date.ok()?.and_hms_opt(0, 0, 0)?.and_utc())
}

fn ago(value: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let unit = value.chars().last()?;
    let count: u32 = value[..value.len() - unit.len_utf8()].parse().ok()?;
    match unit.to_ascii_lowercase() {
        'd' => now.checked_sub_signed(Duration::days(count.into())),
        'w' => now.checked_sub_signed(Duration::weeks(count.into())),
        'm' => now.checked_sub_months(Months::new(count)),
        'y' => now.checked_sub_months(Months::new(count.checked_mul(12)?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        date("2026/03/31").unwrap()
    }

    #[test]
    fn keeps_words_phrases_negation_and_or_for_postgres() {
        let query = parse(r#"quarterly "board meeting" -draft budget OR plan"#, now());
        assert_eq!(query.words, r#"quarterly "board meeting" -draft budget OR plan"#);
        assert!(query.filters.is_empty());
    }

    #[test]
    fn reads_the_parts_of_a_message() {
        let query = parse(r#"from:Ann@Example.com to:"bob lee" subject:"weekly report" lunch"#, now());
        assert_eq!(query.words, "lunch");
        let matches = ["'ann@example.com':*A", "'bob':*B & 'lee':*B", "'weekly':C <-> 'report':C"];
        assert_eq!(query.filters, matches.map(|part| Filter::Matches(part.into())));
    }

    #[test]
    fn reads_filters() {
        let query = parse("has:attachment is:unread is:starred in:sent in:archive in:snoozed label:Work", now());
        assert_eq!(
            query.filters,
            [
                Filter::HasAttachment,
                Filter::Unread,
                Filter::Starred,
                Filter::Role("sent"),
                Filter::Archived,
                Filter::Snoozed,
                Filter::Label("Work".into())
            ]
        );
        assert!(query.words.is_empty());
    }

    #[test]
    fn reads_dates_and_ages() {
        let query = parse("after:2026/01/02 before:2026-02-01 newer_than:2d older_than:1m", now());
        assert_eq!(
            query.filters,
            [
                Filter::After(date("2026/01/02").unwrap()),
                Filter::Before(date("2026/02/01").unwrap()),
                Filter::After(date("2026/03/29").unwrap()),
                Filter::Before(date("2026/02/28").unwrap()),
            ]
        );
    }

    #[test]
    fn treats_what_it_does_not_know_as_words() {
        let query = parse("in:nowhere after:yesterday https://example.com from: older_than:2é", now());
        assert_eq!(query.words, "in:nowhere after:yesterday https://example.com from: older_than:2é");
        assert!(query.filters.is_empty());
    }
}
