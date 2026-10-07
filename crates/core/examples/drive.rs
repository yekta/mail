//! Drives the core from a terminal, as an app would, to try it end to end without one.
//!
//!   cargo run -p mail-core --example drive -- [--server http://localhost:3000] [--data /tmp/mail-drive]
//!
//! Then type commands, one per line (also from a pipe):
//!
//!   jmap <url> <username> <password>     add a JMAP account (signs in)
//!   dev <email>                          dev login
//!   google, finish <mailapp://auth?...>  add a Gmail account
//!   status | mailboxes | signout
//!   threads [mailbox] [limit]            list a mailbox (inbox by default); rows get numbers
//!   open <n> [images]                    open a thread
//!   archive|trash|read|unread|star|unstar|inbox|spam <n>...
//!   snooze <n> <minutes>
//!   reply <n> | send <to> <subject> <text…> | cancel [op_id]   (sends wait 10 s to be undone)
//!   search <words…>
//!   sleep <seconds>                      wait, for scripts
//!   {"type": …}                          any command as JSON

use std::io::BufRead;
use std::sync::{Arc, Mutex};

/// The last send's op id, for `cancel` without one.
static LAST_SEND: Mutex<String> = Mutex::new(String::new());
use std::time::Duration;

use mail_core::api::{Command, Config, Event};
use serde_json::{Value, json};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().position(|arg| arg == name).and_then(|index| args.get(index + 1)).cloned();
    let server = flag("--server").unwrap_or_else(|| "http://localhost:3000".into());
    let data = flag("--data").unwrap_or_else(|| std::env::temp_dir().join("mail-drive").display().to_string());

    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    let rows: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen = rows.clone();
    let sink = Arc::new(move |event: Event| print_event(&event, &seen));
    let handle = {
        let _guard = runtime.enter();
        mail_core::core::start(Config { data_dir: data.clone(), server_url: Some(server.clone()), demo: false }, sink)
            .expect("the core starts")
    };
    println!("core at {data}, server {server}. Type a command.");

    let mut id = 0;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(first) = words.first() else { continue };
        if *first == "sleep" {
            std::thread::sleep(Duration::from_secs_f64(words.get(1).and_then(|s| s.parse().ok()).unwrap_or(1.0)));
            continue;
        }
        let thread = |index: usize| -> String {
            let rows = rows.lock().unwrap();
            let given = words.get(index).copied().unwrap_or("1");
            given
                .parse::<usize>()
                .ok()
                .and_then(|n| rows.get(n.wrapping_sub(1)).cloned())
                .unwrap_or_else(|| given.to_string())
        };
        let command = match *first {
            "status" => json!({ "type": "status" }),
            "mailboxes" => json!({ "type": "mailboxes" }),
            "signout" => json!({ "type": "sign_out" }),
            "dev" => json!({ "type": "dev_login", "email": words.get(1).unwrap_or(&"dev@example.com") }),
            "jmap" => {
                json!({ "type": "add_jmap_account", "url": words.get(1), "username": words.get(2), "password": words.get(3) })
            }
            "google" => json!({ "type": "sign_in_google" }),
            "finish" => json!({ "type": "finish_sign_in", "url": words.get(1) }),
            "threads" => json!({ "type": "threads", "mailbox": words.get(1).unwrap_or(&"inbox"),
                "limit": words.get(2).and_then(|n| n.parse::<usize>().ok()).unwrap_or(20) }),
            "open" => json!({ "type": "open_thread", "thread": thread(1), "images": words.get(2) == Some(&"images") }),
            "archive" | "trash" | "read" | "unread" | "star" | "unstar" | "inbox" | "spam" => {
                let threads: Vec<String> = (1..words.len().max(2)).map(thread).collect();
                json!({ "type": "act", "action": first, "threads": threads })
            }
            "snooze" => {
                let minutes: i64 = words.get(2).and_then(|n| n.parse().ok()).unwrap_or(60);
                let until = mail_protocol::now_ms() + minutes * 60_000;
                json!({ "type": "act", "action": "snooze", "threads": [thread(1)], "until": until })
            }
            "reply" => json!({ "type": "reply_draft", "thread": thread(1), "kind": "reply" }),
            "send" => json!({ "type": "send", "delay": 10, "draft": {
                "account_id": "", "to": [{ "email": words.get(1).unwrap_or(&"") }],
                "subject": words.get(2).unwrap_or(&""), "text": words.get(3..).map(|rest| rest.join(" ")).unwrap_or_default() } }),
            "cancel" => {
                json!({ "type": "cancel_send", "op_id": words.get(1).map(|id| id.to_string()).unwrap_or_else(|| LAST_SEND.lock().unwrap().clone()) })
            }
            "search" => json!({ "type": "search", "query": words[1..].join(" ") }),
            _ if line.trim_start().starts_with('{') => serde_json::from_str(&line).unwrap_or(Value::Null),
            _ => {
                println!("unknown command: {first}");
                continue;
            }
        };
        match serde_json::from_value::<Command>(command) {
            Ok(command) => {
                id += 1;
                handle.send(id, command);
            }
            Err(error) => println!("bad command: {error}"),
        }
    }
    std::thread::sleep(Duration::from_millis(500));
}

fn print_event(event: &Event, rows: &Mutex<Vec<String>>) {
    match event {
        Event::Reply { id, ok, value } => {
            if value["send_at"].is_i64() {
                *LAST_SEND.lock().unwrap() = value["op_id"].as_str().unwrap_or_default().to_string();
            }
            if let Some(list) = value["rows"].as_array() {
                let mut kept = rows.lock().unwrap();
                kept.clear();
                for (index, row) in list.iter().enumerate() {
                    kept.push(row["id"].as_str().unwrap_or_default().to_string());
                    println!(
                        "{:>3} {} {:<24} {:<40} {:>10}{}{}",
                        index + 1,
                        if row["unread"] == true { "●" } else { " " },
                        truncate(row["senders"].as_str().unwrap_or_default(), 24),
                        truncate(row["subject"].as_str().unwrap_or_default(), 40),
                        row["date"].as_str().unwrap_or_default(),
                        if row["starred"] == true { " ★" } else { "" },
                        if row["attachment"] == true { " ⎘" } else { "" },
                    );
                }
                println!(
                    "#{id} {} rows{}",
                    list.len(),
                    value.get("total").map(|total| format!(" of {total}")).unwrap_or_default()
                );
                return;
            }
            if let Some(messages) = value["messages"].as_array() {
                println!(
                    "#{id} {} — {}",
                    value["subject"].as_str().unwrap_or_default(),
                    value["participants"].as_str().unwrap_or_default()
                );
                for message in messages {
                    let html = message["html"]
                        .as_str()
                        .map(|html| format!("{} bytes of HTML", html.len()))
                        .unwrap_or("body loading".into());
                    println!(
                        "    {} · {} · {} · {html}",
                        message["from_name"].as_str().unwrap_or_default(),
                        message["date"].as_str().unwrap_or_default(),
                        message["snippet"].as_str().unwrap_or_default()
                    );
                }
                return;
            }
            println!("#{id} {} {}", if *ok { "ok" } else { "error" }, value);
        }
        Event::Changed { mailboxes, threads } => {
            println!("· changed{} {} threads", if *mailboxes { " (mailboxes)" } else { "" }, threads.len())
        }
        other => println!("· {}", serde_json::to_string(other).unwrap_or_default()),
    }
}

fn truncate(text: &str, width: usize) -> String {
    match text.chars().count() > width {
        true => format!("{}…", text.chars().take(width - 1).collect::<String>()),
        false => text.to_string(),
    }
}
