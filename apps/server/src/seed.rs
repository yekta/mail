//! `mail-server seed`: makes a demo user on a Stalwart server and fills its mailbox with a few
//! hundred made-up messages, in threads, some read, starred or archived. For local work and the
//! end-to-end tests.
//!
//! STALWART_URL (http://localhost:8080), STALWART_ADMIN (admin:adminpass), SEED_USER
//! (demo@example.com), SEED_PASSWORD (quiet-harbor-lantern-42), SEED_COUNT (300).

use std::env;

use mail_builder::MessageBuilder;
use mail_builder::headers::address::Address as BuilderAddress;
use serde_json::{Value, json};

use crate::providers::jmap::Jmap;

const PEOPLE: [(&str, &str); 12] = [
    ("Ada Lovelace", "ada@analytical.engine"),
    ("Alan Turing", "alan@bletchley.park"),
    ("Grace Hopper", "grace@navy.mil"),
    ("Katherine Johnson", "katherine@nasa.gov"),
    ("Claude Shannon", "claude@bell-labs.com"),
    ("Hedy Lamarr", "hedy@frequency.hop"),
    ("Edsger Dijkstra", "edsger@shortest.path"),
    ("Margaret Hamilton", "margaret@apollo.mit.edu"),
    ("Linus Pauling", "linus@caltech.edu"),
    ("Rosalind Franklin", "rosalind@kings.ac.uk"),
    ("Nikola Tesla", "nikola@wardenclyffe.org"),
    ("Marie Curie", "marie@sorbonne.fr"),
];

const SUBJECTS: [&str; 24] = [
    "Notes from Thursday",
    "Draft of the paper",
    "Lunch next week?",
    "The numbers look off",
    "Slides for the talk",
    "Quick question about the proof",
    "Travel plans for the conference",
    "Re-run the experiment",
    "Thank you!",
    "Reading list",
    "Can you review this?",
    "Budget for the lab",
    "The new prototype works",
    "Photos from the trip",
    "Meeting moved to 3pm",
    "Feedback on chapter two",
    "A thought on compilers",
    "Grant deadline",
    "Book recommendation",
    "Coffee machine is fixed",
    "Updated schedule",
    "Weekend hike",
    "Are we still on for Friday?",
    "Results are in",
];

const LINES: [&str; 16] = [
    "I went through it again this morning and I think we are close.",
    "Let me know what you think when you have a minute.",
    "The second half needs another pass, but the idea holds up.",
    "I attached the latest version, the changes are on page four.",
    "Happy to talk it over on a call if that's easier.",
    "We should keep it simple and see how far that gets us.",
    "No rush on this, next week is fine.",
    "I found a small mistake in the third table and fixed it.",
    "Thanks again for helping out with this.",
    "Could you send me the raw data as well?",
    "I'll be in the office all afternoon.",
    "Here is the short version: it works, but it's slow.",
    "I booked the room for Tuesday at ten.",
    "The reviewers liked it, with a few comments.",
    "Sounds good to me.",
    "See you there.",
];

const NEWSLETTER: &str = r##"<html><body style="margin:0;background:#eef1f5">
<table width="100%" cellpadding="0" cellspacing="0" bgcolor="#eef1f5"><tr><td align="center" style="padding:24px">
<table width="560" cellpadding="0" cellspacing="0" bgcolor="#ffffff" style="border-radius:8px">
<tr><td style="padding:32px;font-family:Helvetica,Arial,sans-serif;color:#222">
<h1 style="margin:0 0 12px;font-size:24px;color:#1d4ed8">The Weekly Orbit</h1>
<p style="font-size:15px;line-height:22px">Three launches, one landing and a comet you can see without a telescope this weekend.</p>
<p><a href="https://example.com/read" style="background:#1d4ed8;color:#fff;padding:10px 18px;border-radius:6px;text-decoration:none">Read the issue</a></p>
<img src="https://example.com/pixel.gif" width="1" height="1" alt="">
</td></tr></table></td></tr></table></body></html>"##;

struct Settings {
    url: String,
    admin: (String, String),
    user: String,
    password: String,
    count: usize,
}

