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
                        row(mailbox, symbol: Symbol.named(mailbox.symbol))
                    }
                    if !store.accounts.isEmpty {
                        Text("ACCOUNTS")
                            .font(.ui(10.5, .semibold))
                            .foregroundStyle(Tokens.mutedMostForeground.color)
                            .padding(.horizontal, 18)
                            .padding(.top, 22)
                            .padding(.bottom, 6)
                    }
                    ForEach(store.accounts) { account in
                        accountRow(account)
                        if expanded.contains(account.id) {
                            ForEach(account.mailboxes) { mailbox in
                                row(mailbox, symbol: Symbol.named(mailbox.symbol), indent: 16)
                            }
                        }
                    }
                }
                .padding(.vertical, 8)
            }
            footer
        }
        .background(Tokens.card.color)
    }

    private func row(_ mailbox: Mailbox, symbol: Symbol, indent: CGFloat = 0) -> some View {
        let selected = store.baseMailbox == mailbox.id && store.searchRows == nil
        return Button {
            store.select(mailbox: mailbox.id)
            picked(mailbox.id)
        } label: {
            HStack(spacing: 12) {
                Image(symbol, size: 15).frame(width: 18)
                Text(mailbox.name).font(.ui(13.5, selected ? .medium : .regular)).lineLimit(1)
                Spacer()
                if mailbox.unread > 0 {
                    Text("\(mailbox.unread)").font(.ui(12)).foregroundStyle(Tokens.mutedMoreForeground.color)
                }
            }
            .foregroundStyle(selected ? Tokens.accentForeground.color : Tokens.foreground.color)
            .padding(.leading, 18 + indent)
            .padding(.trailing, 16)
            .frame(height: 34 * Platform.scale)
            .background(selected ? Tokens.accent.color : .clear)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu {
            Button(store.isTab(mailbox.id) ? "Remove from Tabs" : "Add to Tabs") { store.toggleTab(mailbox.id) }
        }
    }

    private func accountRow(_ account: AccountView) -> some View {
        Button {
            if expanded.contains(account.id) { expanded.remove(account.id) } else { expanded.insert(account.id) }
        } label: {
            HStack(spacing: 12) {
                Circle().fill(Theme.accountColor(account.color).color).frame(width: 8, height: 8).frame(width: 18)
                Text(account.address).font(.ui(13)).lineLimit(1).truncationMode(.middle)
                Spacer()
                if account.status == "syncing" {
                    ProgressView().controlSize(.mini)
                } else if account.status == "reauth" || account.status == "error" {
                    Image(.circleAlert, size: 13).foregroundStyle(Tokens.destructive.color)
                }
                Image(expanded.contains(account.id) ? .chevronDown : .chevronRight, size: 12).foregroundStyle(Tokens.mutedMostForeground.color)
            }
            .foregroundStyle(Tokens.foreground.color)
            .padding(.leading, 18)
            .padding(.trailing, 16)
            .frame(height: 34 * Platform.scale)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private var footer: some View {
        HStack(spacing: 10) {
            Button(action: showSettings) {
                HStack(spacing: 10) {
                    Image(.settings, size: 15)
                    Text("Settings").font(.ui(13))
                }
                .foregroundStyle(Tokens.foreground.color)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            Spacer()
            if store.connection == "offline" {
                Image(.wifiOff, size: 14).foregroundStyle(Tokens.mutedMoreForeground.color).help("Offline")
            }
        }
        .padding(.horizontal, 18)
        .frame(height: 48)
        .overlay(alignment: .top) { Rectangle().fill(Tokens.border.color).frame(height: 1) }
    }
}
