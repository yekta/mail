use std::time::Duration;

use mail_protocol::{Address, Draft, DraftAttachment, Op};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::Server;
use crate::db::{self, RemoteMessage};
use crate::ops::{self, MISSING_FORWARD, MISSING_UPLOAD, Outcome};

fn inbox(provider_id: &str) -> RemoteMessage {
    RemoteMessage {
        provider_id: provider_id.into(),
        thread_id: "t1".into(),
        from: Address::new(Some("Cy"), "cy@example.com"),
        subject: "Plan".into(),
        date: 1_700_000_000_000,
        labels: vec!["inbox".into()],
        ..Default::default()
    }
}

async fn upload(server: &Server, token: &str, name: &str, bytes: Vec<u8>) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{}/api/uploads", server.base))
        .bearer_auth(token)
        .header("x-file-name", name)
        .header("content-type", "application/pdf")
        .body(bytes)
        .send()
        .await
        .unwrap()
}

fn failed_with(outcome: Outcome) -> Option<String> {
    match outcome {
        Outcome::Done { ok: false, error } => error,
        _ => None,
    }
}

async fn outgoing_status(server: &Server, op_id: &str) -> (String, Option<String>) {
    for _ in 0..100 {
        let row: (String, Option<String>) = sqlx::query_as("SELECT status, error FROM outgoing WHERE id = $1")
            .bind(op_id)
            .fetch_one(&server.state.db)
            .await
            .unwrap();
        if row.0 == "sent" || row.0 == "failed" {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{op_id} was never sent");
}

#[sqlx::test]
async fn an_attachment_comes_from_the_provider_to_its_owner_only(db: PgPool) {
    let server = Server::start(db.clone()).await;
    let (ann, _, account) = server.gmail_user("ann@gmail.com").await;
    let (bob, _, _) = server.gmail_user("bob@gmail.com").await;
    db::upsert_messages(&db, &account, &[inbox("m1")]).await.unwrap();
    let id: Uuid = sqlx::query_scalar("SELECT id FROM messages").fetch_one(&db).await.unwrap();
    let path = format!("/api/messages/{id}/attachments/0");

    let response = server.get(&path, Some(&ann)).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "application/pdf");
    assert!(response.headers()["content-disposition"].to_str().unwrap().contains("filename=\"plan.pdf\""));
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"%PDF-1.4");

    assert_eq!(server.get(&path, Some(&bob)).await.status(), 404);
    assert_eq!(server.get(&path, None).await.status(), 401);
    assert_eq!(server.get(&format!("/api/messages/{id}/attachments/1"), Some(&ann)).await.status(), 404);
}