fn settings() -> Settings {
    let var =
        |name: &str, default: &str| env::var(name).ok().filter(|value| !value.is_empty()).unwrap_or(default.into());
    let admin = var("STALWART_ADMIN", "admin:adminpass");
    let (name, secret) = admin.split_once(':').unwrap_or(("admin", ""));
    Settings {
        url: var("STALWART_URL", "http://localhost:8080"),
        admin: (name.to_string(), secret.to_string()),
        user: var("SEED_USER", "demo@example.com"),
        password: var("SEED_PASSWORD", "quiet-harbor-lantern-42"),
        count: var("SEED_COUNT", "300").parse().unwrap_or(300),
    }
}

pub async fn run() -> anyhow::Result<()> {
    mail_protocol::tls::install();
    let settings = settings();
    let http = reqwest::Client::new();
    create_user(&http, &settings.url, &settings.admin, &settings.user, &settings.password).await?;
    let jmap = Jmap::open(&http, &settings.url, &settings.user, &settings.password).await?;
    let existing = jmap
        .call(
            &["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
            vec![("Email/query", json!({ "limit": 1, "calculateTotal": true }))],
        )
        .await?;
    if existing[0]["total"].as_u64().unwrap_or(0) > 0 && env::var("SEED_FORCE").is_err() {
        println!("{} already has mail; set SEED_FORCE=1 to add more.", settings.user);
        return Ok(());
    }
    let added = fill(&jmap, &settings.user, settings.count, chrono::Utc::now().timestamp()).await?;
    println!("Added {added} messages to {} at {}", settings.user, settings.url);
    Ok(())
}

/// Makes the domain and the user through Stalwart's management objects, if they are missing.
pub async fn create_user(
    http: &reqwest::Client,
    url: &str,
    admin: &(String, String),
    email: &str,
    password: &str,
) -> anyhow::Result<()> {
    let (name, domain) = email.split_once('@').ok_or_else(|| anyhow::anyhow!("{email} isn't an address"))?;
    let call = |calls: Value| {
        let request = http.post(format!("{}/jmap/", url.trim_end_matches('/'))).basic_auth(&admin.0, Some(&admin.1));
        async move {
            let body = json!({ "using": ["urn:ietf:params:jmap:core", "urn:stalwart:jmap"], "methodCalls": calls });
            let response: Value = request.json(&body).send().await?.error_for_status()?.json().await?;
            anyhow::Ok(response["methodResponses"].clone())
        }
    };
    let session: Value = http
        .get(format!("{}/jmap/session", url.trim_end_matches('/')))
        .basic_auth(&admin.0, Some(&admin.1))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let account = session["primaryAccounts"]["urn:stalwart:jmap"].as_str().unwrap_or_default().to_string();

    let found = call(json!([
        ["x:Domain/get", { "accountId": account, "properties": ["id", "name"] }, "0"],
        ["x:Account/get", { "accountId": account, "properties": ["id", "emailAddress"] }, "1"],
    ]))
    .await?;
    if found[1][1]["list"].as_array().into_iter().flatten().any(|existing| existing["emailAddress"] == email) {
        return Ok(());
    }
    let domain_id =
        match found[0][1]["list"].as_array().into_iter().flatten().find(|existing| existing["name"] == domain) {
            Some(existing) => existing["id"].as_str().unwrap_or_default().to_string(),
            None => {
                let made = call(
                    json!([["x:Domain/set", { "accountId": account, "create": { "d": { "name": domain } } }, "0"]]),
                )
                .await?;
                made[0][1]["created"]["d"]["id"]
                    .as_str()
                    .map(String::from)
                    .ok_or_else(|| anyhow::anyhow!("Stalwart didn't make {domain}: {}", made[0][1]))?
            }
        };
    let user = json!({ "@type": "User", "name": name, "domainId": domain_id,
        "credentials": { "0": { "@type": "Password", "secret": password } } });
    let made = call(json!([["x:Account/set", { "accountId": account, "create": { "u": user } }, "0"]])).await?;
    if made[0][1]["created"]["u"].is_null() {
        anyhow::bail!("Stalwart didn't make {email}: {}", made[0][1]["notCreated"]["u"]);
    }
    Ok(())
}

/// A small deterministic random sequence, so every seeded mailbox looks the same.
struct Dice(u64);

impl Dice {
    fn next(&mut self, below: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) % below as u64) as usize
    }
}

