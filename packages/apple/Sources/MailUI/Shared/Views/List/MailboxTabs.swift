import SwiftUI

/// The mailboxes the user keeps as tabs over the list. Dragging one moves it; a secondary click
/// takes it off. Which they are is the `tabs` preference, so every device shows the same.
struct MailboxTabs: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        PillTabs(
            tabs: store.tabs, selected: store.searchRows == nil ? store.baseMailbox : nil,
            symbol: { Symbol.named($0.symbol) }, title: store.tabName, count: \.unread,
            pick: { store.select(mailbox: $0.id) }, reorder: store.setTabs
        ) { tab in
            Button("Remove from Tabs") { store.toggleTab(tab.id) }
        }
    }
}
