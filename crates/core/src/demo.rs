//! Made-up mail, for looking at the apps without a server. With `Config::demo` the core fills a
//! fresh local copy with it at every start and never connects.

use anyhow::Result;
use mail_protocol::{
    Account, Address, Attachment, Body, Draft, Identity, Label, Message, Preference, Provider, Recipients, SavedDraft,
    Unsubscribe, role,
};
use serde_json::json;

use crate::store::{Batch, Store};

const ACCOUNTS: [(&str, &str, Provider, &str); 2] = [
    ("demo-home", "sam@example.com", Provider::Gmail, "account-2"),
    ("demo-work", "sam@acme.example", Provider::Jmap, "account-5"),
];

/// What each account sends as: its own address, and for work an alias, with their signatures.
const IDENTITIES: [&[(&str, Option<&str>)]; 2] = [
    &[("sam@example.com", None)],
    &[("sam@acme.example", Some("Sam Lee\nProduct, Acme")), ("hello@acme.example", Some("The Acme team"))],
];

const LABELS: [(&str, usize, &str); 2] = [("travel", 0, "Travel"), ("clients", 1, "Clients")];

const HOUR: i64 = 60;
const DAY: i64 = 24 * HOUR;

struct Thread {
    account: usize,
    /// The one the conversation is with.
    with: (&'static str, &'static str),
    subject: &'static str,
    labels: &'static [&'static str],
    /// Whether I wrote it, what it says, and how many minutes ago.
    messages: &'static [(bool, &'static str, i64)],
    unread: bool,
    starred: bool,
    snoozed: bool,
    attachment: Option<&'static str>,
    /// A designed mail's colour; plain text when none.
    designed: Option<&'static str>,
    /// Sent by a machine or to a list.
    bulk: bool,
    /// How to leave its list: a one-click URL, or only a web page.
    unsubscribe: Option<(&'static str, bool)>,
}

const INBOX: Thread = Thread {
    account: 0,
    with: ("", ""),
    subject: "",
    labels: &[role::INBOX],
    messages: &[],
    unread: false,
    starred: false,
    snoozed: false,
    attachment: None,
    designed: None,
    bulk: false,
    unsubscribe: None,
};

