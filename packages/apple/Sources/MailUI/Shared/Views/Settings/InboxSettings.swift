import SwiftUI

/// Split Inbox on or off, and the user's own splits: their senders, their label, their order.
struct SplitSettings: View {
    @Environment(MailStore.self) private var store
    /// The split being written, new or not.
    @State private var editing: CustomSplit?

    var body: some View {
        let splits = CustomSplit.all(in: store.preferences)
        VStack(alignment: .leading, spacing: 10) {
            ToggleRow(
                title: "Split Inbox", detail: "Important mail first, then your splits, then Other: newsletters and notifications.",
                isOn: Binding(get: { store.preferences["split_inbox"]?.bool ?? false }, set: { store.setPreference("split_inbox", .bool($0)) })
            )
            ForEach(Array(splits.enumerated()), id: \.element.id) { index, split in
                if editing?.id == split.id {
                    SplitForm(split: split, labels: labels) { editing = nil }
                } else {
                    HStack(spacing: 8) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(split.name).font(.ui(14))
                            Text(summary(split)).font(.ui(12)).foregroundStyle(Tokens.mutedMoreForeground.color).lineLimit(1)
                        }
                        Spacer()
                        IconButton(symbol: .chevronUp, help: "Move up") { move(splits, from: index, by: -1) }
                            .disabled(index == 0)
                        IconButton(symbol: .chevronDown, help: "Move down") { move(splits, from: index, by: 1) }
                            .disabled(index == splits.count - 1)
                        IconButton(symbol: .pencil, help: "Edit") { editing = split }
                        IconButton(symbol: .trash, help: "Remove") { store.setPreference(split.key, nil) }
                    }
                }
            }
            if let editing, !splits.contains(where: { $0.id == editing.id }) {
                SplitForm(split: editing, labels: labels) { self.editing = nil }
            } else if editing == nil {
                ActionButton(title: "Add split", symbol: .plus, variant: .outline) {
                    let order = (splits.map(\.order).max() ?? -1) + 1
                    editing = CustomSplit(id: UUID().uuidString.lowercased(), name: "", from: [], label: nil, order: order)
                }
            }
        }
    }

    /// Every account's labels, by id.
    private var labels: [(id: String, name: String)] {
        store.accounts.flatMap { account in
            account.mailboxes.compactMap { mailbox -> (id: String, name: String)? in
                guard let id = MailStore.labelID(mailbox.id) else { return nil }
                return (id, store.accounts.count > 1 ? "\(mailbox.name) · \(account.address)" : mailbox.name)
            }
        }
    }

    private func summary(_ split: CustomSplit) -> String {
        var parts = split.from
        if let label = split.label, let name = labels.first(where: { $0.id == label })?.name { parts.append("label \(name)") }
        return parts.isEmpty ? "Nothing yet" : parts.joined(separator: ", ")
    }

    private func move(_ splits: [CustomSplit], from index: Int, by step: Int) {
        var reordered = splits
        reordered.swapAt(index, index + step)
        for (order, split) in reordered.enumerated() where split.order != order {
            var moved = split
            moved.order = order
            store.setPreference(moved.key, moved.value)
        }
    }
}

private struct SplitForm: View {
    @Environment(MailStore.self) private var store
    @State var split: CustomSplit
    let labels: [(id: String, name: String)]
    let done: () -> Void
    @State private var senders = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            InputField(label: "Name", text: $split.name, placeholder: "Team", autofocus: split.name.isEmpty)
            InputField(label: "From", text: $senders, placeholder: "ann@example.com, @example.com")
            if !labels.isEmpty {
                Picker("With the label", selection: $split.label) {
                    Text("None").tag(String?.none)
                    ForEach(labels, id: \.id) { label in Text(label.name).tag(String?.some(label.id)) }
                }
                .font(.ui(13))
                .fixedSize()
            }
            HStack(spacing: 8) {
                ActionButton(title: "Save", variant: .primary) {
                    split.name = split.name.trimmingCharacters(in: .whitespaces)
                    split.from = senders.split(whereSeparator: { ", ;\n".contains($0) }).map { $0.lowercased() }
                    store.setPreference(split.key, split.value)
                    done()
                }
                .disabled(split.name.trimmingCharacters(in: .whitespaces).isEmpty)
                ActionButton(title: "Cancel", variant: .ghost, action: done)
            }
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Tokens.card.color))
        .onAppear { senders = split.from.joined(separator: ", ") }
    }
}

