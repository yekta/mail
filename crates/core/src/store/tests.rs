use std::time::{Duration, Instant};

use chrono::Utc;
use mail_protocol::Provider;
use serde_json::json;

use super::*;

fn store() -> Store {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("mail.db")).unwrap();
    std::mem::forget(dir);
    store
}

fn account() -> Account {
    Account {
        id: "a".into(),
        provider: Provider::Jmap,
        address: "me@example.com".into(),
        status: "ready".into(),
        color: "account-1".into(),
        identities: vec![Identity { name: None, email: "alias@example.com".into(), signature: None }],
        deleted: false,
        rev: 1,
    }
}

fn message(id: &str, thread: &str, from: &str, date: i64) -> Message {
    Message {
        id: id.into(),
        account_id: "a".into(),
        thread_id: thread.into(),
        from: Address::new(Some(from), &format!("{}@x.com", from.to_lowercase())),
        recipients: Recipients::default(),
        subject: format!("About {thread}"),
        snippet: format!("Snippet {id}"),
        date,
        unread: true,
        starred: false,
        labels: vec!["inbox".into()],
        attachments: vec![],
        message_id: None,
        in_reply_to: None,
        references: vec![],
        snoozed_until: None,
        bulk: false,
        unsubscribe: None,
        deleted: false,
        rev: 2,
    }
}

fn mine(id: &str, thread: &str, to: &str, date: i64) -> Message {
    let mut sent = message(id, thread, "Me", date);
    sent.from = Address::new(None, "me@example.com");
    sent.recipients.to = vec![Address::new(None, to)];
    sent.labels = vec!["sent".into()];
    sent.unread = false;
    sent
}

fn apply(store: &mut Store, accounts: &[Account], messages: &[Message], cursor: i64) -> Applied {
    store.apply_changes(Batch { accounts, messages, cursor: Some(cursor), ..Default::default() }).unwrap()
}

fn page(store: &Store, mailbox: &str) -> ThreadPage {
    store.thread_page(mailbox, 0, 50, None, &Utc::now()).unwrap()
}

fn ids(page: &ThreadPage) -> Vec<&str> {
    page.rows.iter().map(|row| row.id.as_str()).collect()
}

fn inbox(store: &Store) -> Vec<ThreadRow> {
    page(store, "inbox").rows
}

fn set(store: &mut Store, key: &str, value: Value) {
    let op = Op::SetPreference { key: key.into(), value: Some(value) };
    store.apply_local(&format!("set {key}"), &op).unwrap();
}

/// A mailbox's counts: its label, account, total, unread and starred.
type Counts = Vec<(String, String, i64, i64, i64)>;

/// Every count, as `counts` keeps it and as a full scan finds it: they must agree.
fn counts(store: &Store) -> (Counts, Counts) {
    let read = |sql: &str| -> Counts {
        let mut statement = store.db.prepare(sql).unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    (
        read("SELECT label, account_id, total, unread, starred FROM counts WHERE total <> 0 ORDER BY 1, 2"),
        read(
            "SELECT label, account_id, count(*), SUM(unread > 0), SUM(starred > 0) FROM thread_labels
             GROUP BY 1, 2 ORDER BY 1, 2",
        ),
    )
}

fn assert_counts(store: &Store) {
    let (kept, scanned) = counts(store);
    assert_eq!(kept, scanned);
}

#[test]
fn threads_are_kept_from_their_messages() {
    let mut store = store();
    let mut mine = message("m2", "t1", "Me", 2_000);
    mine.from = Address::new(None, "me@example.com");
    mine.unread = false;
    let messages = [message("m1", "t1", "Alice", 1_000), mine, message("m3", "t2", "Bob", 1_500)];
    apply(&mut store, &[account()], &messages, 10);
    let rows = inbox(&store);
    assert_eq!(rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), ["a:t1", "a:t2"]);
    assert_eq!(rows[0].senders, "Alice, me (2)");
    assert_eq!(rows[0].snippet, "Snippet m2");
    assert!(rows[0].unread);
    assert_eq!(store.cursor(), 10);
    let (unified, accounts) = store.mailboxes().unwrap();
    assert_eq!(unified[0].unread, 2);
    assert_eq!(accounts[0].identities[0].email, "alias@example.com");
    assert!(store.me().contains("alias@example.com"));
}