const THREADS: &[Thread] = &[
    Thread {
        account: 1,
        with: ("Priya Shah", "priya@acme.example"),
        subject: "Launch checklist for Thursday",
        messages: &[
            (
                false,
                "Here is the checklist for Thursday. Can you take the release notes and the status page?",
                3 * HOUR,
            ),
            (true, "Yes, both are mine. I'll have the notes in the doc by tomorrow noon.", 2 * HOUR),
            (false, "Perfect. I moved the go/no-go to 9:30 so we have time to fix anything that comes up.", 12),
        ],
        unread: true,
        attachment: Some("Launch checklist.pdf"),
        ..INBOX
    },
    Thread {
        with: ("The Morning Orbit", "hello@orbit.example"),
        subject: "Three launches and a comet you can see this weekend",
        messages: &[(false, "Look up after ten on Saturday: the comet sits low in the west, just under the moon.", 35)],
        unread: true,
        designed: Some("#1d4ed8"),
        bulk: true,
        unsubscribe: Some(("https://orbit.example/unsubscribe", true)),
        ..INBOX
    },
    Thread {
        account: 1,
        with: ("Tracker", "notifications@tracker.example"),
        subject: "3 issues were assigned to you",
        messages: &[(false, "Status page copy, Release notes for 4.2, and Broken link in the welcome mail.", 50)],
        unread: true,
        bulk: true,
        ..INBOX
    },
    Thread {
        with: ("Mira Patel", "mira@hey.example"),
        subject: "Dinner on Saturday?",
        messages: &[(
            false,
            "We're making the dumplings again. Come around seven if you can, and bring the good bread.",
            95,
        )],
        unread: true,
        ..INBOX
    },
    Thread {
        account: 1,
        with: ("Tom Becker", "tom@acme.example"),
        subject: "Q4 budget",
        messages: &[
            (
                false,
                "The first draft of the Q4 budget is in the shared folder. Travel is the line I'm least sure about.",
                7 * HOUR,
            ),
            (true, "Travel looks right to me. I'd add a little for the offsite in November.", 3 * HOUR + 20),
        ],
        starred: true,
        ..INBOX
    },
    Thread {
        with: ("Northline Rail", "tickets@northline.example"),
        subject: "Your tickets for Friday",
        labels: &[role::INBOX, "travel"],
        messages: &[(
            false,
            "Your tickets are attached. Coach C, seats 41 and 42, leaving at 08:12 from platform 3.",
            5 * HOUR,
        )],
        starred: true,
        attachment: Some("Tickets.pdf"),
        ..INBOX
    },
    Thread {
        account: 1,
        with: ("Calendar", "calendar@acme.example"),
        subject: "Invitation: Design review, Wednesday 2pm",
        messages: &[(
            false,
            "Priya Shah invited you to Design review on Wednesday from 2pm to 3pm in the Blue Room.",
            6 * HOUR,
        )],
        unread: true,
        ..INBOX
    },
    Thread {
        with: ("Jonas Weber", "jonas@weber.example"),
        subject: "Photos from the hike",
        messages: &[
            (
                false,
                "Finally sorted the photos. The one at the ridge is my favourite, even with the fog.",
                DAY + 4 * HOUR,
            ),
            (true, "These are great! Can you send the one by the lake in full size?", DAY + 2 * HOUR),
            (false, "Attached. Same time next month?", DAY),
        ],
        attachment: Some("Lake.jpg"),
        ..INBOX
    },
    Thread {
        account: 1,
        with: ("Ana Costa", "ana@clientco.example"),
        subject: "Feedback on the new onboarding",
        labels: &[role::INBOX, "clients"],
        messages: &[(
            false,
            "We tried the new onboarding with five people this week. Four finished it without help, which is a first.",
            DAY + 3 * HOUR,
        )],
        ..INBOX
    },
    Thread {
        with: ("Fernhill Library", "desk@fernhill.example"),
        subject: "A book you reserved is ready",
        messages: &[(false, "The Overstory is waiting for you at the front desk until next Thursday.", DAY + 6 * HOUR)],
        ..INBOX
    },
    Thread {
        with: ("Kettle & Co.", "news@kettle.example"),
        subject: "Autumn blends are here",
        messages: &[(
            false,
            "Smoked pear, roasted chestnut and a very quiet chamomile. Free shipping until Sunday.",
            2 * DAY,
        )],
        designed: Some("#b45309"),
        bulk: true,
        unsubscribe: Some(("https://kettle.example/preferences", false)),
        ..INBOX
    },
    Thread {
        account: 1,
        with: ("Ben Ito", "ben@acme.example"),
        subject: "Lunch?",
        messages: &[(false, "The new noodle place opened downstairs. Tomorrow at 12:30?", 2 * DAY + 2 * HOUR)],
        ..INBOX
    },
    Thread {
        account: 1,
        with: ("Status Weekly", "digest@status.example"),
        subject: "This week in infrastructure",
        messages: &[(
            false,
            "Zero incidents, two faster deploys and one very long post-mortem worth reading.",
            2 * DAY + 5 * HOUR,
        )],
        designed: Some("#0f766e"),
        bulk: true,
        unsubscribe: Some(("https://status.example/unsubscribe", true)),
        ..INBOX
    },
    Thread {
        with: ("Mum", "mum@example.com"),
        subject: "Call me when you're free",
        messages: &[(false, "Nothing urgent, just want to hear about the new job. Sunday afternoon?", 3 * DAY)],
        starred: true,
        ..INBOX
    },
    Thread {
        with: ("Harbor Bank", "statements@harbor.example"),
        subject: "Your statement for September is ready",
        messages: &[(false, "Your statement for September is ready to view in the app.", 4 * DAY)],
        bulk: true,
        ..INBOX
    },
    Thread {
        account: 1,
        with: ("Payroll", "payroll@acme.example"),
        subject: "Your payslip for September",
        messages: &[(false, "Your payslip for September is attached.", 5 * DAY)],
        attachment: Some("Payslip September.pdf"),
        ..INBOX
    },
    Thread {
        with: ("Dr. Lena Ortiz", "office@ortiz.example"),
        subject: "Appointment reminder",
        labels: &[],
        messages: &[(
            false,
            "A reminder of your appointment on Monday at 9:00. Please arrive ten minutes early.",
            6 * HOUR,
        )],
        snoozed: true,
        ..INBOX
    },
    Thread {
        with: ("Jonas Weber", "jonas@weber.example"),
        subject: "Spare tent",
        labels: &[role::SENT],
        messages: &[(true, "I still have the spare tent if you want it for the trip. It's yours.", 2 * DAY + 3 * HOUR)],
        ..INBOX
    },
    Thread {
        with: ("Pine & Pixel", "orders@pinepixel.example"),
        subject: "Your order has shipped",
        labels: &[],
        messages: &[(false, "Your order is on its way and should arrive on Wednesday.", 6 * DAY)],
        ..INBOX
    },
    Thread {
        with: ("Casa Azul", "stay@casaazul.example"),
        subject: "Your stay in Lisbon",
        labels: &["travel"],
        messages: &[(
            false,
            "Check-in is from 3pm. The key is in the box by the blue door; the code is 4821.",
            8 * DAY,
        )],
        ..INBOX
    },
    Thread {
        account: 1,
        with: ("Priya Shah", "priya@acme.example"),
        subject: "Offsite notes",
        labels: &[],
        messages: &[(
            false,
            "Notes from the offsite are in the wiki. The short version: fewer meetings, more demos.",
            9 * DAY,
        )],
        ..INBOX
    },
    Thread {
        with: ("Prize Desk", "winner@prizes.example"),
        subject: "You may have already won",
        labels: &[role::TRASH],
        messages: &[(false, "Claim your prize today!", 3 * DAY)],
        ..INBOX
    },
];