/// The mailboxes shown as tabs over the list: which, and in what order.
struct TabSettings: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        let tabs = store.tabs
        VStack(alignment: .leading, spacing: 8) {
            if tabs.isEmpty {
                Text("No tabs. Mailboxes are opened from the sidebar.").font(.ui(13)).foregroundStyle(Tokens.mutedMoreForeground.color)
            }
            ForEach(Array(tabs.enumerated()), id: \.element.id) { index, tab in
                HStack(spacing: 8) {
                    Image(Symbol.named(tab.symbol), size: 14).foregroundStyle(Tokens.mutedMoreForeground.color).frame(width: 18)
                    Text(store.tabName(tab)).font(.ui(14)).lineLimit(1)
                    Spacer()
                    IconButton(symbol: .chevronUp, help: "Move up") { move(tabs, from: index, by: -1) }
                        .disabled(index == 0)
                    IconButton(symbol: .chevronDown, help: "Move down") { move(tabs, from: index, by: 1) }
                        .disabled(index == tabs.count - 1)
                    IconButton(symbol: .x, help: "Remove") { store.toggleTab(tab.id) }
                }
            }
            ActionMenu(title: "Add tab", symbol: .plus) {
                ForEach(store.unified.filter { !store.isTab($0.id) }) { mailbox in
                    item(mailbox)
                }
                ForEach(store.accounts) { account in
                    let left = account.mailboxes.filter { !store.isTab($0.id) }
                    if !left.isEmpty {
                        Menu(account.address) {
                            ForEach(left) { mailbox in item(mailbox) }
                        }
                    }
                }
            }
        }
    }

    private func item(_ mailbox: Mailbox) -> some View {
        Button { store.toggleTab(mailbox.id) } label: {
            Label { Text(mailbox.name) } icon: { Image(Symbol.named(mailbox.symbol), size: 15) }
        }
    }

    private func move(_ tabs: [Mailbox], from index: Int, by step: Int) {
        var ids = tabs.map(\.id)
        ids.swapAt(index, index + step)
        store.setTabs(ids)
    }
}

/// The senders and domains blocked, each with a way back.
struct BlockedSettings: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        let blocked = store.preferences.filter { $0.key.hasPrefix("blocked:") && $0.value.bool != false }
            .map { String($0.key.dropFirst("blocked:".count)) }
            .sorted()
        VStack(alignment: .leading, spacing: 8) {
            if blocked.isEmpty {
                Text("No one is blocked.").font(.ui(13)).foregroundStyle(Tokens.mutedMoreForeground.color)
            }
            ForEach(blocked, id: \.self) { address in
                HStack(spacing: 10) {
                    Image(.ban, size: 13).foregroundStyle(Tokens.mutedMoreForeground.color)
                    Text(address).font(.ui(14))
                    Spacer()
                    ActionButton(title: "Unblock", variant: .ghost) { store.setPreference("blocked:\(address)", nil) }
                }
            }
        }
    }
}

/// Whether new mail of each account makes a notification.
struct NotificationSettings: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(store.accounts) { account in
                ToggleRow(
                    title: account.address,
                    isOn: Binding(
                        get: { store.preferences["notify:\(account.id)"]?.bool ?? true },
                        set: { store.setPreference("notify:\(account.id)", $0 ? nil : .bool(false)) }
                    )
                )
            }
        }
    }
}

/// Whether designed mail is turned dark in dark mode, or left as its sender made it.
struct DarkMailSettings: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        ToggleRow(
            title: "Dark mode for mail",
            detail: "In dark mode, newsletters and other designed mail are drawn in dark colours. Each message can show the original.",
            isOn: Binding(
                get: { store.darkMail },
                set: { store.setPreference("dark_mail", $0 ? nil : .bool(false)) }
            )
        )
    }
}

/// Whether the images a message loads from the web are shown without asking.
struct ImageSettings: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        ToggleRow(
            title: "Load remote images",
            detail: "Off, pictures hosted by the sender wait until you ask for them, so nobody learns when you read their mail.",
            isOn: Binding(
                get: { store.preferences["remote_images"]?.bool ?? true },
                set: { store.setPreference("remote_images", $0 ? nil : .bool(false)) }
            )
        )
    }
}