#[test]
fn a_local_op_survives_a_stale_server_change_and_settles() {
    let mut store = store();
    apply(&mut store, &[account()], &[message("m1", "t1", "Alice", 1_000)], 1);

    store.apply_local("op1", &Op::Archive { ids: vec!["m1".into()] }).unwrap();
    assert!(inbox(&store).is_empty());

    // The server hasn't applied it yet, but the sender's name changed: rebased, it stays archived.
    let mut renamed = message("m1", "t1", "Alicia", 1_000);
    renamed.unread = false;
    apply(&mut store, &[], &[renamed], 2);
    assert!(inbox(&store).is_empty());
    let archived = page(&store, "archive").rows;
    assert_eq!(archived[0].senders, "Alicia");
    assert!(!archived[0].unread);

    store.settle("op1", true).unwrap();
    assert!(store.outbox().unwrap().is_empty());
    assert!(inbox(&store).is_empty());
    assert_counts(&store);
}

#[test]
fn a_refused_op_goes_back_to_the_server_state_under_the_others() {
    let mut store = store();
    apply(&mut store, &[account()], &[message("m1", "t1", "Alice", 1_000)], 1);
    store.apply_local("archive", &Op::Archive { ids: vec!["m1".into()] }).unwrap();
    store.apply_local("read", &Op::SetUnread { ids: vec!["m1".into()], unread: false }).unwrap();

    let (op, threads) = store.settle("archive", false).unwrap();
    assert!(matches!(op, Some(Op::Archive { .. })));
    assert_eq!(threads, ["a:t1"]);
    let rows = inbox(&store);
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].unread, "the read op still waits on top");
    assert_counts(&store);
}

#[test]
fn deleted_messages_and_accounts_leave_their_threads_and_counts() {
    let mut store = store();
    apply(&mut store, &[account()], &[message("m1", "t1", "Alice", 1_000), message("m2", "t2", "Bob", 2_000)], 1);
    let mut gone = message("m1", "t1", "Alice", 1_000);
    gone.deleted = true;
    apply(&mut store, &[], &[gone], 2);
    assert_eq!(inbox(&store).len(), 1);
    assert_eq!(page(&store, "inbox").total, 1);
    let mut removed = account();
    removed.deleted = true;
    apply(&mut store, &[removed], &[], 3);
    assert!(inbox(&store).is_empty());
    assert!(store.me().is_empty());
    assert_eq!(page(&store, "inbox").total, 0);
    assert_counts(&store);
}

#[test]
fn archive_trash_and_snooze_show_in_their_own_mailboxes() {
    let mut archived = message("m", "t", "A", 1);
    archived.labels = vec![];
    assert_eq!(shown_in(&archived), ["archive"]);
    let mut trashed = message("m", "t", "A", 1);
    trashed.labels = vec!["trash".into(), "inbox".into()];
    trashed.starred = true;
    assert_eq!(shown_in(&trashed), ["trash"]);
    let mut sent = message("m", "t", "A", 1);
    sent.labels = vec!["sent".into()];
    assert_eq!(shown_in(&sent), ["sent"]);
    let mut snoozed = message("m", "t", "A", 1);
    snoozed.labels = vec![];
    snoozed.snoozed_until = Some(5);
    assert_eq!(shown_in(&snoozed), ["snoozed"]);
}

