use mail_protocol::{Address, Attachment, Provider, Recipients};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{self, NewAccount, RemoteMessage};
use crate::search;

const DAY: i64 = 24 * 60 * 60 * 1000;

#[sqlx::test]
async fn search_reads_the_operators(db: PgPool) {
    let user = db::create_user(&db).await.unwrap();
    let account = NewAccount {
        user_id: user,
        provider: Provider::Jmap,
        address: "me@example.com",
        login: "l",
        credentials: vec![],
    };
    let account_id = db::upsert_account(&db, account).await.unwrap();
    let account = db::account(&db, account_id).await.unwrap().unwrap();
    let work = db::sync_labels(&db, &account, &[("w".into(), "Work".into())]).await.unwrap()["w"];
    let now = chrono::Utc::now().timestamp_millis();
    let messages = [
        RemoteMessage {
            provider_id: "plan".into(),
            from: Address::new(Some("Ann Lee"), "ann.lee@acme.com"),
            recipients: Recipients { to: vec![Address::new(Some("Bob"), "bob@example.com")], ..Default::default() },
            subject: "Weekly report for March".into(),
            snippet: "the budget is attached".into(),
            date: now - DAY,
            unread: true,
            labels: vec!["inbox".into(), work.to_string()],
            attachments: vec![Attachment { name: "a.pdf".into(), mime: "application/pdf".into(), size: 1 }],
            ..Default::default()
        },
        RemoteMessage {
            provider_id: "lunch".into(),
            from: Address::new(Some("Cy"), "cy@example.com"),
            subject: "Lunch on Friday".into(),
            snippet: "a report of where to eat".into(),
            date: now - 400 * DAY,
            starred: true,
            labels: vec!["sent".into()],
            ..Default::default()
        },
    ];
    db::upsert_messages(&db, &account, &messages).await.unwrap();
    let ids: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, provider_id FROM messages").fetch_all(&db).await.unwrap();

    let found = async |query: &str| -> Vec<String> {
        let found = search::run(&db, user, query).await.unwrap();
        let mut names: Vec<String> =
            found.iter().map(|id| ids.iter().find(|(known, _)| known == id).unwrap().1.clone()).collect();
        names.sort();
        names
    };
    assert_eq!(found("report").await, ["lunch", "plan"]);
    assert_eq!(found("from:ann").await, ["plan"]);
    assert_eq!(found("from:acme.com").await, ["plan"]);
    assert_eq!(found("from:ann.lee@acme.com").await, ["plan"]);
    assert_eq!(found("from:bob").await, Vec::<String>::new());
    assert_eq!(found("to:bob").await, ["plan"]);
    assert_eq!(found("subject:report").await, ["plan"]);
    assert_eq!(found("subject:\"weekly report\"").await, ["plan"]);
    assert_eq!(found("\"weekly report\"").await, ["plan"]);
    assert_eq!(found("report -budget").await, ["lunch"]);
    assert_eq!(found("budget OR friday").await, ["lunch", "plan"]);
    assert_eq!(found("has:attachment").await, ["plan"]);
    assert_eq!(found("is:unread").await, ["plan"]);
    assert_eq!(found("is:starred").await, ["lunch"]);
    assert_eq!(found("in:sent").await, ["lunch"]);
    assert_eq!(found("in:archive").await, ["lunch"]);
    assert_eq!(found("label:work report").await, ["plan"]);
    assert_eq!(found("newer_than:1m").await, ["plan"]);
    assert_eq!(found("older_than:1y").await, ["lunch"]);
    assert_eq!(found("after:2000/01/01 before:2000/02/01").await, Vec::<String>::new());
    assert_eq!(found("").await, Vec::<String>::new());

    // The words of a fetched body are searchable too, and stay so when the provider says more.
    let plan = db::message(&db, user, ids.iter().find(|(_, name)| name == "plan").unwrap().0).await.unwrap().unwrap();
    db::save_body(&db, &plan, user, &Default::default(), &plan.attachments.0, "quarterly forecast").await.unwrap();
    let mut read = messages[0].clone();
    read.unread = false;
    db::upsert_messages(&db, &account, &[read]).await.unwrap();
    assert_eq!(found("forecast").await, ["plan"]);
}

#[sqlx::test]
async fn mail_from_before_the_weights_is_searchable_by_part_once_rebuilt(db: PgPool) {
    let user = db::create_user(&db).await.unwrap();
    let account = NewAccount {
        user_id: user,
        provider: Provider::Jmap,
        address: "me@example.com",
        login: "l",
        credentials: vec![],
    };
    let account_id = db::upsert_account(&db, account).await.unwrap();
    let account = db::account(&db, account_id).await.unwrap().unwrap();
    let messages: Vec<RemoteMessage> = (0..5)
        .map(|index| RemoteMessage {
            provider_id: format!("m{index}"),
            from: Address::new(None, "ann@acme.com"),
            subject: "Hello".into(),
            ..Default::default()
        })
        .collect();
    db::upsert_messages(&db, &account, &messages).await.unwrap();
    sqlx::query("UPDATE messages SET search = to_tsvector('simple', subject || ' ' || from_email)")
        .execute(&db)
        .await
        .unwrap();
    assert!(search::run(&db, user, "from:ann").await.unwrap().is_empty());

    let mut batches = 0;
    while db::backfill_search(&db, 2).await.unwrap() {
        batches += 1;
    }
    assert_eq!(batches, 3);
    assert_eq!(search::run(&db, user, "from:ann").await.unwrap().len(), 5);
    assert!(!db::backfill_search(&db, 2).await.unwrap(), "done for good");
}
