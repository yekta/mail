import SwiftUI

/// Over the list: the split inbox's tabs, and the filter the list is narrowed to.
struct ListHeader: View {
    @Environment(MailStore.self) private var store

    static let height = 38 * Platform.scale

    static func shows(_ store: MailStore) -> Bool {
        store.searchRows == nil && (!store.splits.isEmpty || store.filter != nil)
    }

    var body: some View {
        if Self.shows(store) {
            HStack(spacing: Space.m) {
                ScrollView(.horizontal, showsIndicators: false) {
                    TabStrip(
                        tabs: store.splits, selected: chosenSplit, title: \.name, count: \.unread,
                        pick: { store.select(mailbox: $0.mailbox) }
                    )
                    .padding(.horizontal, Space.l)
                }
                if let filter = store.filter {
                    Chip(title: filter == .unread ? "Unread" : "Starred", symbol: .listFilter, help: "Show everything", remove: { store.setFilter(nil) })
                        .padding(.trailing, Space.l)
                }
            }
            .frame(height: Self.height)
            .background(Tokens.background.color)
            .rule(.bottom)
        }
    }

    /// The tab on screen: the one asked for, else the first, which the core shows for the inbox.
    private var chosenSplit: String? {
        store.splits.first(where: { $0.mailbox == store.mailbox })?.mailbox ?? store.splits.first?.mailbox
    }
}

/// An empty mailbox: a calm line instead of a list.
struct EmptyList: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        EmptyState(symbol: store.searchRows != nil ? .search : .inbox, message: message)
    }

    private var message: String {
        if store.searchRows != nil { return "Nothing matches." }
        if store.filter == .unread || store.inUnread { return "Nothing unread." }
        if store.filter == .starred { return "Nothing starred." }
        if store.connection == "connecting" && store.accounts.isEmpty { return "Syncing…" }
        return store.baseMailbox.hasSuffix("inbox") ? "All done. Enjoy the quiet." : "Nothing here."
    }
}

/// The threads picked on the Mac, with what can be done to them all.
struct SelectionBar: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        let threads = Array(store.selection)
        HStack(spacing: 0) {
            Text("\(store.selection.count) selected").padding(.trailing, Space.s).textStyle(.labelStrong)
            ActionButton(title: "Clear", variant: .ghost, action: store.clearSelection)
            Spacer()
            ForEach(store.applicable([.archive, .trash, .snooze, .label, .read], to: threads)) { command in
                IconButton(symbol: command.symbol, help: command.title(store, threads)) { store.run(command, on: threads) }
            }
            let more = store.applicable(ThreadCommand.more.filter { $0 != .print }, to: threads)
            if !more.isEmpty {
                IconMenu(symbol: .ellipsis, help: "More") {
                    ThreadCommandButtons(commands: more, threads: threads)
                }
            }
        }
    }
}