#[test]
fn counts_and_filters_follow_every_change() {
    let mut store = store();
    let mut starred = message("m3", "t3", "Cy", 3_000);
    starred.starred = true;
    starred.unread = false;
    apply(
        &mut store,
        &[account()],
        &[message("m1", "t1", "Ann", 1_000), message("m2", "t2", "Bob", 2_000), starred],
        1,
    );
    let filtered = |store: &Store, filter| store.thread_page("a/inbox", 0, 50, Some(filter), &Utc::now()).unwrap();
    assert_eq!(page(&store, "inbox").total, 3);
    assert_eq!(ids(&filtered(&store, Filter::Unread)), ["a:t2", "a:t1"]);
    assert_eq!(filtered(&store, Filter::Unread).total, 2);
    assert_eq!(ids(&filtered(&store, Filter::Starred)), ["a:t3"]);

    store.apply_local("1", &Op::SetUnread { ids: vec!["m2".into()], unread: false }).unwrap();
    store.apply_local("2", &Op::Archive { ids: vec!["m1".into()] }).unwrap();
    store.apply_local("3", &Op::SetStarred { ids: vec!["m2".into()], starred: true }).unwrap();
    assert_eq!(filtered(&store, Filter::Unread).total, 0);
    assert_eq!(ids(&filtered(&store, Filter::Starred)), ["a:t3", "a:t2"]);
    assert_eq!(page(&store, "inbox").total, 2);
    assert_eq!(page(&store, "a/archive").total, 1);
    assert_eq!(store.mailboxes().unwrap().0[1].unread, 0, "starred threads are all read");
    store.settle("2", false).unwrap();
    assert_eq!(page(&store, "inbox").total, 3);
    assert_counts(&store);
}

#[test]
fn the_inbox_splits_into_important_custom_splits_and_other() {
    let mut store = store();
    let mut newsletter = message("m1", "t1", "News", 1_000);
    newsletter.bulk = true;
    let mut team = message("m2", "t2", "Ann", 2_000);
    team.from = Address::new(Some("Ann"), "ann@team.example");
    let person = message("m3", "t3", "Bob", 3_000);
    let mut labelled = message("m4", "t4", "Cy", 4_000);
    labelled.labels.push("travel".into());
    labelled.bulk = true;
    // Writing from the team's domain doesn't put a thread in the team's split.
    let mut answered = mine("m5", "t3", "bob@x.com", 5_000);
    answered.from = Address::new(None, "me@team.example");
    let mut me = account();
    me.identities.push(Identity { name: None, email: "me@team.example".into(), signature: None });
    apply(&mut store, &[me], &[newsletter, team, person, labelled, answered], 1);
    assert!(page(&store, "inbox").splits.is_empty());
    assert_eq!(ids(&page(&store, "inbox:other")).len(), 4, "with the splits off, a split is the inbox");

    set(&mut store, "split:team", json!({ "name": "Team", "from": ["@team.example"], "order": 1 }));
    set(&mut store, "split:trips", json!({ "name": "Trips", "label": "travel", "order": 0 }));
    set(&mut store, "split_inbox", json!(true));
    let tabs: Vec<(String, String, u32, u32)> =
        page(&store, "inbox").splits.into_iter().map(|tab| (tab.mailbox, tab.name, tab.unread, tab.total)).collect();
    assert_eq!(
        tabs,
        [
            ("inbox:important".into(), "Important".into(), 1, 1),
            ("inbox:trips".into(), "Trips".into(), 1, 1),
            ("inbox:team".into(), "Team".into(), 1, 1),
            ("inbox:other".into(), "Other".into(), 1, 1),
        ]
    );
    assert_eq!(ids(&page(&store, "a/inbox:important")), ["a:t3"]);
    assert_eq!(page(&store, "a/inbox:team").splits[2].mailbox, "a/inbox:team");
    assert_eq!(ids(&page(&store, "inbox:trips")), ["a:t4"]);
    assert_eq!(ids(&page(&store, "inbox:other")), ["a:t1"]);

    // A reply from a person makes a newsletter's thread important.
    apply(&mut store, &[], &[message("m6", "t1", "Dee", 6_000)], 2);
    assert_eq!(ids(&page(&store, "inbox:important")), ["a:t1", "a:t3"]);
    store.apply_local("archive", &Op::Archive { ids: vec!["m3".into(), "m5".into()] }).unwrap();
    assert_eq!(ids(&page(&store, "inbox:important")), ["a:t1"]);
    assert_counts(&store);

    store.apply_local("off", &Op::SetPreference { key: "split_inbox".into(), value: None }).unwrap();
    assert!(page(&store, "inbox").splits.is_empty());
    assert_counts(&store);
    assert_eq!(counts(&store).0.iter().filter(|(label, ..)| label.starts_with("inbox:")).count(), 0);
}

