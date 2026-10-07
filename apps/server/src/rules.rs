//! What happens to mail new to the server as it is synced: mail in a muted thread leaves the
//! inbox, mail from a blocked address or `@domain` goes to the trash, and mail from someone else
//! ends the snooze of its thread (which is what makes "remind me if no reply" work). The
//! provider learns the first two through the usual ops; a snooze is the server's own.

use std::collections::HashSet;

use mail_protocol::{MessageState, Op, role};
use uuid::Uuid;

use crate::db::{AccountRow, UserTx, Written};

/// Applies the rules to messages just inserted. True when ops were queued for the provider.
pub async fn apply(tx: &mut UserTx, account: &AccountRow, new: &[Written]) -> sqlx::Result<bool> {
    if new.is_empty() {
        return Ok(false);
    }
    let own = account.own_addresses();
    let wanted: Vec<String> = new.iter().flat_map(|message| keys(account, message)).collect();
    let set: HashSet<String> = sqlx::query_scalar(
        "SELECT key FROM preferences WHERE user_id = $1 AND key = ANY($2) AND NOT deleted AND value <> 'false'",
    )
    .bind(account.user_id)
    .bind(&wanted)
    .fetch_all(&mut *tx.tx)
    .await?
    .into_iter()
    .collect();

    let mut trash: Vec<(Uuid, MessageState)> = Vec::new();
    let mut archive: Vec<(Uuid, MessageState)> = Vec::new();
    let mut answered: Vec<&str> = Vec::new();
    for message in new {
        let state = message.state();
        let from_user = own.contains(&message.from_email);
        let [muted, address, domain] = keys(account, message);
        let blocked = !from_user && (set.contains(&address) || set.contains(&domain));
        if blocked && ![role::TRASH, role::SPAM, role::SENT].iter().any(|role| state.has(role)) {
            trash.push((message.id, state));
            continue;
        }
        if !state.has(role::INBOX) {
            continue;
        }
        if set.contains(&muted) {
            archive.push((message.id, state));
            continue;
        }
        if !from_user {
            answered.push(&message.thread_id);
        }
    }

    crate::db::apply_op(tx, &trash, &Op::Trash { ids: Vec::new() }).await?;
    crate::db::apply_op(tx, &archive, &Op::Archive { ids: Vec::new() }).await?;
    if !answered.is_empty() {
        sqlx::query(
            "UPDATE messages SET snoozed_until = NULL, rev = nextval('revs')
             WHERE account_id = $1 AND thread_id = ANY($2) AND snoozed_until IS NOT NULL AND NOT deleted",
        )
        .bind(account.id)
        .bind(&answered)
        .execute(&mut *tx.tx)
        .await?;
    }
    Ok(!trash.is_empty() || !archive.is_empty())
}

/// The preference keys that touch a message: its thread muted, its sender or their domain blocked.
fn keys(account: &AccountRow, message: &Written) -> [String; 3] {
    let domain = message.from_email.rsplit_once('@').map(|(_, domain)| domain).unwrap_or_default();
    [
        format!("muted:{}:{}", account.id, message.thread_id),
        format!("blocked:{}", message.from_email),
        format!("blocked:@{domain}"),
    ]
}
