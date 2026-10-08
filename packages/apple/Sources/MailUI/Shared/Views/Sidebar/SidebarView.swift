import SwiftUI

/// The mailboxes of every account together, then each account with its colour, then settings.
struct SidebarView: View {
    @Environment(MailStore.self) private var store
    @State private var expanded: Set<String> = []
    var showSettings: () -> Void
    /// Called after a mailbox is picked, for iOS to show it.
    var picked: (String) -> Void = { _ in }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 1) {
                    ForEach(store.unified) { mailbox in
                        row(mailbox)
                    }
                    if !store.accounts.isEmpty {
                        SectionHeading(title: "Accounts")
                            .padding(.horizontal, 18)
                            .padding(.top, Space.xl + 2)
                            .padding(.bottom, Space.xs + 2)
                    }
                    ForEach(store.accounts) { account in
                        accountRow(account)
                        if expanded.contains(account.id) {
                            ForEach(account.mailboxes) { mailbox in
                                row(mailbox, indent: Space.l)
                            }
                        }
                    }
                }
                .padding(.vertical, Space.s)
            }
            #if os(iOS)
            .scrollIndicators(.hidden)
            #endif
            footer
        }
        .background(Tokens.background.color)
    }

    private func row(_ mailbox: Mailbox, indent: CGFloat = 0) -> some View {
        NavRow(
            title: mailbox.name, symbol: Symbol.named(mailbox.symbol), count: mailbox.unread,
            selected: store.baseMailbox == mailbox.id && store.searchRows == nil, indent: indent
        ) {
            store.select(mailbox: mailbox.id)
            picked(mailbox.id)
        }
        .contextMenu {
            Button(store.isTab(mailbox.id) ? "Remove from Tabs" : "Add to Tabs") { store.toggleTab(mailbox.id) }
        }
    }

    private func accountRow(_ account: AccountView) -> some View {
        NavRow(title: account.address, dot: Theme.accountColor(account.color).color) {
            if expanded.contains(account.id) { expanded.remove(account.id) } else { expanded.insert(account.id) }
        } trailing: {
            if account.status == "syncing" {
                ProgressView().controlSize(.mini)
            } else if account.status == "reauth" || account.status == "error" {
                Image(.circleAlert, size: 13).foregroundStyle(Tokens.destructive.color)
            }
            Image(expanded.contains(account.id) ? .chevronDown : .chevronRight, size: 12).foregroundStyle(Tokens.mutedMostForeground.color)
        }
    }

    /// A new version of the Mac app as it comes in, then settings.
    private var footer: some View {
        VStack(spacing: 0) {
            #if os(macOS)
            if store.updater.state != .idle {
                UpdateRow(updater: store.updater)
                    .padding(.leading, 18)
                    .padding(.trailing, Space.l)
                    .padding(.vertical, Space.s)
            }
            #endif
            NavRow(title: "Settings", symbol: .settings, verticalPadding: 2, action: showSettings) {
                if store.connection == "offline" {
                    Image(.wifiOff, size: 14).foregroundStyle(Tokens.mutedMoreForeground.color).help("Offline")
                }
            }
        }
        .rule(.top)
    }
}
