//! A message's body as the page a thread's card shows: sanitized, remote images blocked unless
//! asked for, inside the theme's CSS. Mail that brings its own design (tables, colours) is drawn
//! on a "paper" card; in the dark scheme its colours are turned (`dark.rs`) unless the page is
//! asked for the original, when the card is light as its sender made it.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mail_protocol::Body;

use super::dark::Palette;
use super::text;

const TOKENS: &str = include_str!("../../../../packages/theme/tokens.css");

const STYLE: &str = r#"
html, body { margin: 0; padding: 0; background: transparent; }
body { font: 15px/1.55 "Avenir", -apple-system, BlinkMacSystemFont, "Helvetica Neue", sans-serif; color: var(--card-foreground);
  -webkit-text-size-adjust: 100%; overflow-wrap: anywhere; }
a { color: var(--primary); }
img { max-width: 100%; height: auto; }
.plain { white-space: pre-wrap; }
blockquote { margin: 0 0 0 2px; padding-left: 12px; border-left: 2px solid var(--border); color: var(--muted-foreground); }
details.quote > summary { list-style: none; display: inline-block; cursor: pointer; padding: 0 8px; margin: 6px 0;
  border-radius: 9px; background: var(--muted); color: var(--muted-foreground); font-size: 12px; line-height: 18px; letter-spacing: 1px; }
