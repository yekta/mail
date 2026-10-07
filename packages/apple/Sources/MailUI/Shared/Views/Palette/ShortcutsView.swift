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
    @Environment(MailStore.self) private var store

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("Keyboard shortcuts").font(.ui(17, .semibold)).foregroundStyle(Tokens.foreground.color)
                Spacer()
                IconButton(symbol: .x, help: "Close", circled: false) { store.shortcutsOpen = false }
                    .keyboardShortcut(.cancelAction)
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 12)
            Rectangle().fill(Tokens.border.color).frame(height: 1)
            ScrollView {
                VStack(alignment: .leading, spacing: 22) {
                    ForEach(Shortcuts.sections, id: \.0) { title, keys in
                        VStack(alignment: .leading, spacing: 6) {
                            Text(title.uppercased())
                                .font(.ui(10.5, .semibold))
                                .foregroundStyle(Tokens.mutedForeground.color)
                            ForEach(keys, id: \.0) { name, key in
                                HStack {
                                    Text(name).font(.ui(13)).foregroundStyle(Tokens.foreground.color)
                                    Spacer()
                                    Text(key).font(.system(size: 12 * Platform.scale, design: .monospaced)).foregroundStyle(Tokens.secondaryForeground.color)
                                }
                            }
                        }
                    }
                }
                .padding(20)
            }
        }
        #if os(macOS)
        .frame(width: 480, height: 560)
        #endif
        .background(Tokens.popover.color)
    }
}