/// Adds `count` messages, newest at `now` (unix seconds). Returns how many were added.
pub async fn fill(jmap: &Jmap, me: &str, count: usize, now: i64) -> anyhow::Result<usize> {
    let mailboxes = jmap
        .call(
            &["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
            vec![("Mailbox/get", json!({ "ids": null }))],
        )
        .await?;
    let role = |wanted: &str| {
        mailboxes[0]["list"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|mailbox| mailbox["role"] == wanted)
            .and_then(|mailbox| mailbox["id"].as_str())
            .map(String::from)
    };
    let inbox = role("inbox").ok_or_else(|| anyhow::anyhow!("the account has no inbox"))?;
    let sent = role("sent").unwrap_or(inbox.clone());
    let archive = match role("archive") {
        Some(id) => id,
        None => {
            let made = jmap
                .call(
                    &["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
                    vec![("Mailbox/set", json!({ "create": { "a": { "name": "Archive", "role": "archive" } } }))],
                )
                .await?;
            made[0]["created"]["a"]["id"].as_str().unwrap_or(&inbox).to_string()
        }
    };

    let mut dice = Dice(42);
    let mut emails = Vec::new();
    let mut added = 0;
    let mut time = now;
    let domain = me.split('@').nth(1).unwrap_or("example.com");
    while added < count {
        let person = PEOPLE[dice.next(PEOPLE.len())];
        let subject = SUBJECTS[dice.next(SUBJECTS.len())];
        let length = 1 + dice.next(4).min(count - added - 1);
        let archived = dice.next(4) == 0;
        let starred = dice.next(9) == 0;
        let newsletter = dice.next(14) == 0;
        let mut references: Vec<String> = Vec::new();
        time -= 600 + dice.next(20_000) as i64;
        let mut times: Vec<i64> =
            (0..length).map(|index| time - (length - 1 - index) as i64 * (300 + dice.next(7_200) as i64)).collect();
        times.sort();
        for (index, at) in times.into_iter().enumerate() {
            let from_me = index % 2 == 1;
            let id = format!("seed-{added}@{domain}");
            let (from, to) = if from_me {
                ((None, me), (Some(person.0), person.1))
            } else {
                ((Some(person.0), person.1), (None, me))
            };
            let text = (0..2 + dice.next(3)).map(|_| LINES[dice.next(LINES.len())]).collect::<Vec<_>>().join(" ");
            let mut builder = MessageBuilder::new()
                .from(BuilderAddress::new_address(from.0, from.1))
                .to(BuilderAddress::new_address(to.0, to.1))
                .subject(if index == 0 { subject.to_string() } else { format!("Re: {subject}") })
                .message_id(id.as_str())
                .date(at);
            if let Some(parent) = references.last() {
                builder = builder.in_reply_to(parent.as_str()).references(references.clone());
            }
            builder = match (newsletter && index == 0, index) {
                (true, _) => builder
                    .html_body(NEWSLETTER)
                    .text_body("The Weekly Orbit: three launches, one landing and a comet."),
                (false, 0) => {
                    builder.text_body(format!("Hi,\n\n{text}\n\n{}", person.0.split(' ').next().unwrap_or_default()))
                }
                (false, _) => builder.text_body(format!(
                    "{text}\n\nOn an earlier day, someone wrote:\n> {}\n",
                    LINES[dice.next(LINES.len())]
                )),
            };
            if dice.next(12) == 0 {
                builder = builder.attachment("text/plain", "notes.txt", "Plain notes, attached.".as_bytes());
            }
            let raw = builder.write_to_vec()?;
            references.push(id);
            let mailbox = match (from_me, archived) {
                (true, _) => sent.clone(),
                (false, true) => archive.clone(),
                (false, false) => inbox.clone(),
            };
            let read = from_me || archived || dice.next(3) != 0;
            let mut keywords = serde_json::Map::new();
            if read {
                keywords.insert("$seen".into(), json!(true));
            }
            if starred && index == 0 {
                keywords.insert("$flagged".into(), json!(true));
            }
            let blob = jmap.upload(raw).await?;
            let received = chrono::DateTime::from_timestamp(at, 0)
                .unwrap_or_default()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            emails.push(json!({ "blobId": blob, "mailboxIds": { mailbox: true }, "keywords": keywords, "receivedAt": received }));
            added += 1;
        }
        if emails.len() >= 50 {
            jmap.import(std::mem::take(&mut emails)).await?;
        }
    }
    if !emails.is_empty() {
        jmap.import(emails).await?;
    }
    Ok(added)
}