details.quote > summary::-webkit-details-marker { display: none; }
body.paper { background: #ffffff; color: #222222; color-scheme: light; border-radius: var(--radius); padding: 12px; }
body.paper a { color: #1a5fd0; }
@media (prefers-color-scheme: dark) {
  body.paper.dark { background: var(--card); color: var(--card-foreground); color-scheme: dark; }
  body.paper.dark a { color: var(--primary); }
}
table { max-width: 100%; }
"#;

pub struct Page {
    pub html: String,
    /// Remote images were left out; the client offers to load them.
    pub blocked_images: bool,
}

/// The page. With `dark`, the body carries the `dark` class: in the dark scheme the mail's
/// colours are turned. The class taken off shows the original.
pub fn page(body: &Body, show_images: bool, dark: bool) -> Page {
    let blocked = Arc::new(AtomicBool::new(false));
    let palette = Arc::new(Palette::default());
    let (content, paper) = match &body.html {
        Some(html) => (sanitize(html, show_images, blocked.clone(), palette.clone()), is_designed(html)),
        None => (text::to_html(body.text.as_deref().unwrap_or_default()), false),
    };
    let palette = palette.css();
    let images = if show_images { "data: cid: https: http:" } else { "data: cid:" };
    let mut class = String::from(if paper { "paper" } else { "themed" });
    if dark {
        class.push_str(" dark");
    }
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src {images}; style-src 'unsafe-inline'; font-src data:\">\
         <style>{TOKENS}{STYLE}{palette}</style></head><body class=\"{class}\">{content}</body></html>"
    );
    Page { html, blocked_images: blocked.load(Ordering::Relaxed) }
}

/// Whether the mail sets its own look: then the theme's colours would fight it.
pub fn is_designed(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower.contains("bgcolor")
        || lower.contains("background")
        || lower.contains("<style")
        || (lower.contains("<table") && lower.matches("style=").count() > 3)
        || lower.matches("color:").count() > 5
}

fn is_remote(url: &str) -> bool {
    let url = url.trim_start().to_ascii_lowercase();
    url.starts_with("http:") || url.starts_with("https:") || url.starts_with("//")
}

/// Drops `url(...)` with remote addresses from inline CSS.
fn without_remote_urls(style: &str) -> String {
    let mut out = String::new();
    let mut rest = style;
    while let Some(start) = rest.to_ascii_lowercase().find("url(") {
        let end = rest[start..].find(')').map(|end| start + end + 1).unwrap_or(rest.len());
        let inner = rest[start + 4..end.saturating_sub(1).max(start + 4)].trim_matches(['\'', '"', ' ']);
        out.push_str(&rest[..start]);
        if !is_remote(inner) {
            out.push_str(&rest[start..end]);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// The mail's colours go to `palette`, which makes the stylesheet the page needs for them.
pub fn sanitize(html: &str, show_images: bool, blocked: Arc<AtomicBool>, palette: Arc<Palette>) -> String {
    let mut builder = ammonia::Builder::default();
    builder
        .add_tags(["font", "tfoot", "caption"])
        .add_generic_attributes([
            "style",
            "align",
            "valign",
            "bgcolor",
            "width",
            "height",
            "border",
            "cellpadding",
            "cellspacing",
            "color",
            "dir",
            "background",
        ])
        .add_tag_attributes("font", ["face", "size"])
        .add_url_schemes(["data", "cid"])
        .link_rel(Some("noopener noreferrer"))
        .attribute_filter(move |element, attribute, value| match (element, attribute) {
            ("img", "src") | (_, "background") if !show_images && is_remote(value) => {
                blocked.store(true, Ordering::Relaxed);
                None
            }
            (_, "style") => {
                let mut style = Cow::Borrowed(value);
                if !show_images && value.to_ascii_lowercase().contains("url(") {
                    let cleaned = without_remote_urls(value);
                    if cleaned.len() != value.len() {
                        blocked.store(true, Ordering::Relaxed);
                    }
                    style = Cow::Owned(cleaned);
                }
                Some(Cow::Owned(palette.style(&style)))
            }
            (_, "bgcolor") => {
                palette.bgcolor(value);
                Some(Cow::Borrowed(value))
            }
            (_, "color") => {
                palette.color(value);
                Some(Cow::Borrowed(value))
            }
            _ => Some(Cow::Borrowed(value)),
        });
    builder.clean(html).to_string()
}

const PRINT_STYLE: &str = r#"
body { font: 13px/1.5 "Avenir", -apple-system, BlinkMacSystemFont, "Helvetica Neue", sans-serif; color: #111; background: #fff;
  margin: 24px; overflow-wrap: anywhere; }
h1 { font-size: 18px; margin: 0 0 16px; }
.message { border-top: 1px solid #ccc; padding: 12px 0; }
.headers { color: #555; margin-bottom: 12px; }
.headers b { color: #111; }
.plain { white-space: pre-wrap; }
blockquote { margin: 0 0 0 2px; padding-left: 12px; border-left: 2px solid #ccc; color: #555; }
details.quote > summary { display: none; }
img { max-width: 100%; height: auto; }
table { max-width: 100%; }
"#;

/// One message of a thread to print, its headers ready to show.
pub struct Printed<'a> {
    pub from: String,
    pub to: String,
    pub cc: String,
    pub date: String,
    pub body: Option<Body>,
    pub snippet: &'a str,
}

/// A whole thread as one page to print: every message open, with its headers, remote images left
/// out.
pub fn print(subject: &str, messages: &[Printed]) -> String {
    let subject = text::escape(subject);
    let palette = Arc::new(Palette::default());
    let mut html = format!("<h1>{subject}</h1>");
    for message in messages {
        let content = match &message.body {
            Some(Body { html: Some(body), .. }) => {
                sanitize(body, false, Arc::new(AtomicBool::new(false)), palette.clone())
            }
            Some(Body { text: Some(body), .. }) => {
                text::to_html(body).replace("<details class=\"quote\">", "<details class=\"quote\" open>")
            }
            _ => format!("<div class=\"plain\">{}</div>", text::escape(message.snippet)),
        };
        let mut headers = format!("<b>{}</b>", text::escape(&message.from));
        for (name, value) in [("To", &message.to), ("Cc", &message.cc), ("Date", &message.date)] {
            if !value.is_empty() {
                headers.push_str(&format!("<br>{name}: {}", text::escape(value)));
            }
        }
        html.push_str(&format!("<div class=\"message\"><div class=\"headers\">{headers}</div>{content}</div>"));
    }
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{subject}</title>\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data: cid:; style-src 'unsafe-inline'\">\
         <style>{PRINT_STYLE}{}</style></head><body>{html}</body></html>",
        palette.css()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(html: &str) -> Body {
        Body { html: Some(html.into()), text: None }
    }

    #[test]
    fn removes_scripts_and_handlers() {
        let page = page(
            &html("<p onclick=\"steal()\">Hi<script>alert(1)</script></p><a href=\"javascript:x()\">x</a>"),
            false,
            true,
        );
        assert!(!page.html.contains("steal"));
        assert!(!page.html.contains("alert"));
        assert!(!page.html.contains("javascript:"));
        assert!(page.html.contains("<p>Hi</p>"));
    }

    #[test]
    fn blocks_remote_images_until_asked() {
        let mail = html(
            "<img src=\"https://tracker.example/p.gif\"><div style=\"background:url(https://x.example/bg.png);color:red\">Hi</div><img src=\"data:image/png;base64,AA==\">",
        );
        let blocked = page(&mail, false, true);
        assert!(blocked.blocked_images);
        assert!(!blocked.html.contains("tracker.example"));
        assert!(!blocked.html.contains("x.example"));
        assert!(blocked.html.contains("color:var(--c0)") && blocked.html.contains("--c0:red;"));
        assert!(blocked.html.contains("data:image/png"));
        assert!(blocked.html.contains("img-src data: cid:;"));

        let shown = page(&mail, true, true);
        assert!(!shown.blocked_images);
        assert!(shown.html.contains("https://tracker.example/p.gif"));
    }

    #[test]
    fn designed_mail_is_drawn_on_paper() {
        assert!(
            page(&html("<table bgcolor=\"#eee\"><tr><td>Sale</td></tr></table>"), false, true)
                .html
                .contains("class=\"paper dark\"")
        );
        assert!(page(&html("<div>Hi Ann,<br>See you.</div>"), false, true).html.contains("class=\"themed dark\""));
        let plain = page(&Body { html: None, text: Some("Hi <b>".into()) }, false, true);
        assert!(plain.html.contains("class=\"themed dark\"") && plain.html.contains("Hi &lt;b&gt;"));
    }

    #[test]
    fn prints_every_message_open_with_its_headers() {
        let messages = [
            Printed {
                from: "Ann <ann@x.com>".into(),
                to: "me@example.com".into(),
                cc: String::new(),
                date: "Mar 3, 2026 at 9:41 AM".into(),
                body: Some(Body { html: None, text: Some("Lunch?\n\nOn Mar 2, Bob wrote:\n> Hungry".into()) }),
                snippet: "",
            },
            Printed {
                from: "Bob".into(),
                to: String::new(),
                cc: String::new(),
                date: String::new(),
                body: Some(html(
                    "<p style=\"color:#333\">Yes<img src=\"https://t.example/p.gif\"><script>x()</script></p>",
                )),
                snippet: "",
            },
            Printed {
                from: "Cy".into(),
                to: String::new(),
                cc: String::new(),
                date: String::new(),
                body: None,
                snippet: "On its way",
            },
        ];
        let page = print("Lunch <3", &messages);
        assert!(page.contains("<title>Lunch &lt;3</title>"));
        assert!(page.contains("<b>Ann &lt;ann@x.com&gt;</b><br>To: me@example.com<br>Date: Mar 3, 2026 at 9:41 AM"));
        assert!(page.contains("<details class=\"quote\" open>"));
        assert!(page.contains("<p style=\"color:var(--c0)\">Yes") && page.contains("--c0:#333;"));
        assert!(!page.contains("t.example") && !page.contains("x()"));
        assert!(page.contains("On its way"));
    }

    #[test]
    fn carries_the_theme() {
        assert!(page(&html("<p>x</p>"), false, true).html.contains("--background:"));
    }

    #[test]
    fn turns_the_mails_colours_for_the_dark_scheme() {
        let mail = html(
            "<table bgcolor=\"#ffffff\"><tr><td style=\"background:#f4f4f5;color:#222\"><font color=\"gray\">Sale</font></td></tr></table>",
        );
        let dark = page(&mail, false, true);
        assert!(dark.html.contains("class=\"paper dark\""));
        assert!(dark.html.contains("style=\"background:var(--c0);color:var(--c1)\""));
        assert!(dark.html.contains(":root{--c0:#f4f4f5;--c1:#222;}"));
        assert!(dark.html.contains("body.dark [bgcolor=\"#ffffff\" i]{background-color:#1a1a1a}"));
        assert!(dark.html.contains("body.dark [color=\"gray\" i]{color:"));
        assert!(dark.html.contains("bgcolor=\"#ffffff\""));

        let light = page(&mail, false, false);
        assert!(light.html.contains("class=\"paper\""));
        assert!(!page(&html("<p>Hi</p>"), false, true).html.contains(":root{--c0"));
    }
}