#[test]
fn search_finds_words_by_prefix_in_bodies_too() {
    let mut store = store();
    apply(&mut store, &[account()], &[message("m1", "t1", "Alice", 1_000), message("m2", "t2", "Bob", 2_000)], 1);
    store.save_body("m2", &Body { html: None, text: Some("The telescope arrived".into()) }).unwrap();
    let search = |store: &Store, query: &str| store.search(query, &Utc::now()).unwrap();
    assert_eq!(search(&store, "teles"), ["a:t2"]);
    assert_eq!(search(&store, "alice"), ["a:t1"]);
    assert!(search(&store, "\"; DROP").is_empty());
}

#[test]
fn search_takes_operators() {
    let mut store = store();
    let mut old = message("m1", "t1", "Alice", 1_000);
    old.unread = false;
    old.labels.push("clients".into());
    old.attachments = vec![Attachment { name: "a.pdf".into(), mime: "application/pdf".into(), size: 1 }];
    let mut trashed = message("m3", "t3", "Alice", 3_000);
    trashed.labels = vec!["trash".into()];
    let sent = mine("m4", "t4", "alice@x.com", 4_000);
    let labels =
        [Label { id: "clients".into(), account_id: "a".into(), name: "Big Clients".into(), deleted: false, rev: 1 }];
    store
        .apply_changes(Batch {
            accounts: &[account()],
            labels: &labels,
            messages: &[old, message("m2", "t2", "Bob", 2_000), trashed, sent],
            cursor: Some(1),
            ..Default::default()
        })
        .unwrap();
    let search = |query: &str| store.search(query, &Utc::now()).unwrap();
    assert_eq!(search("from:alice"), ["a:t1"], "trash only when asked for");
    assert_eq!(search("from:alice in:trash"), ["a:t3"]);
    assert_eq!(search("to:alice"), ["a:t4"]);
    assert_eq!(search("alice"), ["a:t4", "a:t1"], "newest first");
    assert_eq!(search("alice -sent"), ["a:t4", "a:t1"]);
    assert_eq!(search("snippet -m4"), ["a:t2", "a:t1"]);
    assert_eq!(search("is:unread"), ["a:t2"]);
    assert_eq!(search("-m2 is:read"), ["a:t4", "a:t1"]);
    assert_eq!(search("has:attachment"), ["a:t1"]);
    assert_eq!(search("label:big-clients"), ["a:t1"]);
    assert!(search("label:nothing").is_empty());
    assert_eq!(search("in:sent"), ["a:t4"]);
    assert_eq!(search("bob OR alice is:unread"), ["a:t2"]);
    assert_eq!(search("after:1970/01/01 before:1970/01/01"), Vec::<String>::new());
    assert_eq!(search("older_than:1d").len(), 3);
}

#[test]
fn contacts_rank_people_written_to_and_match_names_and_addresses() {
    let mut store = store();
    let mut ann = message("m1", "t1", "Ann Lee", 1_000);
    ann.recipients.cc = vec![Address::new(Some("Annie Hall"), "annie@y.com")];
    let mut robot = message("m2", "t2", "Robot", 2_000);
    robot.from = Address::new(Some("Ann's shop"), "noreply@annshop.com");
    apply(
        &mut store,
        &[account()],
        &[ann, robot, mine("m3", "t3", "annie@y.com", 3_000), mine("m4", "t4", "zed@z.com", 4_000)],
        1,
    );
    let found = |query: &str| -> Vec<String> {
        store.contacts(query, 8).unwrap().into_iter().map(|address| address.email).collect()
    };
    assert_eq!(found("ann"), ["annie@y.com", "ann lee@x.com"]);
    assert_eq!(found("hall"), ["annie@y.com"]);
    assert_eq!(found("ann l"), ["ann lee@x.com"]);
    assert_eq!(found("y.com"), ["annie@y.com"]);
    assert_eq!(found("me@"), Vec::<String>::new(), "the user isn't a contact");
    assert_eq!(found(""), ["zed@z.com", "annie@y.com", "ann lee@x.com"]);

    let person = store.person("ANNIE@y.com", &Utc::now()).unwrap();
    assert_eq!((person.name.as_str(), person.initials.as_str()), ("Annie Hall", "AH"));
    assert_eq!(person.threads.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), ["a:t3", "a:t1"]);
}