/// Empties the local copy and fills it with the demo's mail, dated around `now`.
pub fn fill(store: &mut Store, now: i64) -> Result<()> {
    store.clear()?;
    let accounts: Vec<Account> = ACCOUNTS
        .iter()
        .enumerate()
        .map(|(index, (id, address, provider, color))| Account {
            id: id.to_string(),
            provider: *provider,
            address: address.to_string(),
            status: "ready".into(),
            color: color.to_string(),
            identities: IDENTITIES[index]
                .iter()
                .map(|(email, signature)| Identity {
                    name: Some("Sam Lee".into()),
                    email: email.to_string(),
                    signature: signature.map(String::from),
                })
                .collect(),
            deleted: false,
            rev: 1,
        })
        .collect();
    let labels: Vec<Label> = LABELS
        .iter()
        .map(|(id, account, name)| Label {
            id: id.to_string(),
            account_id: ACCOUNTS[*account].0.to_string(),
            name: name.to_string(),
            deleted: false,
            rev: 1,
        })
        .collect();

    let mut messages = Vec::new();
    let mut bodies = Vec::new();
    for (index, thread) in THREADS.iter().enumerate() {
        let (account_id, address, ..) = ACCOUNTS[thread.account];
        let me = Address::new(None, address);
        let them = Address::new(Some(thread.with.0), thread.with.1);
        for (position, (mine, text, ago)) in thread.messages.iter().enumerate() {
            let id = format!("demo-{index}-{position}");
            let last = position + 1 == thread.messages.len();
            let (from, to) = if *mine { (me.clone(), them.clone()) } else { (them.clone(), me.clone()) };
            let attachments = match thread.attachment {
                Some(name) if !mine && (last || position == 0) => {
                    vec![Attachment { name: name.into(), mime: "application/octet-stream".into(), size: 184_000 }]
                }
                _ => Vec::new(),
            };
            let mut labels: Vec<String> = thread.labels.iter().map(|label| label.to_string()).collect();
            if *mine && !labels.iter().any(|label| label == role::SENT) {
                labels.push(role::SENT.into());
            }
            messages.push(Message {
                id: id.clone(),
                account_id: account_id.into(),
                thread_id: format!("thread-{index}"),
                from,
                recipients: Recipients { to: vec![to], ..Default::default() },
                subject: thread.subject.into(),
                snippet: text.to_string(),
                date: now - ago * 60_000,
                unread: thread.unread && last,
                starred: thread.starred && last,
                labels,
                attachments,
                message_id: Some(format!("<{id}@demo>")),
                in_reply_to: None,
                references: Vec::new(),
                snoozed_until: thread.snoozed.then_some(now + 15 * HOUR * 60_000),
                bulk: thread.bulk,
                unsubscribe: thread.unsubscribe.map(|(url, one_click)| Unsubscribe {
                    url: Some(url.into()),
                    mailto: None,
                    one_click,
                }),
                deleted: false,
                rev: 1,
            });
            let body = match thread.designed {
                Some(colour) => Body { html: Some(designed(thread.with, colour, thread.subject, text)), text: None },
                None => Body { html: None, text: Some(format!("{text}\n\n{}", from_name(*mine, thread.with.0))) },
            };
            bodies.push((id, body));
        }
    }
    let preferences = [
        ("split_inbox", json!(true)),
        ("split:team", json!({ "name": "Team", "from": ["@acme.example"], "label": null, "order": 0 })),
        ("signature:demo-home", json!("Sam")),
        ("snippet:thanks", json!({ "name": "Thanks", "text": "Thanks {first_name}!" })),
    ]
    .map(|(key, value)| Preference { key: key.into(), value, deleted: false, rev: 1 });
    let drafts = [SavedDraft {
        id: "demo-draft".into(),
        draft: Draft {
            account_id: ACCOUNTS[1].0.into(),
            to: vec![Address::new(Some("Priya Shah"), "priya@acme.example")],
            subject: "Release notes for 4.2".into(),
            text: "Here is a first pass at the notes. The search part still needs a screenshot.".into(),
            ..Default::default()
        },
        updated: now - 40 * 60_000,
        deleted: false,
        rev: 1,
    }];
    store.apply_changes(Batch {
        accounts: &accounts,
        labels: &labels,
        messages: &messages,
        preferences: &preferences,
        drafts: &drafts,
        cursor: Some(0),
    })?;
    for (id, body) in bodies {
        store.save_body(&id, &body)?;
    }
    store.set_meta("token", Some("demo"))?;
    Ok(())
}

