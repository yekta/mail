//! Raw MIME in and out: what a body and its attachments are, the headers of a message as a
//! provider gives them, and the message a draft becomes.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use mail_builder::MessageBuilder;
use mail_builder::headers::address::Address as BuilderAddress;
use mail_parser::{Message, MessageParser, MessagePart, MimeHeaders, PartType};
use mail_protocol::{Address, Attachment, Body, Draft, Recipients, Unsubscribe};

/// Inline images larger than this together stay out of the body.
const INLINE_LIMIT: usize = 2_000_000;

pub struct Parsed {
    pub body: Body,
    pub attachments: Vec<Attachment>,
    /// The body as plain words, for search.
    pub plain: String,
}

/// An attachment with its contents.
pub struct File {
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

pub fn parse(raw: &[u8]) -> Option<Parsed> {
    let message = MessageParser::default().parse(raw)?;
    let mut html = html_of(&message);
    let text = message.text_bodies().find_map(|part| match &part.body {
        PartType::Text(text) => Some(text.to_string()),
        _ => None,
    });

    let (inline, listed) = split_attachments(&message, html.as_deref());
    let mut inline_budget = INLINE_LIMIT;
    for (id, part) in inline {
        let contents = part.contents();
        let Some(page) = html.as_mut().filter(|_| contents.len() <= inline_budget) else { continue };
        inline_budget -= contents.len();
        let data = format!("data:{};base64,{}", mime_of(part), STANDARD.encode(contents));
        *page = page.replace(&format!("cid:{id}"), &data);
    }
    let attachments = listed.iter().map(|part| describe(part)).collect();

    let plain = message.body_text(0).map(|text| text.to_string()).unwrap_or_default();
    Some(Parsed { body: Body { html, text }, attachments, plain })
}

/// The message's attachments with their contents, in the order `parse` lists them.
pub fn files(raw: &[u8]) -> Vec<File> {
    let Some(message) = MessageParser::default().parse(raw) else { return Vec::new() };
    let html = html_of(&message);
    let (_, listed) = split_attachments(&message, html.as_deref());
    listed
        .into_iter()
        .map(|part| {
            let Attachment { name, mime, .. } = describe(part);
            File { name, mime, bytes: part.contents().to_vec() }
        })
        .collect()
}

fn html_of(message: &Message<'_>) -> Option<String> {
    message.html_bodies().find_map(|part| match &part.body {
        PartType::Html(html) => Some(html.to_string()),
        _ => None,
    })
}

/// The attachments the HTML shows by their Content-ID, with it, and the ones listed as files.
fn split_attachments<'a>(
    message: &'a Message<'a>,
    html: Option<&str>,
) -> (Vec<(String, &'a MessagePart<'a>)>, Vec<&'a MessagePart<'a>>) {
    let mut inline = Vec::new();
    let mut listed = Vec::new();
    for part in message.attachments() {
        let content_id = part.content_id().map(|id| id.trim_matches(['<', '>']).to_string());
        match content_id {
            Some(id) if html.is_some_and(|page| page.contains(&format!("cid:{id}"))) => inline.push((id, part)),
            _ => listed.push(part),
        }
    }
    (inline, listed)
}

fn mime_of(part: &MessagePart<'_>) -> String {
    part.content_type()
        .map(|kind| format!("{}/{}", kind.ctype(), kind.subtype().unwrap_or("octet-stream")))
        .unwrap_or_else(|| "application/octet-stream".into())
}

fn describe(part: &MessagePart<'_>) -> Attachment {
    let name = part.attachment_name().unwrap_or("Attachment").to_string();
    Attachment { name, mime: mime_of(part), size: part.contents().len() as i64 }
}

/// What a message's list headers say: whether it was sent in bulk, and how to leave the list.
#[derive(Default)]
pub struct ListHeaders<'a> {
    /// The URLs of List-Unsubscribe.
    pub unsubscribe: Vec<String>,
    pub unsubscribe_post: Option<&'a str>,
    pub list_id: Option<&'a str>,
    pub precedence: Option<&'a str>,
    pub auto_submitted: Option<&'a str>,
}