#[test]
fn saved_drafts_show_in_drafts_and_on_their_thread() {
    let mut store = store();
    apply(&mut store, &[account()], &[message("m1", "t1", "Ann", 1_000)], 1);
    let reply = Draft {
        account_id: "a".into(),
        subject: "Re: About t1".into(),
        thread_id: Some("t1".into()),
        ..Default::default()
    };
    let touched = store.apply_local("save", &Op::SaveDraft { draft_id: "d1".into(), draft: Box::new(reply) }).unwrap();
    assert!(touched.threads.contains("a:t1") && touched.mailboxes);
    assert_eq!(store.reply_draft_of("a:t1").as_deref(), Some("d1"));
    let drafts = page(&store, "drafts");
    assert_eq!((drafts.total, drafts.rows[0].draft_id.as_deref()), (1, Some("d1")));
    assert_eq!(drafts.rows[0].senders, "(no recipients)");

    // The server's older copy doesn't overwrite what waits to be sent; once settled, it does.
    let server = SavedDraft {
        id: "d1".into(),
        draft: Draft { account_id: "a".into(), subject: "Older".into(), ..Default::default() },
        updated: 5,
        deleted: false,
        rev: 3,
    };
    store.apply_changes(Batch { drafts: std::slice::from_ref(&server), ..Default::default() }).unwrap();
    assert_eq!(store.saved_draft("d1").unwrap().subject, "Re: About t1");
    store.settle("save", true).unwrap();
    store.apply_changes(Batch { drafts: &[SavedDraft { deleted: true, ..server }], ..Default::default() }).unwrap();
    assert!(store.saved_draft("d1").is_none());
    assert_eq!(page(&store, "drafts").total, 0);
    assert_eq!(store.reply_draft_of("a:t1"), None);
}

#[test]
fn preferences_set_here_win_until_the_server_has_them() {
    let mut store = store();
    set(&mut store, "notify:a", json!(false));
    let from_server = |value: Value, deleted: bool| Preference { key: "notify:a".into(), value, deleted, rev: 1 };
    let applied =
        store.apply_changes(Batch { preferences: &[from_server(json!(true), false)], ..Default::default() }).unwrap();
    assert!(!applied.touched.preferences);
    assert_eq!(store.preference("notify:a"), Some(json!(false)));
    store.settle("set notify:a", true).unwrap();
    let applied =
        store.apply_changes(Batch { preferences: &[from_server(Value::Null, true)], ..Default::default() }).unwrap();
    assert!(applied.touched.preferences);
    assert!(store.preferences().unwrap().is_empty());
}

#[test]
fn an_account_colour_chosen_here_wins_until_the_server_has_it() {
    let mut store = store();
    apply(&mut store, &[account()], &[message("m", "t", "Alice", 1_000)], 1);
    let color = |store: &Store| store.accounts().unwrap()[0].color.clone();
    let op = Op::SetAccountColor { account_id: "a".into(), color: "account-7".into() };
    let touched = store.apply_local("colour", &op).unwrap();
    assert!(touched.mailboxes);
    assert_eq!(color(&store), "account-7");
    assert_eq!(inbox(&store)[0].color, "account-7");

    apply(&mut store, &[account()], &[], 2);
    assert_eq!(color(&store), "account-7");
    store.settle("colour", true).unwrap();
    apply(&mut store, &[account()], &[], 3);
    assert_eq!(color(&store), "account-1");
}