fn from_name(mine: bool, them: &str) -> &str {
    if mine { "Sam" } else { them.split(' ').next().unwrap_or(them) }
}

/// A newsletter as senders make them: a table, their colours, and the open-tracking pixel from
/// their domain that the `remote_images` preference keeps out.
fn designed((brand, address): (&str, &str), colour: &str, headline: &str, text: &str) -> String {
    let domain = address.rsplit('@').next().unwrap_or_default();
    format!(
        r##"<html><body style="margin:0;background:#f4f4f5">
<img src="https://{domain}/open.gif" width="1" height="1" alt="">
<table width="100%" cellpadding="0" cellspacing="0" bgcolor="#f4f4f5"><tr><td align="center" style="padding:24px">
<table width="560" cellpadding="0" cellspacing="0" bgcolor="#ffffff" style="border-radius:8px">
<tr><td style="padding:32px;font-family:Helvetica,Arial,sans-serif;color:#222">
<p style="margin:0 0 20px;font-size:13px;font-weight:bold;letter-spacing:1px;color:{colour}">{brand}</p>
<h1 style="margin:0 0 12px;font-size:24px;line-height:30px">{headline}</h1>
<p style="font-size:15px;line-height:22px">{text}</p>
<p style="margin-top:24px"><a href="https://example.com" style="background:{colour};color:#fff;padding:10px 18px;border-radius:6px;text-decoration:none">Read more</a></p>
</td></tr></table></td></tr></table></body></html>"##
    )
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    #[test]
    fn fills_every_mailbox_the_demo_has() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("mail.db")).unwrap();
        fill(&mut store, mail_protocol::now_ms()).unwrap();
        fill(&mut store, mail_protocol::now_ms()).unwrap();

        let page = |mailbox: &str| store.thread_page(mailbox, 0, 100, None, &Utc::now()).unwrap();
        let count = |mailbox: &str| page(mailbox).total;
        assert_eq!(count("inbox"), 6, "a split inbox shows Important");
        let splits: Vec<(String, u32)> = page("inbox").splits.into_iter().map(|tab| (tab.name, tab.total)).collect();
        assert_eq!(splits, [("Important".into(), 6), ("Team".into(), 5), ("Other".into(), 5)]);
        assert_eq!(count("drafts"), 1);
        assert_eq!(page("drafts").rows[0].draft_id.as_deref(), Some("demo-draft"));
        assert_eq!(page("demo-work/inbox").splits.iter().map(|tab| tab.total).sum::<u32>(), 8);
        assert_eq!(count("snoozed"), 1);
        assert_eq!(count("sent"), 4);
        assert_eq!(count("demo-home/label/travel"), 2);
        assert_eq!(count("trash"), 1);
        assert!(store.body("demo-1-0").unwrap().html.is_some());
        assert_eq!(store.meta("token").as_deref(), Some("demo"));
    }
}
