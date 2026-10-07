//! A message's body as the page a thread's card shows: sanitized, remote images blocked unless
//! asked for, inside the theme's CSS. Mail that brings its own design (tables, colours) is drawn
//! on a light "paper" card, also in dark mode, as its sender made it.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mail_protocol::Body;

use super::text;

const TOKENS: &str = include_str!("../../../../packages/theme/tokens.css");

const STYLE: &str = r#"
html, body { margin: 0; padding: 0; background: transparent; }
body { font: 15px/1.55 -apple-system, BlinkMacSystemFont, "Helvetica Neue", sans-serif; color: var(--card-foreground);
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
table { max-width: 100%; }
"#;

pub struct Page {
    pub html: String,
    /// Remote images were left out; the client offers to load them.
    pub blocked_images: bool,
}

pub fn page(body: &Body, show_images: bool) -> Page {
    let blocked = Arc::new(AtomicBool::new(false));
    let (content, paper) = match &body.html {
        Some(html) => (sanitize(html, show_images, blocked.clone()), is_designed(html)),
        None => (text::to_html(body.text.as_deref().unwrap_or_default()), false),
    };
    let images = if show_images { "data: cid: https: http:" } else { "data: cid:" };
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src {images}; style-src 'unsafe-inline'; font-src data:\">\
         <style>{TOKENS}{STYLE}</style></head><body class=\"{}\">{content}</body></html>",
        if paper { "paper" } else { "themed" }
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

pub fn sanitize(html: &str, show_images: bool, blocked: Arc<AtomicBool>) -> String {
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
        .attribute_filter(move |element, attribute, value| {
            if show_images {
                return Some(Cow::Borrowed(value));
            }
            match (element, attribute) {
                ("img", "src") | (_, "background") if is_remote(value) => {
                    blocked.store(true, Ordering::Relaxed);
                    None
                }
                (_, "style") if value.to_ascii_lowercase().contains("url(") => {
                    let cleaned = without_remote_urls(value);
                    if cleaned.len() != value.len() {
                        blocked.store(true, Ordering::Relaxed);
                    }
                    Some(Cow::Owned(cleaned))
                }
                _ => Some(Cow::Borrowed(value)),
            }
        });
    builder.clean(html).to_string()
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
        let blocked = page(&mail, false);
        assert!(blocked.blocked_images);
        assert!(!blocked.html.contains("tracker.example"));
        assert!(!blocked.html.contains("x.example"));
        assert!(blocked.html.contains("color:red"));
        assert!(blocked.html.contains("data:image/png"));
        assert!(blocked.html.contains("img-src data: cid:;"));

        let shown = page(&mail, true);
        assert!(!shown.blocked_images);
        assert!(shown.html.contains("https://tracker.example/p.gif"));
    }

    #[test]
    fn designed_mail_is_drawn_on_paper() {
        assert!(
            page(&html("<table bgcolor=\"#eee\"><tr><td>Sale</td></tr></table>"), false)
                .html
                .contains("class=\"paper\"")
        );
        assert!(page(&html("<div>Hi Ann,<br>See you.</div>"), false).html.contains("class=\"themed\""));
        let plain = page(&Body { html: None, text: Some("Hi <b>".into()) }, false);
        assert!(plain.html.contains("class=\"themed\"") && plain.html.contains("Hi &lt;b&gt;"));
    }

    #[test]
    fn carries_the_theme() {
        assert!(page(&html("<p>x</p>"), false).html.contains("--background:"));
    }
}