#[test]
fn new_unread_inbox_mail_from_others_arrives_once() {
    let mut store = store();
    let mut read = message("m2", "t2", "Bob", 2_000);
    read.unread = false;
    let applied = apply(
        &mut store,
        &[account()],
        &[message("m1", "t1", "Ann", 1_000), read, mine("m3", "t3", "x@x.com", 3_000)],
        1,
    );
    let arrived: Vec<&str> = applied.arrived.iter().map(|arrived| arrived.mail.thread.as_str()).collect();
    assert_eq!(arrived, ["a:t1"]);
    assert_eq!(applied.arrived[0].mail.from, "Ann");
    assert!(apply(&mut store, &[], &[message("m1", "t1", "Ann", 1_000)], 2).arrived.is_empty());
}

#[test]
fn a_new_schema_starts_over_but_keeps_the_session_and_the_outbox() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let draft = Draft { account_id: "a".into(), subject: "Plans".into(), ..Default::default() };
    {
        let mut store = Store::open(&path).unwrap();
        store.set_meta("server", Some("https://mail.example")).unwrap();
        store.set_meta("token", Some("secret")).unwrap();
        apply(&mut store, &[account()], &[message("m1", "t1", "Ann", 1_000)], 7);
        store.apply_local("archive", &Op::Archive { ids: vec!["m1".into()] }).unwrap();
        set(&mut store, "notify:a", json!(false));
        store.apply_local("save", &Op::SaveDraft { draft_id: "d1".into(), draft: Box::new(draft.clone()) }).unwrap();
        store.keep_original("send", &draft, Some("d1")).unwrap();
        store.set_meta("schema", Some("0")).unwrap();
    }
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.cursor(), 0);
    assert!(inbox(&store).is_empty());
    assert_eq!(store.meta("server").as_deref(), Some("https://mail.example"));
    assert_eq!(store.meta("token").as_deref(), Some("secret"));
    assert_eq!(store.outbox().unwrap().len(), 3);
    assert_eq!(store.original("send"), Some((draft, Some("d1".into()))));
    assert_eq!(store.preference("notify:a"), Some(json!(false)));
    assert_eq!(store.saved_draft("d1").unwrap().subject, "Plans");

    // Synced again from the start, the message shows with the archive that still waits.
    apply(&mut store, &[account()], &[message("m1", "t1", "Ann", 1_000)], 7);
    assert!(inbox(&store).is_empty());
    assert_eq!(page(&store, "archive").total, 1);
    store.settle("archive", false).unwrap();
    assert_eq!(inbox(&store).len(), 1);
}

#[test]
fn server_changes_to_messages_touch_the_mailboxes() {
    let mut store = store();
    assert!(apply(&mut store, &[account()], &[message("m1", "t1", "Ann", 1_000)], 1).touched.mailboxes);
    let mut gone = message("m1", "t1", "Ann", 1_000);
    gone.deleted = true;
    assert!(apply(&mut store, &[], &[gone], 2).touched.mailboxes);
    assert!(!apply(&mut store, &[], &[], 3).touched.mailboxes);
}

#[test]
fn getting_to_zero_archives_what_the_mailbox_shows() {
    let mut store = store();
    let mut newsletter = message("m1", "t1", "News", 1_000);
    newsletter.bulk = true;
    let mut read = message("m3", "t3", "Cy", 3_000);
    read.unread = false;
    apply(&mut store, &[account()], &[newsletter, message("m2", "t2", "Bob", 2_000), read], 1);
    let archived = |store: &Store, mailbox: &str, filter| store.inbox_messages(mailbox, None, filter).unwrap();
    assert_eq!(archived(&store, "inbox", None), ["m3", "m2", "m1"]);
    assert_eq!(archived(&store, "inbox", Some(Filter::Unread)), ["m2", "m1"]);
    set(&mut store, "split_inbox", json!(true));
    assert_eq!(archived(&store, "inbox", None), ["m3", "m2"], "the inbox shows its Important split");
    assert_eq!(archived(&store, "a/inbox:important", Some(Filter::Unread)), ["m2"]);
    assert_eq!(archived(&store, "inbox:other", None), ["m1"]);
    assert_eq!(archived(&store, "a/inbox:other", Some(Filter::Starred)), Vec::<String>::new());
}

