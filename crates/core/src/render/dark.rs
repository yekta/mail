//! The colours a message brings, turned for a dark page: light backgrounds go dark and dark
//! text goes light, while saturated colours keep their hue. Each colour in an inline style is
//! replaced by a variable, and `bgcolor` and `color` attributes are matched by value, so the
//! page's stylesheet decides which of the two sets shows.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// The colours of one page, in the order they were met.
#[derive(Default)]
pub struct Palette {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Colours taken out of inline styles, as written; `var(--cN)` is the Nth.
    styles: Vec<String>,
    /// Values of `bgcolor` attributes and of `color` attributes, as written.
    bgcolors: BTreeMap<String, Rgba>,
    colors: BTreeMap<String, Rgba>,
}

impl Palette {
    /// An inline style with each colour replaced by a variable.
    pub fn style(&self, style: &str) -> String {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        rewrite(style, |text| {
            let index = inner.styles.iter().position(|known| known == text).unwrap_or_else(|| {
                inner.styles.push(text.to_string());
                inner.styles.len() - 1
            });
            format!("var(--c{index})")
        })
    }

    pub fn bgcolor(&self, value: &str) {
        if let Some(color) = parse(value) {
            self.inner.lock().unwrap_or_else(|e| e.into_inner()).bgcolors.insert(value.to_string(), color);
        }
    }

    pub fn color(&self, value: &str) {
        if let Some(color) = parse(value) {
            self.inner.lock().unwrap_or_else(|e| e.into_inner()).colors.insert(value.to_string(), color);
        }
    }

    /// The stylesheet: every colour as written, and under `body.dark` in the dark scheme, turned.
    pub fn css(&self) -> String {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.styles.is_empty() && inner.bgcolors.is_empty() && inner.colors.is_empty() {
            return String::new();
        }
        let mut light = String::new();
        let mut dark = String::new();
        for (index, text) in inner.styles.iter().enumerate() {
            light.push_str(&format!("--c{index}:{text};"));
            if let Some(color) = parse(text) {
                dark.push_str(&format!("--c{index}:{};", turned(color)));
            }
        }
        let mut css = format!(":root{{{light}}}@media (prefers-color-scheme: dark){{body.dark{{{dark}}}");
        for (value, color) in &inner.bgcolors {
            css.push_str(&format!(
                "body.dark [bgcolor=\"{}\" i]{{background-color:{}}}",
                escape(value),
                turned(*color)
            ));
        }
        for (value, color) in &inner.colors {
            css.push_str(&format!("body.dark [color=\"{}\" i]{{color:{}}}", escape(value), turned(*color)));
        }
        css.push('}');
        css
    }
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    r: f64,
    g: f64,
    b: f64,
    a: f64,
}

/// Replaces every colour outside quotes and `url()` with what `replace` makes of it.
fn rewrite(style: &str, mut replace: impl FnMut(&str) -> String) -> String {
    let mut out = String::with_capacity(style.len());
    let bytes = style.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &style[i..];
        let lower = rest.get(..4).map(|s| s.to_ascii_lowercase());
        if let Some(quote @ (b'"' | b'\'')) = bytes.get(i).copied() {
            let end = rest[1..].find(quote as char).map(|end| i + end + 2).unwrap_or(bytes.len());
            out.push_str(&style[i..end]);
            i = end;
            continue;
        }
        if lower.as_deref() == Some("url(") {
            let end = rest.find(')').map(|end| i + end + 1).unwrap_or(bytes.len());
            out.push_str(&style[i..end]);
            i = end;
            continue;
        }
        if i > 0 && is_word(bytes[i - 1]) {
            out.push(rest.chars().next().unwrap_or_default());
            i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
            continue;
        }
        let Some(len) = color_len(rest) else {
            out.push(rest.chars().next().unwrap_or_default());
            i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
            continue;
        };
        out.push_str(&replace(&rest[..len]));
        i += len;
    }
    out
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' || byte == b'#'
}

/// How long the colour at the start of `text` is, when there is one.
fn color_len(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.first() == Some(&b'#') {
        let digits = bytes[1..].iter().take_while(|b| b.is_ascii_hexdigit()).count();
        let ends = bytes.get(1 + digits).is_none_or(|b| !is_word(*b));
        return (matches!(digits, 3 | 4 | 6 | 8) && ends).then_some(1 + digits);
    }
    let lower = text.get(..5).map(|s| s.to_ascii_lowercase()).unwrap_or_default();
    if lower.starts_with("rgb(") || lower.starts_with("rgba(") {
        let end = text.find(')')? + 1;
        return parse(&text[..end]).map(|_| end);
    }
    let word = bytes.iter().take_while(|b| b.is_ascii_alphabetic()).count();
    let ends = bytes.get(word).is_none_or(|b| !is_word(*b));
    (word > 0 && ends && named(&text[..word]).is_some()).then_some(word)
}

