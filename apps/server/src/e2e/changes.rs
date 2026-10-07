use mail_protocol::wire::ServerMessage;
use mail_protocol::{Address, BATCH_SIZE, Op, Provider};
use sqlx::PgPool;

use super::Server;
use crate::db::{self, NewAccount, RemoteMessage};
use crate::ops;

fn remote(index: usize) -> RemoteMessage {
    RemoteMessage {
        provider_id: format!("p{index}"),
        thread_id: format!("t{}", index / 3),
        from: Address::new(Some("Ann"), "ann@example.com"),
        subject: format!("Message {index}"),
        date: 1_700_000_000_000 + index as i64,
        unread: true,
        labels: vec!["inbox".into()],
        ..Default::default()
    }
}

async fn changes(server: &Server, user: uuid::Uuid, cursor: i64) -> (usize, usize, i64, bool) {
    let ServerMessage::Changes { accounts, messages, cursor, more, .. } =
        crate::changes::read(&server.state.db, user, cursor).await.unwrap()
    else {
        panic!("not changes");
    };
    (accounts.len(), messages.len(), cursor, more)
}

#[sqlx::test]
async fn changes_come_in_batches_that_never_skip_a_rev(db: PgPool) {
    let server = Server::start(db.clone()).await;
    let user = db::create_user(&db).await.unwrap();
    let account = NewAccount {
        user_id: user,
        provider: Provider::Jmap,
        address: "ann@example.com",
        login: "l",
        credentials: vec![],
    };
    let account_id = db::upsert_account(&db, account).await.unwrap();
    let account = db::account(&db, account_id).await.unwrap().unwrap();
    let all: Vec<RemoteMessage> = (0..1200).map(remote).collect();
    db::upsert_messages(&db, &account, &all).await.unwrap();
    db::set_account_status(&db, &account, "ready").await.unwrap();

    // The account's status changed after every message was written, so it comes last.
    let (accounts, messages, cursor, more) = changes(&server, user, 0).await;
    assert_eq!((accounts, messages, more), (0, BATCH_SIZE, true));
    let (accounts, messages, cursor, more) = changes(&server, user, cursor).await;
    assert_eq!((accounts, messages, more), (0, BATCH_SIZE, true));
    let (accounts, messages, cursor, more) = changes(&server, user, cursor).await;
    assert_eq!((accounts, messages, more), (1, 200, false));
    assert_eq!(changes(&server, user, cursor).await, (0, 0, cursor, false));

    // Writing what the provider says again changes nothing, so the cursor stays.
    db::upsert_messages(&db, &account, &all[..10]).await.unwrap();
    assert_eq!(changes(&server, user, cursor).await, (0, 0, cursor, false));
}

#[sqlx::test]
async fn an_op_sent_twice_is_applied_once_and_holds_off_stale_provider_state(db: PgPool) {
    let server = Server::start(db.clone()).await;
    let user = db::create_user(&db).await.unwrap();
    let account = NewAccount {
        user_id: user,
        provider: Provider::Jmap,
        address: "ann@example.com",
        login: "l",
        credentials: vec![],
    };
    let account_id = db::upsert_account(&db, account).await.unwrap();
    let account = db::account(&db, account_id).await.unwrap().unwrap();
    db::upsert_messages(&db, &account, &[remote(0)]).await.unwrap();
    let id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM messages").fetch_one(&db).await.unwrap();

    let archive = Op::Archive { ids: vec![id.to_string()] };
    for _ in 0..2 {
        let outcome = ops::apply(&server.state, user, "op-1", archive.clone()).await.unwrap();
        assert!(matches!(outcome, ops::Outcome::Done { ok: true, .. }));
    }
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM provider_ops").fetch_one(&db).await.unwrap();
    assert_eq!(pending, 1);

    // The provider hasn't seen the archive yet and still says inbox: the message stays archived.
    db::upsert_messages(&db, &account, &[remote(0)]).await.unwrap();
    let labels: Vec<String> = sqlx::query_scalar("SELECT labels FROM messages").fetch_one(&db).await.unwrap();
    assert!(labels.is_empty());
}

#[sqlx::test]
async fn preferences_and_drafts_sync_and_leave_tombstones(db: PgPool) {
    let server = Server::start(db.clone()).await;
    let user = db::create_user(&db).await.unwrap();
    let read = async |cursor| {
        let ServerMessage::Changes { preferences, drafts, cursor, .. } =
            crate::changes::read(&db, user, cursor).await.unwrap()
        else {
            panic!("not changes");
        };
        (preferences, drafts, cursor)
    };
    let apply = async |op_id: &str, op: Op| {
        let outcome = ops::apply(&server.state, user, op_id, op).await.unwrap();
        assert!(matches!(outcome, ops::Outcome::Done { ok: true, .. }));
    };
    let draft = mail_protocol::Draft { subject: "Plans".into(), text: "Hi".into(), ..Default::default() };

    apply("1", Op::SetPreference { key: "split_inbox".into(), value: Some(serde_json::json!(true)) }).await;
    apply("2", Op::SaveDraft { draft_id: "d1".into(), draft: Box::new(draft.clone()) }).await;
    let (preferences, drafts, cursor) = read(0).await;
    assert_eq!((preferences[0].key.as_str(), &preferences[0].value), ("split_inbox", &serde_json::json!(true)));
    assert_eq!((drafts[0].id.as_str(), &drafts[0].draft, drafts[0].deleted), ("d1", &draft, false));

    apply("3", Op::SetPreference { key: "split_inbox".into(), value: None }).await;
    apply("4", Op::DeleteDraft { draft_id: "d1".into() }).await;
    let (preferences, drafts, next) = read(cursor).await;
    assert!(preferences.len() == 1 && preferences[0].deleted);
    assert!(drafts.len() == 1 && drafts[0].deleted);
    assert_eq!(read(next).await, (vec![], vec![], next));
}
