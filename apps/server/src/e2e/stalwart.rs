//! The whole system on a real JMAP server: Stalwart, Postgres, the server and the client core.
//! Runs when STALWART_URL (and STALWART_ADMIN, `admin:password`) name a Stalwart server;
//! `compose.yaml` starts one.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mail_core::api::{Command, Config, Event};
use serde_json::{Value, json};
use sqlx::PgPool;

use super::Server;
use crate::providers::jmap::Jmap;
use crate::seed;

struct Driver {
    handle: mail_core::core::Handle,
    events: Arc<Mutex<Vec<Event>>>,
    next: u64,
    _data: tempfile::TempDir,
}

impl Driver {
    fn start(server: &str) -> Self {
        let data = tempfile::tempdir().unwrap();
        let events: Arc<Mutex<Vec<Event>>> = Arc::default();
        let sink = events.clone();
        let config =
            Config { data_dir: data.path().display().to_string(), server_url: Some(server.to_string()), demo: false };
        let handle = mail_core::core::start(config, Arc::new(move |event| sink.lock().unwrap().push(event))).unwrap();
        Self { handle, events, next: 0, _data: data }
    }

    async fn call(&mut self, command: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.handle.send(id, serde_json::from_value::<Command>(command).unwrap());
        for _ in 0..600 {
            let reply = self.events.lock().unwrap().iter().find_map(|event| match event {
                Event::Reply { id: replied, ok, value } if *replied == id => Some((*ok, value.clone())),
                _ => None,
            });
            if let Some((ok, value)) = reply {
                assert!(ok, "{value}");
                return value;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("no reply to command {id}");
    }
}

async fn eventually<T>(what: &str, mut check: impl AsyncFnMut() -> Option<T>) -> T {
    for _ in 0..120 {
        if let Some(found) = check().await {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("timed out waiting for {what}");
}

const PASSWORD: &str = "quiet-harbor-lantern-42";

/// A new Stalwart user with `count` seeded messages: the server's address, the user's and a
/// connection. None when STALWART_URL is not set.
async fn stalwart_user(count: usize) -> Option<(String, String, Jmap)> {
    let Ok(stalwart) = std::env::var("STALWART_URL") else {
        eprintln!("skipped: STALWART_URL is not set");
        return None;
    };
    let admin = std::env::var("STALWART_ADMIN").unwrap_or_else(|_| "admin:adminpass".into());
    let (name, secret) = admin.split_once(':').unwrap();
    let admin = (name.to_string(), secret.to_string());
    let email = format!("e2e-{}@example.com", &crate::random_token()[..10]);

    mail_protocol::tls::install();
    let http = reqwest::Client::new();
    seed::create_user(&http, &stalwart, &admin, &email, PASSWORD).await.unwrap();
    let jmap = Jmap::open(&http, &stalwart, &email, PASSWORD).await.unwrap();
    seed::fill(&jmap, &email, count, chrono::Utc::now().timestamp()).await.unwrap();
    Some((stalwart, email, jmap))
}

async fn inbox_rows(core: &mut Driver) -> Vec<Value> {
    let page = core.call(json!({ "type": "threads", "mailbox": "inbox" })).await;
    page["rows"].as_array().cloned().unwrap_or_default()
}

#[sqlx::test]
async fn seeded_mail_reaches_the_core_and_archiving_reaches_stalwart(db: PgPool) {
    let Some((stalwart, email, jmap)) = stalwart_user(40).await else { return };

    let server = Server::start_with(db, |config| config.workers = true).await;
    let mut core = Driver::start(&server.base);
    core.call(json!({ "type": "add_jmap_account", "url": stalwart, "username": email, "password": PASSWORD })).await;

    let rows = eventually("the inbox in the core", async || {
        let rows = inbox_rows(&mut core).await;
        (!rows.is_empty()).then_some(rows)
    })
    .await;
    let first = rows[0].clone();
    assert!(first["senders"].as_str().unwrap().contains("me ("), "{first}");

    let thread = first["id"].as_str().unwrap().to_string();
    let opened = core.call(json!({ "type": "open_thread", "thread": thread })).await;
    assert!(opened["messages"].as_array().unwrap().len() > 1);
    eventually("the bodies", async || {
        let opened = core.call(json!({ "type": "open_thread", "thread": thread })).await;
        opened["messages"].as_array().unwrap().iter().all(|message| message["html"].is_string()).then_some(())
    })
    .await;

    core.call(json!({ "type": "act", "action": "archive", "threads": [thread] })).await;
    let inbox = core.call(json!({ "type": "threads", "mailbox": "inbox" })).await;
    assert!(inbox["rows"].as_array().unwrap().iter().all(|row| row["id"] != first["id"]), "archived at once");

    let provider_thread = thread.split_once(':').unwrap().1.to_string();
    eventually("Stalwart to archive the thread", async || {
        let answers = jmap
            .call(
                &["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
                vec![
                    ("Thread/get", json!({ "ids": [provider_thread] })),
                    ("Email/get", json!({ "#ids": { "resultOf": "0", "name": "Thread/get", "path": "/list/*/emailIds" }, "properties": ["mailboxIds"] })),
                    ("Mailbox/get", json!({ "ids": null, "properties": ["id", "role"] })),
                ],
            )
            .await
            .unwrap();
        let inbox = answers[2]["list"].as_array().unwrap().iter().find(|mailbox| mailbox["role"] == "inbox").unwrap()["id"].clone();
        let inbox = inbox.as_str().unwrap();
        let emails = answers[1]["list"].as_array().unwrap();
        emails.iter().all(|email| email["mailboxIds"][inbox].is_null()).then_some(())
    })
    .await;
}

#[sqlx::test]
async fn new_mail_at_stalwart_is_pushed_without_waiting_for_a_poll(db: PgPool) {
    let Some((stalwart, email, jmap)) = stalwart_user(5).await else { return };
    let server = Server::start_with(db, |config| {
        config.workers = true;
        config.poll_interval = Duration::from_secs(3600);
    })
    .await;
    let mut core = Driver::start(&server.base);
    core.call(json!({ "type": "add_jmap_account", "url": stalwart, "username": email, "password": PASSWORD })).await;
    eventually("the inbox in the core", async || (!inbox_rows(&mut core).await.is_empty()).then_some(())).await;

    let mailboxes = jmap
        .call(
            &["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
            vec![("Mailbox/get", json!({ "ids": null, "properties": ["id", "role"] }))],
        )
        .await
        .unwrap();
    let inbox =
        mailboxes[0]["list"].as_array().unwrap().iter().find(|mailbox| mailbox["role"] == "inbox").unwrap()["id"]
            .clone();
    let raw = mail_builder::MessageBuilder::new()
        .from(("Ada Lovelace", "ada@example.com"))
        .to(email.as_str())
        .subject("Pushed, not polled")
        .text_body("Hello.")
        .write_to_vec()
        .unwrap();
    let blob = jmap.upload(raw).await.unwrap();
    jmap.import(vec![json!({ "blobId": blob, "mailboxIds": { inbox.as_str().unwrap(): true } })]).await.unwrap();

    eventually("the new mail in the core", async || {
        inbox_rows(&mut core).await.iter().any(|row| row["subject"] == "Pushed, not polled").then_some(())
    })
    .await;
}

#[sqlx::test]
async fn a_refresh_answers_once_the_server_has_synced(db: PgPool) {
    let Some((stalwart, email, jmap)) = stalwart_user(5).await else { return };
    let server = Server::start_with(db, |config| {
        config.workers = true;
        config.poll_interval = Duration::from_secs(3600);
    })
    .await;
    let mut core = Driver::start(&server.base);
    core.call(json!({ "type": "add_jmap_account", "url": stalwart, "username": email, "password": PASSWORD })).await;
    eventually("the inbox in the core", async || (!inbox_rows(&mut core).await.is_empty()).then_some(())).await;

    let mailboxes = jmap
        .call(
            &["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"],
            vec![("Mailbox/get", json!({ "ids": null, "properties": ["id", "role"] }))],
        )
        .await
        .unwrap();
    let inbox =
        mailboxes[0]["list"].as_array().unwrap().iter().find(|mailbox| mailbox["role"] == "inbox").unwrap()["id"]
            .clone();
    let raw = mail_builder::MessageBuilder::new()
        .from(("Ada Lovelace", "ada@example.com"))
        .to(email.as_str())
        .subject("Pulled down for")
        .text_body("Hello.")
        .write_to_vec()
        .unwrap();
    let blob = jmap.upload(raw).await.unwrap();
    jmap.import(vec![json!({ "blobId": blob, "mailboxIds": { inbox.as_str().unwrap(): true } })]).await.unwrap();

    let started = std::time::Instant::now();
    core.call(json!({ "type": "refresh" })).await;
    assert!(started.elapsed() < Duration::from_secs(10), "the server answered, not the core's timeout");
    assert!(inbox_rows(&mut core).await.iter().any(|row| row["subject"] == "Pulled down for"), "there when answered");
}

#[sqlx::test]
async fn a_refresh_answers_at_once_when_no_worker_runs(db: PgPool) {
    let server = Server::start(db).await;
    let mut core = Driver::start(&server.base);
    core.call(json!({ "type": "dev_login", "email": "pull@example.com" })).await;
    let started = std::time::Instant::now();
    core.call(json!({ "type": "refresh" })).await;
    assert!(started.elapsed() < Duration::from_secs(5), "the server answered, not the core's timeout");
}
