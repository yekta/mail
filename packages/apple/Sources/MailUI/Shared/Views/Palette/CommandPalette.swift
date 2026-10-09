import SwiftUI

/// One thing the palette can do.
struct PaletteItem: Identifiable {
    let id: String
    let title: String
    let symbol: Symbol
    var shortcut: String?
    /// Opens a sheet or asks a question of its own, so it waits for the palette to be gone.
    var afterClosing = false
    let perform: @MainActor () -> Void
}

/// ⌘K: everything that can be done to the threads at hand, and everywhere to go, found by typing
/// a few of the letters of its name.
struct CommandPalette: View {
    @Environment(MailStore.self) private var store
    let scope: PaletteScope
    @State private var query = ""
    @State private var highlighted = 0
    @State private var snoozeChoices: [TimeChoice] = []

    var body: some View {
        let found = Self.filter(items, by: query)
        Sheet(title: scope == .labels ? "Go to a label" : "Commands", size: .medium) {
            VStack(spacing: 0) {
                SearchField(
                    placeholder: scope == .labels ? "Go to a label" : "Type a command or a place to go", text: $query,
                    submit: { run(found, highlighted) }, move: { highlighted = highlighted.moved(by: $0, in: found.count) },
                    escape: { store.palette = nil }
                )
                ChoiceList(items: found, highlighted: highlighted, empty: "Nothing by that name.") { index, item in
                    ChoiceRow(title: item.title, detail: item.shortcut, symbol: item.symbol, highlighted: index == highlighted) {
                        run(found, index)
                    }
                }
            }
        }
        .onChange(of: query) { highlighted = 0 }
        .task {
            guard scope == .everything, !store.targets.isEmpty else { return }
            snoozeChoices = await store.parseTime("")
        }
    }

    private func run(_ found: [PaletteItem], _ index: Int) {
        guard found.indices.contains(index) else { return }
        let item = found[index]
        store.palette = nil
        guard item.afterClosing else {
            item.perform()
            return
        }
        Task {
            try? await Task.sleep(for: .milliseconds(350))
            item.perform()
        }
    }

    // MARK: Items

    private var items: [PaletteItem] {
        guard scope == .everything else { return labelItems }
        return threadItems + generalItems + mailboxItems + labelItems
    }

    private var threadItems: [PaletteItem] {
        let store = store
        let threads = store.targets
        guard !threads.isEmpty else { return [] }
        let many = threads.count > 1
        let opensElsewhere: Set<ThreadCommand> = [.reply, .replyAll, .forward, .block]
        let commands = store.applicable(ThreadCommand.allCases.filter { !many || ![.reply, .replyAll, .forward, .print].contains($0) }, to: threads)
        let suffix = many ? " \(threads.count) threads" : ""
        var items = commands.map { command in
            PaletteItem(
                id: "command/\(command.rawValue)", title: command.title(store, threads) + suffix, symbol: command.symbol,
                shortcut: command.shortcut, afterClosing: opensElsewhere.contains(command)
            ) { store.run(command, on: threads) }
        }
        let verb = store.remindsInsteadOfSnoozing ? "Remind me" : "Snooze"
        guard commands.contains(.snooze) else { return items }
        items += snoozeChoices.map { choice in
            PaletteItem(id: "snooze/\(choice.id)", title: "\(verb): \(choice.name) · \(choice.label)", symbol: .clock) {
                store.act(.snooze, on: threads, until: choice.date)
            }
        }
        return items
    }

