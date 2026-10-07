import SwiftUI

/// Over the list: the split inbox's tabs, and the filter the list is narrowed to.
struct ListHeader: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        if store.searchRows == nil, !store.splits.isEmpty || store.filter != nil {
            HStack(spacing: 12) {
                ScrollView(.horizontal, showsIndicators: false) {
                    TabStrip(
                        tabs: store.splits, selected: chosenSplit, title: \.name, count: \.unread,
                        pick: { store.select(mailbox: $0.mailbox) }
                    )
                    .padding(.horizontal, 16)
                }
                if let filter = store.filter {
                    Chip(title: filter == .unread ? "Unread" : "Starred", symbol: .listFilter, help: "Show everything", remove: { store.setFilter(nil) })
                        .padding(.trailing, 16)
                }
            }
            .frame(height: 38 * Platform.scale)
            .background(Tokens.card.color)
            .overlay(alignment: .bottom) { Rectangle().fill(Tokens.border.color).frame(height: 1) }
        }
    }

    /// The tab on screen: the one asked for, else the first, which the core shows for the inbox.
    private var chosenSplit: String? {
        store.splits.first(where: { $0.mailbox == store.mailbox })?.mailbox ?? store.splits.first?.mailbox
    }
}

/// The threads picked on the Mac, with what can be done to them all.
struct SelectionBar: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        let threads = Array(store.selection)
        HStack(spacing: 8) {
            Text("\(store.selection.count) selected").font(.ui(13, .medium)).foregroundStyle(Tokens.foreground.color)
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
