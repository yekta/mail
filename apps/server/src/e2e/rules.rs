use mail_protocol::{Address, Op, Provider};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::Server;
use crate::db::{self, AccountRow, NewAccount, RemoteMessage};
use crate::ops;

fn mail(provider_id: &str, thread_id: &str, from: &str) -> RemoteMessage {
    RemoteMessage {
        provider_id: provider_id.into(),
        thread_id: thread_id.into(),
        from: Address::new(None, from),
        subject: provider_id.into(),
        date: 1_700_000_000_000,
        unread: true,
        labels: vec!["inbox".into()],
        ..Default::default()
    }
}

async fn labels_of(db: &PgPool, provider_id: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT labels FROM messages WHERE provider_id = $1")
        .bind(provider_id)
        .fetch_one(db)
        .await
        .unwrap()
}

async fn snoozed(db: &PgPool, provider_id: &str) -> bool {
    let until: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT snoozed_until FROM messages WHERE provider_id = $1")
            .bind(provider_id)
            .fetch_one(db)
            .await
            .unwrap();
    until.is_some()
}

async fn account(db: &PgPool) -> (Uuid, AccountRow) {
    let user = db::create_user(db).await.unwrap();
    let account = NewAccount {
        user_id: user,
        provider: Provider::Jmap,
        address: "me@example.com",
        login: "l",
        credentials: vec![],
    };
    let account_id = db::upsert_account(db, account).await.unwrap();
    (user, db::account(db, account_id).await.unwrap().unwrap())
}

#[sqlx::test]
async fn new_mail_from_the_blocked_is_trashed_and_in_muted_threads_archived(db: PgPool) {
    let server = Server::start(db.clone()).await;
    let (user, account) = account(&db).await;
    db::upsert_messages(&db, &account, &[mail("known", "t0", "ads@shop.com")]).await.unwrap();

    let preferences =
        ["blocked:ads@shop.com".to_string(), "blocked:@junk.com".to_string(), format!("muted:{}:t-muted", account.id)];
    for (index, key) in preferences.into_iter().enumerate() {
        let op = Op::SetPreference { key, value: Some(json!(true)) };
        ops::apply(&server.state, user, &format!("pref-{index}"), op).await.unwrap();
    }

    let mut known = mail("known", "t0", "ads@shop.com");
    known.unread = false;
    let new = [
        known,
        mail("ad", "t1", "ads@shop.com"),
        mail("junk", "t2", "anyone@junk.com"),
        mail("muted", "t-muted", "ann@example.com"),
        mail("person", "t3", "ann@example.com"),
    ];
    db::upsert_messages(&db, &account, &new).await.unwrap();

    assert_eq!(labels_of(&db, "known").await, ["inbox"], "mail the server had already is left alone");
    assert_eq!(labels_of(&db, "ad").await, ["trash"]);
    assert_eq!(labels_of(&db, "junk").await, ["trash"]);
    assert!(labels_of(&db, "muted").await.is_empty());
    assert_eq!(labels_of(&db, "person").await, ["inbox"]);
    let queued: Vec<String> =
        sqlx::query_scalar("SELECT op->>'kind' FROM provider_ops ORDER BY op->>'kind'").fetch_all(&db).await.unwrap();
    assert_eq!(queued, ["archive", "trash", "trash"]);
}

#[sqlx::test]
async fn a_reply_ends_the_snooze_of_its_thread(db: PgPool) {
    let server = Server::start(db.clone()).await;
    let (user, account) = account(&db).await;
    db::upsert_messages(&db, &account, &[mail("first", "t1", "ann@example.com")]).await.unwrap();
    let id: Uuid = sqlx::query_scalar("SELECT id FROM messages").fetch_one(&db).await.unwrap();
    let until = chrono::Utc::now().timestamp_millis() + 3_600_000;
    let snooze = Op::Snooze { ids: vec![id.to_string()], until };
    ops::apply(&server.state, user, "snooze", snooze).await.unwrap();

    // The user's own mail in the thread doesn't end it; someone else's does.
    db::upsert_messages(&db, &account, &[mail("mine", "t1", "me@example.com")]).await.unwrap();
    assert!(snoozed(&db, "first").await);
    db::upsert_messages(&db, &account, &[mail("reply", "t1", "ann@example.com")]).await.unwrap();
    assert!(!snoozed(&db, "first").await);
    assert_eq!(labels_of(&db, "reply").await, ["inbox"]);
}
