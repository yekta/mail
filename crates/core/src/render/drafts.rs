//! A reply, reply-all or forward, filled in from the message it answers.

use std::collections::HashSet;

use chrono::TimeZone;
use mail_protocol::{Address, Draft, Message};
use serde::{Deserialize, Serialize};

use super::{dates, text};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyKind {
    Reply,
    ReplyAll,
    Forward,
}

fn prefixed(subject: &str, prefix: &str) -> String {
    let lower = subject.trim().to_lowercase();
    let already = match prefix {
        "Re:" => lower.starts_with("re:"),
        _ => lower.starts_with("fwd:") || lower.starts_with("fw:"),
    };
    match already {
        true => subject.trim().to_string(),
        false => format!("{prefix} {}", subject.trim()),
    }
}

/// Addresses as a header shows them: `Ann <ann@x.com>, bob@x.com`.
pub fn line(addresses: &[Address]) -> String {
    addresses
        .iter()
        .map(|address| match &address.name {
            Some(name) => format!("{name} <{}>", address.email),
            None => address.email.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `body` is the text of the message answered.
pub fn reply<Tz: TimeZone>(message: &Message, body: &str, kind: ReplyKind, me: &HashSet<String>, zone: &Tz) -> Draft
where
    Tz::Offset: std::fmt::Display,
{
    let from_me = me.contains(&message.from.email);
    let sender = match message.recipients.reply_to.is_empty() {
        true => vec![message.from.clone()],
        false => message.recipients.reply_to.clone(),
    };
    let not_me = |address: &&Address| !me.contains(&address.email);
    let (to, cc) = match (kind, from_me) {
        (ReplyKind::Forward, _) => (Vec::new(), Vec::new()),
        // Answering your own message goes to whoever it went to.
        (ReplyKind::Reply, true) => (message.recipients.to.clone(), Vec::new()),
        (ReplyKind::Reply, false) => (sender, Vec::new()),
        (ReplyKind::ReplyAll, true) => (message.recipients.to.clone(), message.recipients.cc.clone()),
        (ReplyKind::ReplyAll, false) => {
            let mut to = sender;
            to.extend(message.recipients.to.iter().filter(not_me).cloned());
            (to, message.recipients.cc.iter().filter(not_me).cloned().collect())
        }
    };
    let mut seen = HashSet::new();
    let to: Vec<Address> = to.into_iter().filter(|address| seen.insert(address.email.clone())).collect();
    let cc: Vec<Address> = cc.into_iter().filter(|address| seen.insert(address.email.clone())).collect();

    let when = dates::quoted(message.date, zone);
    let (subject, quote) = match kind {
        ReplyKind::Forward => {
            let header = format!(
                "---------- Forwarded message ----------\nFrom: {}\nDate: {when}\nSubject: {}\nTo: {}\n\n",
                line(std::slice::from_ref(&message.from)),
                message.subject,
                line(&message.recipients.to)
            );
            (prefixed(&message.subject, "Fwd:"), format!("{header}{body}"))
        }
        _ => {
            let quoted: Vec<String> = body.lines().map(|line| format!("> {line}").trim_end().to_string()).collect();
            (
                prefixed(&message.subject, "Re:"),
                format!("On {when}, {} wrote:\n{}", message.from.display(), quoted.join("\n")),
            )
        }
    };
    let mut references = message.references.clone();
    references.extend(message.message_id.clone());
    Draft {
        account_id: message.account_id.clone(),
        to,
        cc,
        bcc: Vec::new(),
        subject,
        text: String::new(),
        quote: Some(quote),
        in_reply_to: match kind {
            ReplyKind::Forward => None,
            _ => message.message_id.clone(),
        },
        references: match kind {
            ReplyKind::Forward => Vec::new(),
            _ => references,
        },
        thread_id: match kind {
            ReplyKind::Forward => None,
            _ => Some(message.thread_id.clone()),
        },
        forward_attachments_of: (kind == ReplyKind::Forward && !message.attachments.is_empty())
            .then(|| message.id.clone()),
        ..Default::default()
    }
}

/// The text of a body, for quoting.
pub fn body_text(body: &mail_protocol::Body) -> String {
    match (&body.text, &body.html) {
        (Some(text), _) => text.clone(),
        (None, Some(html)) => text::from_html(html),
        (None, None) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use mail_protocol::Recipients;

    use super::*;

    fn message() -> Message {
        Message {
            id: "m".into(),
            account_id: "a".into(),
            thread_id: "t".into(),
            from: Address::new(Some("Ann Lee"), "ann@x.com"),
            recipients: Recipients {
                to: vec![Address::new(None, "me@example.com"), Address::new(Some("Bob"), "bob@x.com")],
                cc: vec![Address::new(None, "cy@x.com")],
                ..Default::default()
            },
            subject: "Lunch".into(),
            snippet: String::new(),
            date: 1_700_000_000_000,
            unread: false,
            starred: false,
            labels: vec![],
            attachments: vec![],
            message_id: Some("<one@x>".into()),
            in_reply_to: None,
            references: vec!["<zero@x>".into()],
            snoozed_until: None,
            bulk: false,
            unsubscribe: None,
            deleted: false,
            rev: 1,
        }
    }

    fn me() -> HashSet<String> {
        HashSet::from(["me@example.com".to_string()])
    }

    #[test]
    fn reply_goes_to_the_sender_and_quotes() {
        let draft = reply(&message(), "Friday?\nAt noon", ReplyKind::Reply, &me(), &Utc);
        assert_eq!(draft.to, vec![Address::new(Some("Ann Lee"), "ann@x.com")]);
        assert!(draft.cc.is_empty());
        assert_eq!(draft.subject, "Re: Lunch");
        assert!(draft.text.is_empty());
        assert!(draft.quote.as_deref().unwrap().ends_with("Ann Lee wrote:\n> Friday?\n> At noon"), "{:?}", draft.quote);
        assert_eq!(draft.in_reply_to.as_deref(), Some("<one@x>"));
        assert_eq!(draft.references, ["<zero@x>", "<one@x>"]);
        assert_eq!(draft.thread_id.as_deref(), Some("t"));
    }

    #[test]
    fn reply_all_leaves_me_out() {
        let draft = reply(&message(), "", ReplyKind::ReplyAll, &me(), &Utc);
        let to: Vec<&str> = draft.to.iter().map(|address| address.email.as_str()).collect();
        assert_eq!(to, ["ann@x.com", "bob@x.com"]);
        assert_eq!(draft.cc, vec![Address::new(None, "cy@x.com")]);
    }

    #[test]
    fn forward_starts_empty_and_keeps_the_subject_once() {
        let mut forwarded = message();
        forwarded.subject = "Fwd: Lunch".into();
        let draft = reply(&forwarded, "Body", ReplyKind::Forward, &me(), &Utc);
        assert!(draft.to.is_empty());
        assert_eq!(draft.subject, "Fwd: Lunch");
        let quote = draft.quote.unwrap();
        assert!(quote.starts_with("---------- Forwarded message") && quote.ends_with("Body"), "{quote}");
        assert_eq!(draft.in_reply_to, None);
        assert_eq!(draft.forward_attachments_of, None, "nothing to bring along");
        forwarded.attachments =
            vec![mail_protocol::Attachment { name: "a.pdf".into(), mime: "application/pdf".into(), size: 1 }];
        let draft = reply(&forwarded, "Body", ReplyKind::Forward, &me(), &Utc);
        assert_eq!(draft.forward_attachments_of.as_deref(), Some("m"));
    }
}
