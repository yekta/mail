//! Taking an action back: the ops that bring each message it changed back to the state it had.

use std::collections::BTreeMap;

use mail_protocol::{MessageState, Op};

/// How many message ids one op carries at most.
pub const OP_SIZE: usize = 500;

/// One step back, in the order they must run: out of a snooze first, then labels, then flags.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Step {
    Unsnooze,
    Snooze(i64),
    Add(String),
    Remove(String),
    Unread(bool),
    Starred(bool),
}

impl Step {
    fn op(&self, ids: Vec<String>) -> Op {
        match self {
            Step::Unsnooze => Op::MoveToInbox { ids },
            Step::Snooze(until) => Op::Snooze { ids, until: *until },
            Step::Add(label) => Op::AddLabel { ids, label: label.clone() },
            Step::Remove(label) => Op::RemoveLabel { ids, label: label.clone() },
            Step::Unread(unread) => Op::SetUnread { ids, unread: *unread },
            Step::Starred(starred) => Op::SetStarred { ids, starred: *starred },
        }
    }
}

/// An op on many messages as ops on at most `OP_SIZE` each.
pub fn in_chunks(op: Op) -> Vec<Op> {
    if op.ids().len() <= OP_SIZE {
        return vec![op];
    }
    op.ids().chunks(OP_SIZE).map(|ids| with_ids(&op, ids.to_vec())).collect()
}

fn with_ids(op: &Op, ids: Vec<String>) -> Op {
    match op {
        Op::SetUnread { unread, .. } => Op::SetUnread { ids, unread: *unread },
        Op::SetStarred { starred, .. } => Op::SetStarred { ids, starred: *starred },
        Op::Archive { .. } => Op::Archive { ids },
        Op::MoveToInbox { .. } => Op::MoveToInbox { ids },
        Op::Trash { .. } => Op::Trash { ids },
        Op::Spam { .. } => Op::Spam { ids },
        Op::AddLabel { label, .. } => Op::AddLabel { ids, label: label.clone() },
        Op::RemoveLabel { label, .. } => Op::RemoveLabel { ids, label: label.clone() },
        Op::Snooze { until, .. } => Op::Snooze { ids, until: *until },
        other => other.clone(),
    }
}

/// The ops that undo `ops` on messages that were in the states `before`.
pub fn inverse(before: &[(String, MessageState)], ops: &[Op]) -> Vec<Op> {
    let mut groups: BTreeMap<Step, Vec<String>> = BTreeMap::new();
    for (id, earlier) in before {
        let mut state = earlier.clone();
        for op in ops.iter().filter(|op| op.ids().contains(id)) {
            op.apply(&mut state);
        }
        if state == *earlier {
            continue;
        }
        let mut steps = Vec::new();
        match (earlier.snoozed_until, state.snoozed_until) {
            (None, Some(_)) => steps.push(Step::Unsnooze),
            (Some(until), now) if now != Some(until) => steps.push(Step::Snooze(until)),
            _ => {}
        }
        for step in &steps {
            step.op(Vec::new()).apply(&mut state);
        }
        steps.extend(state.labels.iter().filter(|label| !earlier.has(label)).map(|label| Step::Remove(label.clone())));
        steps.extend(earlier.labels.iter().filter(|label| !state.has(label)).map(|label| Step::Add(label.clone())));
        if state.unread != earlier.unread {
            steps.push(Step::Unread(earlier.unread));
        }
        if state.starred != earlier.starred {
            steps.push(Step::Starred(earlier.starred));
        }
        for step in steps {
            groups.entry(step).or_default().push(id.clone());
        }
    }
    groups.into_iter().flat_map(|(step, ids)| in_chunks(step.op(ids))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(labels: &[&str], unread: bool, snoozed_until: Option<i64>) -> MessageState {
        MessageState {
            labels: labels.iter().map(|label| label.to_string()).collect(),
            unread,
            starred: false,
            snoozed_until,
        }
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    /// Runs `ops`, then their inverse, and checks every message is back as it was.
    fn round_trip(before: &[(String, MessageState)], ops: &[Op]) -> Vec<Op> {
        let undo = inverse(before, ops);
        for (id, earlier) in before {
            let mut state = earlier.clone();
            for op in ops.iter().chain(&undo).filter(|op| op.ids().contains(id)) {
                op.apply(&mut state);
            }
            let mut sorted = state.labels.clone();
            sorted.sort();
            let mut expected = earlier.labels.clone();
            expected.sort();
            assert_eq!(
                (sorted, state.unread, state.starred, state.snoozed_until),
                (expected, earlier.unread, earlier.starred, earlier.snoozed_until),
                "{id}"
            );
        }
        undo
    }

    #[test]
    fn archive_comes_back_to_the_inbox_only() {
        let before = [("a".to_string(), state(&["inbox", "work"], true, None))];
        let undo = round_trip(&before, &[Op::Archive { ids: ids(&["a"]) }]);
        assert_eq!(undo, [Op::AddLabel { ids: ids(&["a"]), label: "inbox".into() }]);
    }

    #[test]
    fn trash_and_spam_go_back_where_they_were() {
        let before = [
            ("a".to_string(), state(&["inbox"], false, None)),
            ("b".to_string(), state(&["sent"], false, None)),
            ("c".to_string(), state(&["spam"], true, None)),
        ];
        round_trip(&before, &[Op::Trash { ids: ids(&["a", "b", "c"]) }]);
        round_trip(&before, &[Op::Spam { ids: ids(&["a", "b"]) }]);
    }

    #[test]
    fn snoozes_end_and_old_snoozes_come_back() {
        let before = [
            ("a".to_string(), state(&["inbox"], true, None)),
            ("b".to_string(), state(&["sent"], false, None)),
            ("c".to_string(), state(&[], false, Some(7))),
        ];
        let undo = round_trip(&before, &[Op::Snooze { ids: ids(&["a", "b", "c"]), until: 99 }]);
        assert_eq!(undo[0], Op::MoveToInbox { ids: ids(&["a", "b"]) });
        round_trip(&before, &[Op::MoveToInbox { ids: ids(&["c"]) }]);
    }

    #[test]
    fn moves_flags_and_untouched_messages() {
        let before = [("a".to_string(), state(&["inbox"], true, None)), ("b".to_string(), state(&["x"], false, None))];
        let ops = [
            Op::AddLabel { ids: ids(&["a", "b"]), label: "x".into() },
            Op::Archive { ids: ids(&["a"]) },
            Op::SetUnread { ids: ids(&["a"]), unread: false },
            Op::SetStarred { ids: ids(&["a", "b"]), starred: true },
        ];
        let undo = round_trip(&before, &ops);
        assert_eq!(undo.len(), 4, "{undo:?}");
    }

    #[test]
    fn big_undos_are_split_into_ops_of_a_bounded_size() {
        let before: Vec<(String, MessageState)> =
            (0..1200).map(|index| (index.to_string(), state(&["inbox"], false, None))).collect();
        let all: Vec<String> = before.iter().map(|(id, _)| id.clone()).collect();
        let undo = round_trip(&before, &[Op::Archive { ids: all }]);
        assert_eq!(undo.iter().map(|op| op.ids().len()).collect::<Vec<_>>(), [500, 500, 200]);
    }
}