/// The query plans of the reads the apps make all the time: none scans a table or sorts.
#[test]
fn reads_use_indexes() {
    let store = store();
    let plan = |sql: &str| -> String {
        let mut statement = store.db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let rows = statement.query_map([], |row| row.get::<_, String>(3)).unwrap();
        rows.map(Result::unwrap).collect::<Vec<_>>().join("; ")
    };
    let checks = [
        format!(
            "SELECT {ROW_COLUMNS} FROM thread_labels l JOIN threads t ON t.id = l.thread JOIN accounts a ON a.id = t.account_id
             WHERE l.label = 'inbox' AND 'x' IS NULL ORDER BY l.last_date DESC LIMIT 100"
        ),
        format!(
            "SELECT {ROW_COLUMNS} FROM thread_labels l JOIN threads t ON t.id = l.thread JOIN accounts a ON a.id = t.account_id
             WHERE l.label = 'inbox' AND l.account_id = 'a' ORDER BY l.last_date DESC LIMIT 100"
        ),
        format!(
            "SELECT {ROW_COLUMNS} FROM thread_labels l JOIN threads t ON t.id = l.thread JOIN accounts a ON a.id = t.account_id
             WHERE l.label = 'inbox' AND 'x' IS NULL AND l.unread > 0 ORDER BY l.last_date DESC LIMIT 100"
        ),
        format!(
            "SELECT {ROW_COLUMNS} FROM thread_labels l JOIN threads t ON t.id = l.thread JOIN accounts a ON a.id = t.account_id
             WHERE l.label = 'inbox' AND 'x' IS NULL AND l.starred > 0 ORDER BY l.last_date DESC LIMIT 100"
        ),
        "SELECT COALESCE(SUM(total), 0) FROM counts WHERE label = 'inbox' AND 'x' IS NULL".into(),
        "SELECT thread FROM thread_people WHERE email = 'a@x.com' ORDER BY last_date DESC LIMIT 30".into(),
        "SELECT c.email FROM contacts c WHERE c.email IN (SELECT email FROM contact_words WHERE word >= 'a' AND word < 'b')"
            .into(),
        "SELECT email FROM contacts ORDER BY (sent * 4 + received) DESC, last DESC LIMIT 8".into(),
        "SELECT m.thread FROM search JOIN messages m ON m.search_id = search.rowid WHERE search MATCH 'x' \
         ORDER BY search.rowid DESC"
            .into(),
        "SELECT m.thread FROM messages m WHERE m.unread = 1 ORDER BY m.search_id DESC".into(),
        "SELECT id FROM saved_drafts WHERE thread = 'a:t' ORDER BY updated DESC LIMIT 1".into(),
    ];
    for sql in checks {
        let plan = plan(&sql);
        let scans = plan
            .split("; ")
            .any(|step| step.starts_with("SCAN") && !step.contains("VIRTUAL TABLE") && !step.contains("USING"));
        assert!(!scans && !plan.contains("TEMP B-TREE FOR ORDER BY"), "{sql}\n{plan}");
    }
}

