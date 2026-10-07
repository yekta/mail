//! The words of a thread row: who wrote, as "Alice, me (3)".

use std::collections::HashSet;

use mail_protocol::Address;

/// The thread's senders in the order they first wrote, "me" for the user's own addresses, first
/// names when there are several, with the number of messages when there is more than one.
pub fn senders(from: &[Address], me: &HashSet<String>) -> String {
    let mut people: Vec<&Address> = Vec::new();
    for address in from {
        if !people.iter().any(|known| known.email == address.email) {
            people.push(address);
        }
    }
    let name = |address: &Address, short: bool| -> String {
        if me.contains(&address.email) {
            return "me".into();
        }
        let full = address.display();
        match short {
            true => full.split_whitespace().next().unwrap_or(full).to_string(),
            false => full.to_string(),
        }
    };
    let short = people.len() > 1;
    let names: Vec<String> = match people.len() {
        0 => vec!["(no sender)".into()],
        1..=3 => people.iter().map(|address| name(address, short)).collect(),
        count => vec![name(people[0], true), "…".into(), name(people[count - 2], true), name(people[count - 1], true)],
    };
    let mut text = names.join(", ").replace(", …,", " …");
    if from.len() > 1 {
        text.push_str(&format!(" ({})", from.len()));
    }
    text
}

/// "Roger & me" under a thread's subject: everyone in it, not only who wrote.
pub fn participants(people: &[Address], me: &HashSet<String>) -> String {
    let mut names: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    let mut includes_me = false;
    for address in people {
        if !seen.insert(address.email.clone()) {
            continue;
        }
        match me.contains(&address.email) {
            true => includes_me = true,
            false => names.push(address.display().split_whitespace().next().unwrap_or(address.display()).to_string()),
        }
    }
    if includes_me {
        names.push("me".into());
    }
    match names.len() {
        0 => String::new(),
        1 => names.remove(0),
        _ => {
            let last = names.pop().unwrap_or_default();
            format!("{} & {last}", names.join(", "))
        }
    }
}

/// Two letters for an avatar: the first letters of the first and last name.
pub fn initials(address: &Address) -> String {
    let words: Vec<&str> = address
        .display()
        .split(|c: char| c.is_whitespace() || c == '.' || c == '_')
        .filter(|word| !word.is_empty())
        .collect();
    let letter = |word: &&str| {
        word.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string()).unwrap_or_default()
    };
    match words.as_slice() {
        [] => "?".into(),
        [one] => letter(one),
        [first, .., last] => format!("{}{}", letter(first), letter(last)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str, email: &str) -> Address {
        Address::new(Some(name), email)
    }

    fn me() -> HashSet<String> {
        HashSet::from(["me@example.com".to_string()])
    }

    #[test]
    fn one_sender_keeps_the_full_name() {
        assert_eq!(senders(&[person("Alice Smith", "a@x.com")], &me()), "Alice Smith");
        assert_eq!(
            senders(&[person("Alice Smith", "a@x.com"), person("Alice Smith", "a@x.com")], &me()),
            "Alice Smith (2)"
        );
    }

    #[test]
    fn several_senders_use_first_names_and_me() {
        let thread =
            [person("Alice Smith", "a@x.com"), Address::new(None, "me@example.com"), person("Alice Smith", "a@x.com")];
        assert_eq!(senders(&thread, &me()), "Alice, me (3)");
    }

    #[test]
    fn many_senders_keep_the_first_and_the_last_two() {
        let thread = [
            person("Ann Lee", "a@x.com"),
            person("Bob Ray", "b@x.com"),
            person("Cy Twombly", "c@x.com"),
            person("Di Fox", "d@x.com"),
            person("Ed Wood", "e@x.com"),
        ];
        assert_eq!(senders(&thread, &me()), "Ann … Di, Ed (5)");
    }

    #[test]
    fn participants_end_with_me() {
        let people = [Address::new(None, "me@example.com"), person("Roger Cotes", "r@x.com")];
        assert_eq!(participants(&people, &me()), "Roger & me");
        assert_eq!(participants(&[person("Roger Cotes", "r@x.com")], &me()), "Roger");
    }

    #[test]
    fn initials_take_the_first_and_last_name() {
        assert_eq!(initials(&person("Ada King Lovelace", "ada@x.com")), "AL");
        assert_eq!(initials(&Address::new(None, "grace.hopper@navy.mil")), "GH");
        assert_eq!(initials(&Address::new(None, "x@y.z")), "X");
    }
}
