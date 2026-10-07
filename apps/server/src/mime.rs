//! Raw MIME in and out: what a body and its attachments are, the headers of a message as a
//! provider gives them, and the message a draft becomes.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use mail_builder::MessageBuilder;
use mail_builder::headers::address::Address as BuilderAddress;
use mail_parser::{MessageParser, MimeHeaders, PartType};
use mail_protocol::{Address, Attachment, Body, Draft, Recipients};

/// Inline images larger than this together stay out of the body.
const INLINE_LIMIT: usize = 2_000_000;

pub struct Parsed {
    pub body: Body,
    pub attachments: Vec<Attachment>,
    /// The body as plain words, for search.
    pub plain: String,
}

pub fn parse(raw: &[u8]) -> Option<Parsed> {
    let message = MessageParser::default().parse(raw)?;
    let html = message.html_bodies().find_map(|part| match &part.body {
        PartType::Html(html) => Some(html.to_string()),
        _ => None,
    });
    let text = message.text_bodies().find_map(|part| match &part.body {
        PartType::Text(text) => Some(text.to_string()),
        _ => None,
    });

    let mut inline_budget = INLINE_LIMIT;
    let mut html = html;
    let mut attachments = Vec::new();
    for part in message.attachments() {
        let content_id = part.content_id().map(|id| id.trim_matches(['<', '>']).to_string());
        let mime = part
            .content_type()
            .map(|kind| format!("{}/{}", kind.ctype(), kind.subtype().unwrap_or("octet-stream")))
            .unwrap_or_else(|| "application/octet-stream".into());
        if let (Some(id), Some(page)) = (&content_id, html.as_mut())
            && page.contains(&format!("cid:{id}"))
        {
            let contents = part.contents();
            if contents.len() <= inline_budget {
                inline_budget -= contents.len();
                let data = format!("data:{mime};base64,{}", STANDARD.encode(contents));
                *page = page.replace(&format!("cid:{id}"), &data);
            }
            continue;
        }
        let name = part.attachment_name().unwrap_or("Attachment").to_string();
        attachments.push(Attachment { name, mime, size: part.contents().len() as i64 });
    }

    let plain = message.body_text(0).map(|text| text.to_string()).unwrap_or_default();
    Some(Parsed { body: Body { html, text }, attachments, plain })
}

/// Headers as a provider lists them, read as one message would be.
pub struct Headers {
    pub from: Address,
    pub recipients: Recipients,
    pub subject: String,
    pub date: Option<i64>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub multipart_mixed: bool,
}

pub fn headers(lines: &[(String, String)]) -> Headers {
    let mut raw = String::new();
    for (name, value) in lines {
        raw.push_str(&format!("{name}: {value}\r\n"));
    }
    raw.push_str("\r\n");
    let parsed = MessageParser::default().parse(raw.as_bytes());
    let Some(message) = parsed else {
        return Headers {
            from: Address::default(),
            recipients: Recipients::default(),
            subject: String::new(),
            date: None,
            message_id: None,
            in_reply_to: None,
            references: Vec::new(),
            multipart_mixed: false,
        };
    };
    let list = |address: Option<&mail_parser::Address>| -> Vec<Address> {
        address
            .map(|address| address.iter().filter_map(|addr| Some(Address::new(addr.name(), addr.address()?))).collect())
            .unwrap_or_default()
    };
    let ids = |value: &mail_parser::HeaderValue| -> Vec<String> {
        match value {
            mail_parser::HeaderValue::Text(text) => vec![text.to_string()],
            mail_parser::HeaderValue::TextList(list) => list.iter().map(|id| id.to_string()).collect(),
            _ => Vec::new(),
        }
    };
    let content_type = lines
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.to_ascii_lowercase())
        .unwrap_or_default();
    Headers {
        from: list(message.from()).into_iter().next().unwrap_or_default(),
        recipients: Recipients {
            to: list(message.to()),
            cc: list(message.cc()),
            bcc: list(message.bcc()),
            reply_to: list(message.reply_to()),
        },
        subject: message.subject().unwrap_or_default().to_string(),
        date: message.date().map(|date| date.to_timestamp() * 1000),
        message_id: message.message_id().map(String::from),
        in_reply_to: ids(message.in_reply_to()).into_iter().next(),
        references: ids(message.references()),
        multipart_mixed: content_type.starts_with("multipart/mixed"),
    }
}

