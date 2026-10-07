//! What a change asked for does to a message. The server and the client core apply the same
//! function: the core to show it at once, the server to record it before the provider has it.

use serde::{Deserialize, Serialize};

use crate::types::{Draft, role};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Op {
    SetUnread { ids: Vec<String>, unread: bool },
    SetStarred { ids: Vec<String>, starred: bool },
    Archive { ids: Vec<String> },
    MoveToInbox { ids: Vec<String> },
    Trash { ids: Vec<String> },
    Spam { ids: Vec<String> },
    AddLabel { ids: Vec<String>, label: String },
    RemoveLabel { ids: Vec<String>, label: String },
    Snooze { ids: Vec<String>, until: i64 },
    Send { draft: Box<Draft>, send_at: i64 },
    CancelSend { op_id: String },
    RemoveAccount { account_id: String },
}

/// The part of a message that ops change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageState {
    pub labels: Vec<String>,
    pub unread: bool,
    pub starred: bool,
    pub snoozed_until: Option<i64>,
}

impl Op {
    /// The messages it changes.
    pub fn ids(&self) -> &[String] {
        match self {
            Op::SetUnread { ids, .. }
            | Op::SetStarred { ids, .. }
            | Op::Archive { ids }
            | Op::MoveToInbox { ids }
            | Op::Trash { ids }
            | Op::Spam { ids }
            | Op::AddLabel { ids, .. }
            | Op::RemoveLabel { ids, .. }
            | Op::Snooze { ids, .. } => ids,
            Op::Send { .. } | Op::CancelSend { .. } | Op::RemoveAccount { .. } => &[],
        }
    }

    pub fn apply(&self, state: &mut MessageState) {
        match self {
            Op::SetUnread { unread, .. } => state.unread = *unread,
            Op::SetStarred { starred, .. } => state.starred = *starred,
            Op::Archive { .. } => state.remove(role::INBOX),
            Op::MoveToInbox { .. } => {
                state.remove(role::TRASH);
                state.remove(role::SPAM);
                state.add(role::INBOX);
                state.snoozed_until = None;
            }
            Op::Trash { .. } => {
                state.remove(role::INBOX);
                state.remove(role::SPAM);
                state.add(role::TRASH);
            }
            Op::Spam { .. } => {
                state.remove(role::INBOX);
                state.remove(role::TRASH);
                state.add(role::SPAM);
            }
            Op::AddLabel { label, .. } => state.add(label),
            Op::RemoveLabel { label, .. } => state.remove(label),
            Op::Snooze { until, .. } => {
                state.remove(role::INBOX);
                state.snoozed_until = Some(*until);
            }
            Op::Send { .. } | Op::CancelSend { .. } | Op::RemoveAccount { .. } => {}
        }
    }
}

impl MessageState {
    pub fn has(&self, label: &str) -> bool {
        self.labels.iter().any(|existing| existing == label)
    }

    fn add(&mut self, label: &str) {
        if !self.has(label) {
            self.labels.push(label.to_string());
        }
    }

    fn remove(&mut self, label: &str) {
        self.labels.retain(|existing| existing != label);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbox() -> MessageState {
        MessageState { labels: vec!["inbox".into(), "work".into()], unread: true, starred: false, snoozed_until: None }
    }

    fn ids() -> Vec<String> {
        vec!["m".into()]
    }

    #[test]
    fn archive_keeps_custom_labels() {
        let mut state = inbox();
        Op::Archive { ids: ids() }.apply(&mut state);
        assert_eq!(state.labels, ["work"]);
    }

    #[test]
    fn trash_then_inbox_round_trips() {
        let mut state = inbox();
        Op::Trash { ids: ids() }.apply(&mut state);
        assert_eq!(state.labels, ["work", "trash"]);
        Op::MoveToInbox { ids: ids() }.apply(&mut state);
        assert_eq!(state.labels, ["work", "inbox"]);
    }

    #[test]
    fn snooze_leaves_the_inbox_until_moved_back() {
        let mut state = inbox();
        Op::Snooze { ids: ids(), until: 42 }.apply(&mut state);
        assert!(!state.has("inbox"));
        assert_eq!(state.snoozed_until, Some(42));
        Op::MoveToInbox { ids: ids() }.apply(&mut state);
        assert_eq!(state.snoozed_until, None);
    }

    #[test]
    fn serializes_with_a_kind() {
        let json = serde_json::to_value(Op::SetUnread { ids: ids(), unread: false }).unwrap();
        assert_eq!(json, serde_json::json!({"kind": "set_unread", "ids": ["m"], "unread": false}));
    }
}