impl ListHeaders<'_> {
    pub fn bulk(&self) -> bool {
        let present = |value: Option<&str>| value.is_some_and(|value| !value.trim().is_empty());
        let precedence = self.precedence.map(|value| value.trim().to_ascii_lowercase());
        let auto_submitted = self.auto_submitted.map(|value| value.trim().to_ascii_lowercase());
        !self.unsubscribe.is_empty()
            || present(self.list_id)
            || precedence.is_some_and(|value| ["bulk", "list", "junk"].contains(&value.as_str()))
            || auto_submitted.is_some_and(|value| !value.is_empty() && !value.starts_with("no"))
    }

    /// Only https URLs are kept; the mailto keeps its `?subject=`.
    pub fn unsubscribe(&self) -> Option<Unsubscribe> {
        let starting = |prefix: &str| {
            self.unsubscribe
                .iter()
                .find(|url| url.get(..prefix.len()).is_some_and(|start| start.eq_ignore_ascii_case(prefix)))
        };
        let url = starting("https://").cloned();
        let mailto = starting("mailto:").cloned();
        if url.is_none() && mailto.is_none() {
            return None;
        }
        let one_click = url.is_some()
            && self
                .unsubscribe_post
                .is_some_and(|post| post.to_ascii_lowercase().contains("list-unsubscribe=one-click"));
        Some(Unsubscribe { url, mailto, one_click })
    }
}

/// The URLs of a List-Unsubscribe header: `<https://...>, <mailto:...>`.
pub fn unsubscribe_urls(header: &str) -> Vec<String> {
    header
        .split('<')
        .skip(1)
        .filter_map(|part| part.split_once('>'))
        .map(|(url, _)| url.split_whitespace().collect::<String>())
        .filter(|url| !url.is_empty())
        .collect()
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
    pub bulk: bool,
    pub unsubscribe: Option<Unsubscribe>,
}

