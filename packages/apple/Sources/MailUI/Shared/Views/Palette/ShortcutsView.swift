import SwiftUI

/// The Mac's keys, as the shortcuts sheet lists them.
enum Shortcuts {
    /// `g` then a key: where it goes, by unified mailbox.
    static let go: [String: String] = [
        "inbox": "G I", "starred": "G S", "sent": "G T", "drafts": "G D", "archive": "G A", "snoozed": "G H", "spam": "G !",
        "trash": "G #",
    ]

    static let sections: [(String, [(String, String)])] = [
        (
            "Moving around",
            [
                ("Next or previous thread", "J  K  ↓  ↑"), ("Open", "Enter  O"), ("Back, clear the selection or the search", "Esc"),
                ("Next or previous message", "N  P"), ("Expand every message", "⇧O"), ("Next or previous split", "Tab  ⇧Tab"),
                ("Search", "/"), ("Command palette", "⌘K"),
            ]
        ),
        (
            "Selecting",
            [("Select", "X"), ("Select the next or previous too", "⇧J  ⇧K"), ("Select all", "⌘A"), ("Clear the selection", "Esc")]
        ),
        (
            "Acting",
            [
                ("Archive", "E"), ("Move to Inbox", "⇧E"), ("Trash", "#  Delete"), ("Spam", "!"), ("Star", "S"),
                ("Mark read or unread", "U"), ("Mark read", "⇧I"), ("Snooze or remind me", "H"), ("Label", "L"), ("Move to", "V"),
                ("Mute", "⇧M"), ("Unsubscribe", "⌘U"), ("Undo", "Z  ⌘Z"), ("Print", "⌘P"),
            ]
        ),
        (
            "Writing",
            [("Reply", "R"), ("Reply all", "A  Enter in a thread"), ("Forward", "F"), ("New message", "C  ⌘N")]
        ),
        (
            "Going to",
            [
                ("Inbox", "G I"), ("Starred", "G S"), ("Sent", "G T"), ("Drafts", "G D"), ("Archive", "G A"), ("Snoozed", "G H"),
                ("Spam", "G !"), ("Trash", "G #"), ("A label", "G L"), ("All inboxes", "⌘0"), ("An account's inbox", "⌘1 … ⌘9"),
                ("Unread only", "⇧U"), ("Starred only", "⇧S"), ("Settings", "⌘,"), ("These shortcuts", "?"),
            ]
        ),
    ]
}

/// `?`: every key, in a sheet.
struct ShortcutsView: View {
    var body: some View {
        Sheet(title: "Keyboard shortcuts", size: .large) {
            ScrollView {
                VStack(alignment: .leading, spacing: Space.xxl) {
                    ForEach(Shortcuts.sections, id: \.0) { title, keys in
                        FormSection(title: title) {
                            VStack(alignment: .leading, spacing: Space.s) {
                                ForEach(keys, id: \.0) { name, key in
                                    HStack {
                                        Text(name).textStyle(.label)
                                        Spacer()
                                        Text(key).textStyle(.mono)
                                    }
                                }
                            }
                        }
                    }
                }
                .padding(Space.xl)
            }
        }
    }
}