#[sqlx::test]
async fn an_upload_goes_out_only_with_its_owners_mail(db: PgPool) {
    let server = Server::start(db.clone()).await;
    let (ann, ann_id, ann_account) = server.gmail_user("ann@gmail.com").await;
    let (_, bob_id, bob_account) = server.gmail_user("bob@gmail.com").await;
    db::upsert_messages(&db, &ann_account, &[inbox("m1")]).await.unwrap();
    let forwarded: Uuid = sqlx::query_scalar("SELECT id FROM messages").fetch_one(&db).await.unwrap();

    let too_big = upload(&server, &ann, "big.pdf", vec![0; crate::api::UPLOAD_LIMIT + 1]).await;
    assert_eq!(too_big.status(), 413);
    let response = upload(&server, &ann, "Notes%20%C3%A9t%C3%A9.pdf", b"%PDF-notes".to_vec()).await;
    assert_eq!(response.status(), 200);
    let upload_id = response.json::<Value>().await.unwrap()["id"].as_str().unwrap().to_string();
    let name: String = sqlx::query_scalar("SELECT name FROM uploads").fetch_one(&db).await.unwrap();
    assert_eq!(name, "Notes été.pdf");

    let draft = |account: &db::AccountRow| Draft {
        account_id: account.id.to_string(),
        to: vec![Address::new(None, "cy@example.com")],
        subject: "Notes".into(),
        text: "Attached.".into(),
        attachments: vec![DraftAttachment {
            name: "Notes été.pdf".into(),
            mime: "application/pdf".into(),
            size: 10,
            upload: Some(upload_id.clone()),
            path: None,
        }],
        ..Default::default()
    };
    let send = |draft: Draft| Op::Send { draft: Box::new(draft), send_at: 0, remind_at: None };

    // Bob can't send Ann's upload, nor forward her message's attachments.
    let outcome = ops::apply(&server.state, bob_id, "bob-1", send(draft(&bob_account))).await.unwrap();
    assert_eq!(failed_with(outcome).as_deref(), Some(MISSING_UPLOAD));
    let forward =
        Draft { forward_attachments_of: Some(forwarded.to_string()), attachments: vec![], ..draft(&bob_account) };
    let outcome = ops::apply(&server.state, bob_id, "bob-2", send(forward)).await.unwrap();
    assert_eq!(failed_with(outcome).as_deref(), Some(MISSING_FORWARD));

    // Ann's goes out as her named identity, with her file and the forwarded one; the upload is gone.
    let mine = Draft { forward_attachments_of: Some(forwarded.to_string()), ..draft(&ann_account) };
    let outcome = ops::apply(&server.state, ann_id, "ann-1", send(mine)).await.unwrap();
    assert!(matches!(outcome, Outcome::Waiting));
    assert_eq!(outgoing_status(&server, "ann-1").await, ("sent".to_string(), None));
    let raw = &server.sent()[0];
    assert!(raw.contains("From: \"Ann Lee\" <ann@gmail.com>"), "{raw}");
    let files: Vec<String> = crate::mime::files(raw.as_bytes()).into_iter().map(|file| file.name).collect();
    assert_eq!(files, ["Notes été.pdf", "plan.pdf"]);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM uploads").fetch_one(&db).await.unwrap();
    assert_eq!(left, 0);
}

