//! Leaving a mailing list: RFC 8058's one-click POST to the list's https URL when it takes one,
//! else an email to its mailto address from the message's account. Both are given a short time,
//! so the socket that asked isn't held for long. The URL comes from a stranger's mail, so the
//! POST only goes to public addresses, checked after the name is resolved, and never redirects.

use std::net::{IpAddr, SocketAddr};
use std::sync::LazyLock;
use std::time::Duration;

use mail_protocol::{Address, Draft};
use percent_encoding::percent_decode_str;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::CONTENT_TYPE;
use uuid::Uuid;

use crate::{AppState, db, scheduler};

const TIMEOUT: Duration = Duration::from_secs(10);

static ONE_CLICK: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .dns_resolver(PublicOnly)
        .timeout(TIMEOUT)
        .build()
        .expect("HTTP client")
});

/// Resolves names as the system does, and refuses one that points anywhere but the internet.
struct PublicOnly;

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let found: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if found.is_empty() || !found.iter().all(|address| public(address.ip())) {
                return Err(format!("{host} isn't a public address").into());
            }
            Ok(Box::new(found.into_iter()) as Addrs)
        })
    }
}

/// Not loopback, private, link-local, shared (CGNAT), unspecified, multicast or reserved.
fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [first, second, ..] = ip.octets();
            !(ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_documentation()
                || first == 0
                || first >= 240
                || (first == 100 && (64..128).contains(&second)))
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return public(IpAddr::V4(mapped));
            }
            let first = ip.segments()[0];
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80)
        }
    }
}

/// Err is what to tell the user.
pub async fn run(state: &AppState, user_id: Uuid, id: &str) -> anyhow::Result<Result<(), String>> {
    let message = match id.parse::<Uuid>() {
        Ok(id) => db::message(&state.db, user_id, id).await?,
        Err(_) => None,
    };
    let Some(message) = message else {
        return Ok(Err("No such message.".into()));
    };
    let Some(unsubscribe) = message.unsubscribe.map(|unsubscribe| unsubscribe.0) else {
        return Ok(Err("This message has no way to unsubscribe.".into()));
    };
    let attempt = match (&unsubscribe.url, &unsubscribe.mailto) {
        (Some(url), _) if unsubscribe.one_click => tokio::time::timeout(TIMEOUT, one_click(url)).await,
        (_, Some(mailto)) => {
            let Some(account) = db::account(&state.db, message.account_id).await? else {
                return Ok(Err("The account was removed.".into()));
            };
            let Some(draft) = mailto_draft(mailto, &account.id.to_string()) else {
                return Ok(Err("This list's unsubscribe address can't be read.".into()));
            };
            let sent = async { scheduler::deliver(state, &account, &draft).await.map(|_| ()) };
            tokio::time::timeout(TIMEOUT, sent).await
        }
        (Some(_), None) => return Ok(Err("This list can only be left on its web page.".into())),
        (None, None) => return Ok(Err("This message has no way to unsubscribe.".into())),
    };
    Ok(match attempt {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => {
            tracing::info!("unsubscribing failed: {error:#}");
            Err("The list didn't take the request. Try again later.".into())
        }
        Err(_) => Err("The list took too long to answer. Try again later.".into()),
    })
}

