import SwiftUI

/// Accounts, signatures, snippets, Split Inbox, blocked senders, notifications, appearance, the
/// undo delay and the server. What is synced is kept as the core's preferences.
struct SettingsView: View {
    @Environment(MailStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var adding = false

    var body: some View {
        @Bindable var store = store
        Sheet(title: "Settings", size: .large, background: Tokens.background) {
            ScrollView {
                VStack(alignment: .leading, spacing: Space.xxl) {
                    FormSection(title: "Accounts") {
                        ForEach(store.accounts) { account in
                            ItemRow(title: account.address, detail: status(account), dot: Theme.accountColor(account.color).color) {
                                ActionButton(title: "Remove", variant: .destructive) { store.removeAccount(account.id) }
                            }
                            SwatchRow(selected: account.color) { store.setAccountColor(account.id, $0) }
                        }
                        if adding {
                            Card { AddAccountForm() }
                        } else {
                            ActionButton(title: "Add account", symbol: .plus, variant: .outline) { adding = true }
                        }
                    }
                    if store.preferencesLoaded {
                        FormSection(title: "Signatures", detail: "Put under what you write from each account.") { SignatureSettings() }
                        FormSection(title: "Snippets", detail: "Type ; and a name while writing to put one in, or press ⌘;. {first_name} becomes the first recipient's first name.") {
                            SnippetSettings()
                        }
                        FormSection(title: "Tabs", detail: "The mailboxes kept over the list. Drag a tab to move it.") { TabSettings() }
                        FormSection(title: "Split Inbox") { SplitSettings() }
                        FormSection(title: "Blocked senders", detail: "Their new mail goes to the trash.") { BlockedSettings() }
                        FormSection(title: "Images") { ImageSettings() }
                        FormSection(title: "Notifications") { NotificationSettings() }
                    }
                    FormSection(title: "Appearance") {
                        Segmented(options: Appearance.allCases, selection: $store.appearance, title: \.name)
                        DarkMailSettings()
                    }
                    FormSection(title: "Undo send") {
                        Segmented(options: [0, 5, 10, 20, 30], selection: $store.undoDelay) { $0 == 0 ? "Off" : "\($0) s" }
                    }
                    FormSection(title: "Server") {
                        Text(store.server).textStyle(.label, color: Tokens.mutedForeground.color).textSelection(.enabled)
                        Notice(text: connectionText)
                    }
                    #if os(macOS)
                    FormSection(title: "Updates", detail: "New versions are downloaded and installed on their own; a restart finishes them.") {
                        UpdateSettings()
                    }
                    #else
                    FormSection(title: "Version") {
                        Text("Wonnet \(Platform.version)").textStyle(.label, color: Tokens.mutedForeground.color)
                    }
                    #endif
                    ActionButton(title: "Sign out", symbol: .logOut, variant: .destructive) {
                        store.signOut()
                        dismiss()
                    }
                }
                .padding(Space.xxl)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .task { await store.loadPreferences() }
    }

    private var connectionText: String {
        switch store.connection {
        case "online": "Connected"
        case "connecting": "Connecting…"
        default: "Offline. Changes wait and are sent once connected."
        }
    }

    private func status(_ account: AccountView) -> String {
        switch account.status {
        case "syncing": "Syncing…"
        case "reauth": "Sign in again: remove the account and add it back"
        case "error": "Can't reach the provider right now"
        default: account.provider == "gmail" ? "Gmail" : "JMAP"
        }
    }
}

#if os(macOS)
/// The version this is, the offer of a new one, the update as it goes, and the restart once one
/// is in.
private struct UpdateSettings: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        let updater = store.updater
        ItemRow(title: "Wonnet \(updater.current)", detail: status) {
            switch updater.state {
            case .available: ActionButton(title: "Update", variant: .primary) { updater.install() }
            case .ready: ActionButton(title: "Restart", variant: .primary) { updater.relaunch() }
            case .failed: ActionButton(title: "Try again", symbol: .refreshCw) { updater.retry() }
            default: ActionButton(title: "Check for updates", symbol: .refreshCw, pending: updater.state == .checking) { updater.check(asked: true) }
            }
        }
    }

    private var status: String? {
        switch store.updater.state {
        case .idle, .checking: nil
        case .upToDate: "Up to date."
        case .available(let version): "Wonnet \(version) is available."
        case .downloading(let version, let fraction): "Downloading \(version)… \(Int(fraction * 100))%"
        case .installing(let version): "Installing \(version)…"
        case .ready(let version): "Wonnet \(version) is installed. Restart to use it."
        case .failed(let message): message
        }
    }
}
#endif