    private var generalItems: [PaletteItem] {
        let store = store
        var items = [
            PaletteItem(id: "compose", title: "Compose", symbol: .squarePen, shortcut: "C", afterClosing: true) { store.newMessage() },
            PaletteItem(id: "undo", title: "Undo", symbol: .undo, shortcut: "Z") { store.undo() },
            PaletteItem(id: "check-mail", title: "Check for new mail", symbol: .refreshCw) { Task { await store.checkMail() } },
            PaletteItem(id: "unread", title: store.filter == .unread ? "Show everything" : "Unread only", symbol: .listFilter, shortcut: "⇧U") {
                store.toggleFilter(.unread)
            },
            PaletteItem(id: "starred", title: store.filter == .starred ? "Show everything" : "Starred only", symbol: .star, shortcut: "⇧S") {
                store.toggleFilter(.starred)
            },
        ]
        if store.searchRows == nil, !store.rows.isEmpty {
            items.append(PaletteItem(id: "zero", title: "Get Me To Zero: archive everything here", symbol: .archive, afterClosing: true) { store.archiveAll() })
        }
        if store.conversation == nil, !store.visibleRows.isEmpty {
            items.append(PaletteItem(id: "select-all", title: "Select all", symbol: .squareCheck, shortcut: "⌘A") { store.selectAll() })
        }
        if !store.selection.isEmpty {
            items.append(PaletteItem(id: "select-none", title: "Clear the selection", symbol: .x, shortcut: "Esc") { store.clearSelection() })
        }
        if store.splits.count > 1 {
            items.append(PaletteItem(id: "split-next", title: "Next split", symbol: .chevronRight, shortcut: "Tab") { store.moveSplit(1) })
            items.append(PaletteItem(id: "split-previous", title: "Previous split", symbol: .chevronLeft, shortcut: "⇧Tab") { store.moveSplit(-1) })
        }
        items += [
            PaletteItem(id: "settings", title: "Settings", symbol: .settings, shortcut: "⌘,", afterClosing: true) { store.settingsOpen = true },
            PaletteItem(id: "shortcuts", title: "Keyboard shortcuts", symbol: .keyboard, shortcut: "?", afterClosing: true) { store.shortcutsOpen = true },
            PaletteItem(id: "gallery", title: "Component gallery", symbol: .monitor, afterClosing: true) { store.galleryOpen = true },
        ]
        return items
    }

    private var mailboxItems: [PaletteItem] {
        let store = store
        var items = store.unified.map { mailbox in
            PaletteItem(id: "go/\(mailbox.id)", title: "Go to \(mailbox.name)", symbol: Symbol.named(mailbox.symbol), shortcut: Shortcuts.go[mailbox.id]) {
                store.select(mailbox: mailbox.id)
            }
        }
        for (index, account) in store.accounts.enumerated() {
            items.append(PaletteItem(id: "go/\(account.id)", title: "Go to \(account.address)", symbol: .inbox, shortcut: index < 9 ? "⌘\(index + 1)" : nil) {
                store.select(mailbox: "\(account.id)/inbox")
            })
            for mailbox in account.mailboxes where MailStore.labelID(mailbox.id) == nil && !mailbox.id.hasSuffix("/inbox") {
                items.append(PaletteItem(id: "go/\(mailbox.id)", title: "Go to \(mailbox.name) · \(account.address)", symbol: Symbol.named(mailbox.symbol)) {
                    store.select(mailbox: mailbox.id)
                })
            }
        }
        return items
    }

    private var labelItems: [PaletteItem] {
        let store = store
        let several = store.accounts.count > 1
        return store.accounts.flatMap { account in
            store.labels(of: account.id).map { label in
                PaletteItem(id: "go/\(label.id)", title: several ? "\(label.name) · \(account.address)" : label.name, symbol: .tag) {
                    store.select(mailbox: label.id)
                }
            }
        }
    }

    // MARK: Matching

    /// The items whose titles have the letters typed, in order, best first.
    static func filter(_ items: [PaletteItem], by query: String) -> [PaletteItem] {
        let typed = Array(query.lowercased().filter { !$0.isWhitespace })
        guard !typed.isEmpty else { return items }
        return items
            .compactMap { item in score(typed, Array(item.title.lowercased())).map { (item, $0) } }
            .enumerated()
            .sorted { $0.element.1 != $1.element.1 ? $0.element.1 > $1.element.1 : $0.offset < $1.offset }
            .map(\.element.0)
    }

    /// How well the letters match: more when they start words or follow each other. Nil when
    /// they don't all appear in order.
    static func score(_ typed: [Character], _ title: [Character]) -> Int? {
        var score = 0
        var next = 0
        var previous = -2
        for (index, character) in title.enumerated() where next < typed.count && character == typed[next] {
            let startsWord = index == 0 || !title[index - 1].isLetter
            score += 1 + (startsWord ? 3 : 0) + (index == previous + 1 ? 2 : 0)
            previous = index
            next += 1
        }
        return next == typed.count ? score : nil
    }
}