#[sqlx::test]
async fn sent_mail_with_a_reminder_comes_back_to_the_inbox(db: PgPool) {
    let server = Server::start(db.clone()).await;
    let (_, ann_id, account) = server.gmail_user("ann@gmail.com").await;
    let draft = Draft {
        account_id: account.id.to_string(),
        to: vec![Address::new(None, "cy@example.com")],
        subject: "Any news?".into(),
        text: "Let me know.".into(),
        from: Some(Address::new(None, "ann@work.com")),
        ..Default::default()
    };
    let remind_at = chrono::Utc::now().timestamp_millis() + 1500;
    let send = Op::Send { draft: Box::new(draft), send_at: 0, remind_at: Some(remind_at) };
    ops::apply(&server.state, ann_id, "send-1", send).await.unwrap();
    assert_eq!(outgoing_status(&server, "send-1").await.0, "sent");
    assert!(server.sent()[0].contains("From: \"Ann at work\" <ann@work.com>"));

    // The sent message arrives with the next sync, snoozed until the reminder.
    let sent = RemoteMessage { provider_id: "sent-1".into(), labels: vec!["sent".into()], ..inbox("sent-1") };
    db::upsert_messages(&db, &account, &[sent]).await.unwrap();
    let snoozed: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT snoozed_until FROM messages").fetch_one(&db).await.unwrap();
    assert_eq!(snoozed.map(db::millis), Some(remind_at));

    // With no reply it comes back to the inbox, and Gmail is told.
    for _ in 0..50 {
        let labels: Vec<String> = sqlx::query_scalar("SELECT labels FROM messages").fetch_one(&db).await.unwrap();
        if labels.contains(&"inbox".to_string()) {
            let op: Value = sqlx::query_scalar("SELECT op FROM provider_ops").fetch_one(&db).await.unwrap();
            assert_eq!(op["kind"], "move_to_inbox");
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the reminder never came back");
}

#[sqlx::test]
async fn a_send_waits_out_a_spent_gmail_quota(db: PgPool) {
    let server = Server::start(db.clone()).await;
    let (_, _, account) = server.gmail_user("ann@gmail.com").await;
    server.refuse_sends(1);
    let draft = Draft {
        account_id: account.id.to_string(),
        to: vec![Address::new(None, "cy@example.com")],
        subject: "Lunch".into(),
        text: "Noon?".into(),
        ..Default::default()
    };
    crate::scheduler::deliver(&server.state, &account, &draft).await.unwrap();
    assert_eq!(server.sent().len(), 1);
}

#[sqlx::test]
async fn uploads_a_waiting_send_or_a_saved_draft_uses_outlive_the_week(db: PgPool) {
    let server = Server::start(db.clone()).await;
    let (ann, ann_id, account) = server.gmail_user("ann@gmail.com").await;
    let mut ids = Vec::new();
    for name in ["sending.pdf", "draft.pdf", "forgotten.pdf"] {
        let response = upload(&server, &ann, name, b"%PDF".to_vec()).await;
        ids.push(response.json::<Value>().await.unwrap()["id"].as_str().unwrap().to_string());
    }
    sqlx::query("UPDATE uploads SET created_at = now() - interval '8 days'").execute(&db).await.unwrap();
    let draft = |upload: &str| Draft {
        account_id: account.id.to_string(),
        to: vec![Address::new(None, "cy@example.com")],
        attachments: vec![DraftAttachment { upload: Some(upload.into()), ..Default::default() }],
        ..Default::default()
    };
    let later = chrono::Utc::now().timestamp_millis() + 3_600_000;
    let send = Op::Send { draft: Box::new(draft(&ids[0])), send_at: later, remind_at: None };
    ops::apply(&server.state, ann_id, "later", send).await.unwrap();
    let save = Op::SaveDraft { draft_id: "d".into(), draft: Box::new(draft(&ids[1])) };
    ops::apply(&server.state, ann_id, "save", save).await.unwrap();

    db::cleanup(&db).await.unwrap();
    let mut left: Vec<String> = sqlx::query_scalar("SELECT name FROM uploads").fetch_all(&db).await.unwrap();
    left.sort();
    assert_eq!(left, ["draft.pdf", "sending.pdf"]);
}

#[sqlx::test]
async fn a_label_made_in_the_app_reaches_gmail_before_the_ops_that_use_it(db: PgPool) {
    let server = Server::start(db.clone()).await;
    let (_, ann_id, account) = server.gmail_user("ann@gmail.com").await;
    db::upsert_messages(&db, &account, &[inbox("m1")]).await.unwrap();
    let message: Uuid = sqlx::query_scalar("SELECT id FROM messages").fetch_one(&db).await.unwrap();
    let create = async |name: &str| {
        let label = Uuid::new_v4().to_string();
        let op = Op::CreateLabel { account_id: account.id.to_string(), label_id: label.clone(), name: name.into() };
        ops::apply(&server.state, ann_id, &format!("create-{name}"), op).await.unwrap();
        label
    };
    let trips = create("Trips").await;
    create("Taken").await;

    // The op goes through without the worker having made the label beforehand.
    let add = Op::AddLabel { ids: vec![message.to_string()], label: trips };
    ops::apply(&server.state, ann_id, "add", add).await.unwrap();
    let connection = crate::providers::Connection::open(&server.state, &account).await.unwrap();
    crate::workers::flush_ops(&server.state, &account, &connection).await.unwrap();
    assert_eq!(server.modified()[0]["addLabelIds"], serde_json::json!(["Label_Trips"]));
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM provider_ops").fetch_one(&db).await.unwrap();
    assert_eq!(pending, 0);

    // Gmail has Taken already, so it is deleted; Busy is kept for when Gmail answers.
    create("Busy").await;
    assert!(crate::workers::create_labels(&server.state, &account, &connection).await.is_err());
    let rows: Vec<(String, Option<String>, bool)> =
        sqlx::query_as("SELECT name, provider_id, deleted FROM labels ORDER BY name").fetch_all(&db).await.unwrap();
    assert_eq!(
        rows,
        [
            ("Busy".to_string(), None, false),
            ("Taken".to_string(), None, true),
            ("Trips".to_string(), Some("Label_Trips".to_string()), false)
        ]
    );
}