fn builder_list(addresses: &[Address]) -> BuilderAddress<'_> {
    BuilderAddress::new_list(
        addresses
            .iter()
            .map(|address| BuilderAddress::new_address(address.name.as_deref(), address.email.as_str()))
            .collect(),
    )
}

/// The message a draft becomes. `with_bcc` keeps the Bcc header, for a provider that reads the
/// recipients from it and strips it (Gmail); otherwise they go in the envelope.
pub fn build(draft: &Draft, from: &Address, message_id: &str, with_bcc: bool) -> Vec<u8> {
    let mut builder = MessageBuilder::new()
        .from(BuilderAddress::new_address(from.name.as_deref(), from.email.as_str()))
        .subject(draft.subject.as_str())
        .message_id(message_id)
        .text_body(draft.text.as_str());
    if !draft.to.is_empty() {
        builder = builder.to(builder_list(&draft.to));
    }
    if !draft.cc.is_empty() {
        builder = builder.cc(builder_list(&draft.cc));
    }
    if with_bcc && !draft.bcc.is_empty() {
        builder = builder.bcc(builder_list(&draft.bcc));
    }
    if let Some(in_reply_to) = &draft.in_reply_to {
        builder = builder.in_reply_to(in_reply_to.trim_matches(['<', '>']));
    }
    if !draft.references.is_empty() {
        let references: Vec<&str> = draft.references.iter().map(|id| id.trim_matches(['<', '>'])).collect();
        builder = builder.references(references);
    }
    builder.write_to_vec().unwrap_or_default()
}

pub fn new_message_id(domain: &str) -> String {
    format!("{}@{}", hex::encode(crate::random_bytes::<12>()), domain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_html_text_and_attachments_and_inlines_cid_images() {
        let raw = "From: Ann <ann@example.com>\r\nTo: bob@example.com\r\nSubject: Hi\r\n\
            Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
            --b\r\nContent-Type: text/html\r\n\r\n<p>Hello <img src=\"cid:logo\"></p>\r\n\
            --b\r\nContent-Type: image/png\r\nContent-ID: <logo>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw==\r\n\
            --b\r\nContent-Type: application/pdf; name=\"plan.pdf\"\r\nContent-Disposition: attachment; filename=\"plan.pdf\"\r\n\r\n%PDF\r\n\
            --b--\r\n";
        let parsed = parse(raw.as_bytes()).unwrap();
        let html = parsed.body.html.unwrap();
        assert!(html.contains("data:image/png;base64,"), "{html}");
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].name, "plan.pdf");
        assert!(parsed.plain.contains("Hello"));
    }

    #[test]
    fn reads_listed_headers() {
        let headers = headers(&[
            ("From".into(), "\"Ann Lee\" <Ann@Example.com>".into()),
            ("To".into(), "bob@example.com, Cy <cy@example.com>".into()),
            ("Subject".into(), "=?utf-8?q?Caf=C3=A9?=".into()),
            ("References".into(), "<a@x> <b@x>".into()),
            ("Content-Type".into(), "multipart/mixed; boundary=x".into()),
        ]);
        assert_eq!(headers.from, Address { name: Some("Ann Lee".into()), email: "ann@example.com".into() });
        assert_eq!(headers.recipients.to.len(), 2);
        assert_eq!(headers.subject, "Café");
        assert_eq!(headers.references, ["a@x", "b@x"]);
        assert!(headers.multipart_mixed);
    }

    #[test]
    fn builds_a_reply_without_bcc_unless_asked() {
        let draft = Draft {
            account_id: "a".into(),
            to: vec![Address::new(None, "bob@example.com")],
            bcc: vec![Address::new(None, "secret@example.com")],
            subject: "Re: Hi".into(),
            text: "Sure.".into(),
            in_reply_to: Some("<one@x>".into()),
            references: vec!["<one@x>".into()],
            ..Default::default()
        };
        let from = Address::new(Some("Ann"), "ann@example.com");
        let raw = String::from_utf8(build(&draft, &from, "id@example.com", false)).unwrap();
        assert!(raw.contains("In-Reply-To: <one@x>"), "{raw}");
        assert!(!raw.contains("secret@example.com"));
        let raw = String::from_utf8(build(&draft, &from, "id@example.com", true)).unwrap();
        assert!(raw.contains("secret@example.com"));
    }
}