/// Fills a store as a big mailbox would and times what the apps do most.
fn fill_and_time(messages_count: usize, accounts_count: usize) -> Vec<(&'static str, Duration)> {
    let mut store = store();
    let accounts: Vec<Account> = (0..accounts_count)
        .map(|index| Account { id: format!("a{index}"), address: format!("me{index}@example.com"), ..account() })
        .collect();
    let start = Instant::now();
    store.apply_changes(Batch { accounts: &accounts, ..Default::default() }).unwrap();
    let words = ["budget", "launch", "dinner", "invoice", "travel", "review", "photos", "notes"];
    let mut batch = Vec::new();
    for index in 0..messages_count {
        let account = (index / 3) % accounts_count;
        let mut item = message(
            &format!("m{index}"),
            &format!("t{}", index / 3),
            &format!("P{}", index % 997),
            index as i64 * 60_000,
        );
        item.account_id = format!("a{account}");
        item.unread = index % 5 == 0;
        item.starred = index % 50 == 0;
        item.bulk = index % 4 == 0;
        item.labels = match index % 10 {
            0..=3 => vec!["inbox".into()],
            4 => vec!["sent".into()],
            5 => vec!["trash".into()],
            _ => vec![format!("label{}", index % 7)],
        };
        item.recipients.to = vec![Address::new(None, &format!("me{account}@example.com"))];
        item.snippet = format!("{} {} {index}", words[index % 8], words[(index / 8) % 8]);
        batch.push(item);
        if batch.len() == 500 {
            apply(&mut store, &[], &batch, index as i64);
            batch.clear();
        }
    }
    apply(&mut store, &[], &batch, messages_count as i64);
    let filled = start.elapsed();
    set(&mut store, "split:p", json!({ "name": "P", "from": ["p1@x.com"] }));
    let start = Instant::now();
    set(&mut store, "split_inbox", json!(true));
    let resplit = start.elapsed();

    let time = |work: &dyn Fn()| {
        let start = Instant::now();
        for _ in 0..10 {
            work();
        }
        start.elapsed() / 10
    };
    let now = Utc::now();
    let mut timings = vec![("fill", filled), ("turn the splits on", resplit)];
    timings.push(("mailboxes", time(&|| drop(store.mailboxes().unwrap()))));
    timings.push(("inbox, first page", time(&|| drop(store.thread_page("inbox", 0, 100, None, &now).unwrap()))));
    timings.push(("inbox, page 50", time(&|| drop(store.thread_page("inbox", 5_000, 100, None, &now).unwrap()))));
    timings
        .push(("one account's archive", time(&|| drop(store.thread_page("a1/archive", 0, 100, None, &now).unwrap()))));
    timings.push((
        "unread filter",
        time(&|| drop(store.thread_page("inbox", 0, 100, Some(Filter::Unread), &now).unwrap())),
    ));
    timings.push((
        "starred filter",
        time(&|| drop(store.thread_page("inbox", 0, 100, Some(Filter::Starred), &now).unwrap())),
    ));
    timings.push(("split", time(&|| drop(store.thread_page("inbox:important", 0, 100, None, &now).unwrap()))));
    timings.push(("contacts", time(&|| drop(store.contacts("p1", 8).unwrap()))));
    timings.push(("person", time(&|| drop(store.person("p1@x.com", &now).unwrap()))));
    timings.push(("search, common word", time(&|| drop(store.search("budget", &now).unwrap()))));
    timings.push(("search, operators", time(&|| drop(store.search("from:p12 is:unread", &now).unwrap()))));
    let start = Instant::now();
    let ids: Vec<String> = (0..500).map(|index| format!("m{}", index * 10)).collect();
    store.apply_local("archive", &Op::Archive { ids }).unwrap();
    timings.push(("archive 500 messages", start.elapsed()));
    assert_counts(&store);
    timings
}

fn report(timings: &[(&str, Duration)]) {
    for (name, took) in timings {
        eprintln!("{name:>24}: {:>8.2} ms", took.as_secs_f64() * 1000.0);
    }
}

#[test]
fn a_big_store_stays_fast() {
    let timings = fill_and_time(30_000, 10);
    report(&timings);
    for (name, took) in &timings[2..timings.len() - 1] {
        assert!(*took < Duration::from_millis(50), "{name} took {took:?}");
    }
}

#[test]
#[ignore = "fills half a million messages; run with --ignored --nocapture to see the timings"]
fn a_huge_store_stays_fast() {
    report(&fill_and_time(500_000, 50));
}