/// A name is checked by `PublicOnly` as it is resolved; an address written in the URL here.
async fn one_click(url: &str) -> anyhow::Result<()> {
    let url = reqwest::Url::parse(url)?;
    let literal = url.host_str().unwrap_or_default().trim_matches(['[', ']']).parse::<IpAddr>();
    if url.scheme() != "https" || literal.is_ok_and(|ip| !public(ip)) {
        anyhow::bail!("{url} isn't a public https address");
    }
    ONE_CLICK
        .post(url)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body("List-Unsubscribe=One-Click")
        .timeout(TIMEOUT)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

/// The mail a `mailto:` URI asks for: its address, its `subject` (else "unsubscribe") and `body`.
/// The address must be one plain address; control characters are dropped from the rest, so
/// nothing in the URI becomes a header of its own.
fn mailto_draft(uri: &str, account_id: &str) -> Option<Draft> {
    let rest = uri.get(..7).filter(|scheme| scheme.eq_ignore_ascii_case("mailto:")).and(uri.get(7..))?;
    let (address, query) = rest.split_once('?').unwrap_or((rest, ""));
    let address = percent_decode_str(address).decode_utf8_lossy().to_string();
    let plain =
        |character: char| !character.is_control() && !character.is_whitespace() && !",;<>\"()[]\\".contains(character);
    let at_signs = address.matches('@').count();
    let (local, domain) = address.split_once('@')?;
    if at_signs != 1 || local.is_empty() || domain.is_empty() || !address.chars().all(plain) {
        return None;
    }
    let field = |wanted: &str, keep: &[char]| {
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            let value = percent_decode_str(value).decode_utf8_lossy();
            let value: String =
                value.chars().filter(|character| !character.is_control() || keep.contains(character)).collect();
            let value = value.trim().to_string();
            (key.eq_ignore_ascii_case(wanted) && !value.is_empty()).then_some(value)
        })
    };
    Some(Draft {
        account_id: account_id.to_string(),
        to: vec![Address::new(None, &address)],
        subject: field("subject", &[]).unwrap_or_else(|| "unsubscribe".into()),
        text: field("body", &['\n']).unwrap_or_else(|| "unsubscribe".into()),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_mailto_uri() {
        let draft = mailto_draft("MAILTO:leave%2Bnews@shop.com?subject=Leave%20now&body=bye", "a").unwrap();
        assert_eq!(draft.to[0].email, "leave+news@shop.com");
        assert_eq!((draft.subject.as_str(), draft.text.as_str()), ("Leave now", "bye"));

        let plain = mailto_draft("mailto:leave@shop.com", "a").unwrap();
        assert_eq!((plain.subject.as_str(), plain.text.as_str()), ("unsubscribe", "unsubscribe"));
        assert!(mailto_draft("https://shop.com", "a").is_none());
        assert!(mailto_draft("mailto:?subject=x", "a").is_none());
    }

    #[test]
    fn keeps_a_mailto_uri_from_adding_headers() {
        assert!(mailto_draft("mailto:x@evil.com%0D%0ABcc:%20victim@corp.com", "a").is_none());
        assert!(mailto_draft("mailto:x@evil.com,victim@corp.com", "a").is_none());
        assert!(mailto_draft("mailto:x@evil.com%20victim@corp.com", "a").is_none());
        let draft =
            mailto_draft("mailto:x@list.com?subject=Bye%0D%0ABcc:%20victim@corp.com&body=a%0D%0Ab", "a").unwrap();
        assert_eq!(draft.subject, "ByeBcc: victim@corp.com");
        assert_eq!(draft.text, "a\nb");
    }

    #[test]
    fn posts_only_to_public_addresses() {
        for private in [
            "127.0.0.1",
            "10.0.0.5",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fc00::1",
            "fd12::1",
            "fe80::1",
            "ff02::1",
            "::ffff:10.0.0.1",
        ] {
            assert!(!public(private.parse().unwrap()), "{private}");
        }
        for open in ["93.184.216.34", "100.128.0.1", "2606:4700::1111"] {
            assert!(public(open.parse().unwrap()), "{open}");
        }
    }

    #[tokio::test]
    async fn refuses_names_and_addresses_that_point_inside() {
        for name in ["localhost", "localhost.", "127.0.0.1.nip.io"] {
            let name: Name = name.parse().unwrap();
            assert!(PublicOnly.resolve(name).await.is_err());
        }
        mail_protocol::tls::install();
        for url in ["https://127.0.0.1/u", "https://[::1]/u", "https://10.0.0.5/u", "http://example.com/u"] {
            let error = one_click(url).await.unwrap_err().to_string();
            assert!(error.contains("isn't a public https address"), "{url}: {error}");
        }
    }
}