pub fn headers(lines: &[(String, String)]) -> Headers {
    let mut raw = String::new();
    for (name, value) in lines {
        raw.push_str(&format!("{name}: {value}\r\n"));
    }
    raw.push_str("\r\n");
    let line = |wanted: &str| {
        lines.iter().find(|(name, _)| name.eq_ignore_ascii_case(wanted)).map(|(_, value)| value.as_str())
    };
    let list = ListHeaders {
        unsubscribe: line("List-Unsubscribe").map(unsubscribe_urls).unwrap_or_default(),
        unsubscribe_post: line("List-Unsubscribe-Post"),
        list_id: line("List-Id"),
        precedence: line("Precedence"),
        auto_submitted: line("Auto-Submitted"),
    };
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
            bulk: list.bulk(),
            unsubscribe: list.unsubscribe(),
        };
    };
    let addresses = |address: Option<&mail_parser::Address>| -> Vec<Address> {
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
    let content_type = line("Content-Type").map(str::to_ascii_lowercase).unwrap_or_default();
    Headers {
        from: addresses(message.from()).into_iter().next().unwrap_or_default(),
        recipients: Recipients {
            to: addresses(message.to()),
            cc: addresses(message.cc()),
            bcc: addresses(message.bcc()),
            reply_to: addresses(message.reply_to()),
        },
        subject: message.subject().unwrap_or_default().to_string(),
        date: message.date().map(|date| date.to_timestamp() * 1000),
        message_id: message.message_id().map(String::from),
        in_reply_to: ids(message.in_reply_to()).into_iter().next(),
        references: ids(message.references()),
        multipart_mixed: content_type.starts_with("multipart/mixed"),
        bulk: list.bulk(),
        unsubscribe: list.unsubscribe(),
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

/// The message a draft becomes, with `draft.html` beside its text and `files` attached.
/// `with_bcc` keeps the Bcc header, for a provider that reads the recipients from it and strips
/// it (Gmail); otherwise they go in the envelope.
pub fn build(draft: &Draft, from: &Address, message_id: &str, with_bcc: bool, files: &[File]) -> Vec<u8> {
    let mut builder = MessageBuilder::new()
        .from(BuilderAddress::new_address(from.name.as_deref(), from.email.as_str()))
        .subject(draft.subject.as_str())
        .message_id(message_id)
        .text_body(draft.text.as_str());
    if let Some(html) = &draft.html {
        builder = builder.html_body(html.as_str());
    }
    for file in files {
        builder = builder.attachment(file.mime.as_str(), file.name.as_str(), file.bytes.as_slice());
    }
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
    fn lists_the_same_files_as_the_attachments() {
        let raw = "Subject: Hi\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
            --b\r\nContent-Type: text/html\r\n\r\n<p><img src=\"cid:logo\"></p>\r\n\
            --b\r\nContent-Type: image/png\r\nContent-ID: <logo>\r\n\r\nPNG\r\n\
            --b\r\nContent-Type: text/csv\r\nContent-Disposition: attachment; filename=\"a.csv\"\r\n\r\n1,2\r\n\
            --b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"b.pdf\"\r\n\r\n%PDF\r\n\
            --b--\r\n";
        let names: Vec<String> = parse(raw.as_bytes()).unwrap().attachments.into_iter().map(|file| file.name).collect();
        let files = files(raw.as_bytes());
        assert_eq!(names, ["a.csv", "b.pdf"]);
        assert_eq!(files.iter().map(|file| file.name.as_str()).collect::<Vec<_>>(), names);
        assert_eq!((files[1].mime.as_str(), files[1].bytes.as_slice()), ("application/pdf", b"%PDF".as_slice()));
    }

    #[test]
    fn reads_how_to_leave_a_list() {
        let headers = headers(&[
            ("From".into(), "news@shop.com".into()),
            (
                "List-Unsubscribe".into(),
                "<mailto:leave@shop.com?subject=unsubscribe>, <http://shop.com/u>, <https://shop.com/u?id=1>".into(),
            ),
            ("List-Unsubscribe-Post".into(), "List-Unsubscribe=One-Click".into()),
        ]);
        assert!(headers.bulk);
        let unsubscribe = headers.unsubscribe.unwrap();
        assert_eq!(unsubscribe.url.as_deref(), Some("https://shop.com/u?id=1"));
        assert_eq!(unsubscribe.mailto.as_deref(), Some("mailto:leave@shop.com?subject=unsubscribe"));
        assert!(unsubscribe.one_click);

        let plain = headers_of(&[("List-Unsubscribe", "<http://shop.com/u>")]);
        assert!(plain.bulk && plain.unsubscribe.is_none());
        let no_post = headers_of(&[("List-Unsubscribe", "<https://shop.com/u>")]).unsubscribe.unwrap();
        assert!(!no_post.one_click);
    }

    #[test]
    fn tells_bulk_mail_from_a_person() {
        assert!(!headers_of(&[("From", "ann@example.com")]).bulk);
        assert!(headers_of(&[("List-Id", "<team.lists.example.com>")]).bulk);
        assert!(headers_of(&[("Precedence", "Bulk")]).bulk);
        assert!(headers_of(&[("Auto-Submitted", "auto-generated")]).bulk);
        assert!(!headers_of(&[("Auto-Submitted", "no")]).bulk);
        assert!(!headers_of(&[("Precedence", "first-class")]).bulk);
    }

    fn headers_of(lines: &[(&str, &str)]) -> Headers {
        headers(&lines.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect::<Vec<_>>())
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
        let raw = String::from_utf8(build(&draft, &from, "id@example.com", false, &[])).unwrap();
        assert!(raw.contains("In-Reply-To: <one@x>"), "{raw}");
        assert!(!raw.contains("secret@example.com"));
        let raw = String::from_utf8(build(&draft, &from, "id@example.com", true, &[])).unwrap();
        assert!(raw.contains("secret@example.com"));
    }

    #[test]
    fn builds_html_beside_the_text_with_files() {
        let draft = Draft {
            subject: "Plan".into(),
            text: "See the plan.".into(),
            html: Some("<p>See the plan.</p>".into()),
            ..Default::default()
        };
        let file = File { name: "plan.pdf".into(), mime: "application/pdf".into(), bytes: b"%PDF-1".to_vec() };
        let raw = build(&draft, &Address::new(Some("Ann"), "ann@example.com"), "id@example.com", false, &[file]);
        let parsed = parse(&raw).unwrap();
        assert_eq!(parsed.body.html.as_deref().map(str::trim), Some("<p>See the plan.</p>"));
        assert_eq!(parsed.body.text.as_deref().map(str::trim), Some("See the plan."));
        assert_eq!(files(&raw)[0].bytes, b"%PDF-1");
        assert!(String::from_utf8_lossy(&raw).contains("From: \"Ann\" <ann@example.com>"));
    }
}