pub fn parse(text: &str) -> Option<Rgba> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        let digit = |i: usize| u8::from_str_radix(hex.get(i..i + 1)?, 16).ok();
        let pair = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        let (r, g, b, a) = match hex.len() {
            3 => (digit(0)? * 17, digit(1)? * 17, digit(2)? * 17, 255),
            4 => (digit(0)? * 17, digit(1)? * 17, digit(2)? * 17, digit(3)? * 17),
            6 => (pair(0)?, pair(2)?, pair(4)?, 255),
            8 => (pair(0)?, pair(2)?, pair(4)?, pair(6)?),
            _ => return None,
        };
        return Some(Rgba { r: r as f64, g: g as f64, b: b as f64, a: a as f64 / 255.0 });
    }
    let lower = text.to_ascii_lowercase();
    if let Some(inner) = lower.strip_prefix("rgba(").or_else(|| lower.strip_prefix("rgb(")) {
        let inner = inner.strip_suffix(')')?;
        let parts: Vec<&str> = inner.split([',', ' ', '/']).map(str::trim).filter(|s| !s.is_empty()).collect();
        if parts.len() != 3 && parts.len() != 4 {
            return None;
        }
        let channel = |s: &str| -> Option<f64> {
            match s.strip_suffix('%') {
                Some(percent) => percent.parse::<f64>().ok().map(|p| p * 2.55),
                None => s.parse::<f64>().ok(),
            }
            .map(|v| v.clamp(0.0, 255.0))
        };
        let a = match parts.get(3) {
            Some(s) => match s.strip_suffix('%') {
                Some(percent) => percent.parse::<f64>().ok()? / 100.0,
                None => s.parse::<f64>().ok()?,
            },
            None => 1.0,
        };
        return Some(Rgba {
            r: channel(parts[0])?,
            g: channel(parts[1])?,
            b: channel(parts[2])?,
            a: a.clamp(0.0, 1.0),
        });
    }
    named(&lower).map(|(r, g, b)| Rgba { r: r as f64, g: g as f64, b: b as f64, a: 1.0 })
}

/// The named colours mail uses; the rest keep their names, which stay as they are.
fn named(name: &str) -> Option<(u8, u8, u8)> {
    Some(match name.to_ascii_lowercase().as_str() {
        "white" => (255, 255, 255),
        "black" => (0, 0, 0),
        "gray" | "grey" => (128, 128, 128),
        "silver" => (192, 192, 192),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "darkgray" | "darkgrey" => (169, 169, 169),
        "dimgray" | "dimgrey" => (105, 105, 105),
        "gainsboro" => (220, 220, 220),
        "whitesmoke" => (245, 245, 245),
        "red" => (255, 0, 0),
        "green" => (0, 128, 0),
        "blue" => (0, 0, 255),
        "navy" => (0, 0, 128),
        "maroon" => (128, 0, 0),
        "orange" => (255, 165, 0),
        "yellow" => (255, 255, 0),
        "purple" => (128, 0, 128),
        _ => return None,
    })
}

/// The colour for a dark page: its lightness is turned over, pulled a little in from the ends
/// so white becomes a soft dark and black a soft light, and its hue stays.
pub fn turned(color: Rgba) -> String {
    let (h, s, l) = to_hsl(color);
    let turned = 0.10 + 0.83 * (1.0 - l);
    let (r, g, b) = from_hsl(h, s, turned);
    if color.a < 1.0 {
        return format!("rgba({r},{g},{b},{})", (color.a * 100.0).round() / 100.0);
    }
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn to_hsl(color: Rgba) -> (f64, f64, f64) {
    let (r, g, b) = (color.r / 255.0, color.g / 255.0, color.b / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    if s == 0.0 {
        let v = channel(l);
        return (v, v, v);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |t: f64| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (channel(hue(h + 1.0 / 3.0)), channel(hue(h)), channel(hue(h - 1.0 / 3.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_light_dark_and_dark_light_but_keeps_hue() {
        assert_eq!(turned(parse("#ffffff").unwrap()), "#1a1a1a");
        assert_eq!(turned(parse("black").unwrap()), "#ededed");
        assert_eq!(turned(parse("#808080").unwrap()), "#838383");
        let (h, _, l) = to_hsl(parse(&turned(parse("#2da44e").unwrap())).unwrap());
        let (hue, _, light) = to_hsl(parse("#2da44e").unwrap());
        assert!((h - hue).abs() < 0.02 && l > light);
        assert_eq!(turned(parse("rgba(255, 255, 255, 0.5)").unwrap()), "rgba(26,26,26,0.5)");
    }

    #[test]
    fn replaces_colours_in_a_style_and_leaves_urls_and_words_alone() {
        let palette = Palette::default();
        let style = palette.style("background:#FFF url(white.png);color:rgb(34,34,34);font-family:'Orange Sans';border:1px solid #ddd;width:100px");
        assert_eq!(
            style,
            "background:var(--c0) url(white.png);color:var(--c1);font-family:'Orange Sans';border:1px solid var(--c2);width:100px"
        );
        assert_eq!(palette.style("color:#FFF"), "color:var(--c0)");
        assert_eq!(palette.style("color:whitesmoke1;margin:#12345"), "color:whitesmoke1;margin:#12345");
        palette.bgcolor("#f4f4f5");
        palette.color("red");
        let css = palette.css();
        assert!(css.starts_with(":root{--c0:#FFF;--c1:rgb(34,34,34);--c2:#ddd;}"));
        assert!(css.contains("body.dark{--c0:#1a1a1a;--c1:#d1d1d1;--c2:#363636;}"), "{css}");
        assert!(css.contains("body.dark [bgcolor=\"#f4f4f5\" i]{background-color:#212124}"), "{css}");
        assert!(css.contains("body.dark [color=\"red\" i]{color:"));
        assert!(Palette::default().css().is_empty());
    }
}
