import SwiftUI

/// The mailboxes the user keeps as tabs over the list. Dragging one moves it; a secondary click
/// takes it off. Which they are is the `tabs` preference, so every device shows the same; in one
/// account's mailboxes, they are that account's.
struct MailboxTabs: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        PillTabs(
            tabs: store.tabs, selected: store.searchRows == nil ? store.baseMailbox : nil,
            symbol: { Symbol.named($0.symbol) }, title: { store.tabName($0, shownIn: store.listAccount) }, count: \.unread,
            pick: { store.select(mailbox: $0.id) }, reorder: { store.setTabs($0.map(store.savedTab)) },
            menu: { tab in [RowMenuItem(title: "Remove from Tabs") { store.toggleTab(store.savedTab(tab.id)) }] }
        )
    }
}
