import SwiftUI

/// The mailboxes of every account together, then each account with its colour, then settings.
/// The mailboxes are dragged into the order wanted, and so are the accounts; both orders are
/// synced, so every device shows the same.
struct SidebarView: View {
    @Environment(MailStore.self) private var store
    @State private var expanded: Set<String> = []
    var showSettings: () -> Void
    /// Called after a mailbox is picked, for iOS to show it.
    var picked: (String) -> Void = { _ in }

    /// A row: a mailbox of every account together, the accounts' heading, an account, or a
    /// mailbox under an account that is open.
    private enum Row: Identifiable {
        case unified(Mailbox)
        case heading
        case account(AccountView)
        case mailbox(Mailbox)

        var id: String {
            switch self {
            case .unified(let mailbox), .mailbox(let mailbox): mailbox.id
            case .heading: "accounts"
            case .account(let account): "account:\(account.id)"
            }
        }

        /// The rows that change places with each other.
        var group: String? {
            switch self {
            case .unified: "mailboxes"
            case .account: "accounts"
            case .heading, .mailbox: nil
            }
        }

        /// A mailbox under an account goes along with it.
        var follows: Bool {
            guard case .mailbox = self else { return false }
            return true
        }
    }

    private var rows: [Row] {
        var rows = store.unified.map(Row.unified)
        if !store.accounts.isEmpty { rows.append(.heading) }
        for account in store.accounts {
            rows.append(.account(account))
            guard expanded.contains(account.id) else { continue }
            rows += account.mailboxes.map(Row.mailbox)
        }
        return rows
    }

    #if os(iOS)
    private static let indicators = false
    #else
    private static let indicators = true
    #endif

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            MovableList(
                items: rows, inset: Space.s, group: \.group, follows: \.follows,
                rowInset: EdgeInsets(top: Theme.buttonGap / 2, leading: 0, bottom: Theme.buttonGap / 2, trailing: 0), indicators: Self.indicators,
                clicked: clicked, moved: moved, menu: menu
            ) { row in
                switch row {
                case .unified(let mailbox): self.row(mailbox)
                case .mailbox(let mailbox): self.row(mailbox, indent: Space.l)
                case .account(let account): accountRow(account)
                case .heading:
                    SectionHeading(title: "Accounts")
                        .padding(.horizontal, 18)
                        .padding(.top, Space.xl + 2)
                        .padding(.bottom, Space.xs + 2)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            footer
        }
        .background(Tokens.background.color)
    }

    private func row(_ mailbox: Mailbox, indent: CGFloat = 0) -> some View {
        NavRow(
            title: mailbox.name, symbol: Symbol.named(mailbox.symbol), count: mailbox.unread,
            selected: store.baseMailbox == mailbox.id && store.searchRows == nil, indent: indent
        )
    }

    private func accountRow(_ account: AccountView) -> some View {
        NavRow(title: account.address, dot: Theme.accountColor(account.color).color, action: nil) {
            if account.status == "syncing" {
                ProgressView().controlSize(.mini)
            } else if account.status == "reauth" || account.status == "error" {
                Image(.circleAlert, size: 13).foregroundStyle(Tokens.destructive.color)
            }
            Image(expanded.contains(account.id) ? .chevronDown : .chevronRight, size: 12).foregroundStyle(Tokens.mutedMostForeground.color)
        }
    }

    private func clicked(_ row: Row) {
        switch row {
        case .unified(let mailbox), .mailbox(let mailbox):
            store.select(mailbox: mailbox.id)
            picked(mailbox.id)
        case .account(let account):
            if expanded.contains(account.id) { expanded.remove(account.id) } else { expanded.insert(account.id) }
        case .heading:
            break
        }
    }

    private func menu(_ row: Row) -> [RowMenuItem] {
        switch row {
        case .unified(let mailbox), .mailbox(let mailbox):
            [RowMenuItem(title: store.isTab(mailbox.id) ? "Remove from Tabs" : "Add to Tabs") { store.toggleTab(mailbox.id) }]
        case .account, .heading:
            []
        }
    }

    /// The row landed where `index` is among the rows without it and the mailboxes under it:
    /// after the rows of its kind before that.
    private func moved(_ row: Row, to index: Int) {
        let rows = rows
        guard let from = rows.firstIndex(where: { $0.id == row.id }) else { return }
        let lifted = Landings.lifted(from: from, follows: rows.map(\.follows))
        let before = rows.indices.filter { !lifted.contains($0) }.prefix(index).map { rows[$0] }
        switch row {
        case .unified(let mailbox):
            let ids = before.compactMap { if case .unified(let other) = $0 { other.id } else { nil } }
            store.setSidebarOrder(mailboxes: SidebarOrder.placed(mailbox.id, after: ids, among: store.unified.map(\.id)))
        case .account(let account):
            let ids = before.compactMap { if case .account(let other) = $0 { other.id } else { nil } }
            store.setSidebarOrder(accounts: SidebarOrder.placed(account.id, after: ids, among: store.accounts.map(\.id)))
        case .heading, .mailbox:
            break
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
