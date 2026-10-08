import SwiftUI

/// Accounts, signatures, snippets, Split Inbox, blocked senders, notifications, appearance, the
/// undo delay and the server. What is synced is kept as the core's preferences.
struct SettingsView: View {
    @Environment(MailStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var adding = false

    var body: some View {
        @Bindable var store = store
        ScrollView {
            VStack(alignment: .leading, spacing: 30) {
                HStack {
                    Text("Settings").font(.ui(20, .semibold))
                    Spacer()
                    IconButton(symbol: .x, help: "Close", circled: false) { dismiss() }
                }
                SettingsSection(title: "Accounts") {
                    ForEach(store.accounts) { account in
                        HStack(spacing: 10) {
                            Circle().fill(Theme.accountColor(account.color).color).frame(width: 8, height: 8)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(account.address).font(.ui(14))
                                Text(status(account)).font(.ui(12)).foregroundStyle(Tokens.mutedMoreForeground.color)
                            }
                            Spacer()
                            ActionButton(title: "Remove", variant: .destructive) { store.removeAccount(account.id) }
                        }
                    }
                    if adding {
                        AddAccountForm().padding(.top, 8)
                    } else {
                        ActionButton(title: "Add account", symbol: .plus, variant: .outline) { adding = true }
                    }
                }
                if store.preferencesLoaded {
                    SettingsSection(title: "Signatures", detail: "Put under what you write from each account.") { SignatureSettings() }
                    SettingsSection(title: "Snippets", detail: "Type ; and a name while writing to put one in, or press ⌘;. {first_name} becomes the first recipient's first name.") {
                        SnippetSettings()
                    }
                    SettingsSection(title: "Split Inbox") { SplitSettings() }
                    SettingsSection(title: "Blocked senders", detail: "Their new mail goes to the trash.") { BlockedSettings() }
                    SettingsSection(title: "Images") { ImageSettings() }
                    SettingsSection(title: "Notifications") { NotificationSettings() }
                }
                SettingsSection(title: "Appearance") {
                    Picker("", selection: $store.appearance) {
                        ForEach(Appearance.allCases) { appearance in Text(appearance.name).tag(appearance) }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .fixedSize()
                    DarkMailSettings()
                }
                SettingsSection(title: "Undo send") {
                    Picker("", selection: $store.undoDelay) {
                        Text("Off").tag(0)
                        ForEach([5, 10, 20, 30], id: \.self) { seconds in Text("\(seconds) s").tag(seconds) }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .fixedSize()
                }
                SettingsSection(title: "Server") {
                    Text(store.server).font(.ui(13)).foregroundStyle(Tokens.mutedForeground.color).textSelection(.enabled)
                    Text(connectionText).font(.ui(12)).foregroundStyle(Tokens.mutedMoreForeground.color)
                }
                #if os(macOS)
                SettingsSection(title: "Updates", detail: "New versions are downloaded and installed on their own; a restart finishes them.") {
                    UpdateSettings()
                }
                #else
                SettingsSection(title: "Version") {
                    Text("Wonnet \(Platform.version)").font(.ui(13)).foregroundStyle(Tokens.mutedForeground.color)
                }
                #endif
                ActionButton(title: "Sign out", symbol: .logOut, variant: .destructive) {
                    store.signOut()
                    dismiss()
                }
            }
            .foregroundStyle(Tokens.foreground.color)
            .padding(28)
            .frame(maxWidth: 560, alignment: .leading)
        }
        .background(Tokens.background.color)
        .task { await store.loadPreferences() }
        #if os(macOS)
        .frame(width: 560, height: 620)
        #endif
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
/// The version this is, the update as it goes, and the restart once one is in.
private struct UpdateSettings: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        let updater = store.updater
        HStack(spacing: 12) {
            Text("Wonnet \(updater.current)").font(.ui(13)).foregroundStyle(Tokens.mutedForeground.color)
            Spacer()
            switch updater.state {
            case .ready: ActionButton(title: "Restart", variant: .primary) { updater.relaunch() }
            case .failed: ActionButton(title: "Try again", symbol: .refreshCw) { updater.retry() }
            default: ActionButton(title: "Check for updates", symbol: .refreshCw, pending: updater.state == .checking) { updater.check(asked: true) }
            }
        }
        if let status {
            Text(status).font(.ui(12)).foregroundStyle(Tokens.mutedMoreForeground.color).fixedSize(horizontal: false, vertical: true)
        }
    }

    private var status: String? {
        switch store.updater.state {
        case .idle, .checking: nil
        case .upToDate: "Up to date."
        case .downloading(let version, let fraction): "Downloading \(version)… \(Int(fraction * 100))%"
        case .installing(let version): "Installing \(version)…"
        case .ready(let version): "Wonnet \(version) is installed. Restart to use it."
        case .failed(let message): message
        }
    }
}
#endif

/// A part of the settings: a small heading, maybe a line about it, and its controls.
struct SettingsSection<Content: View>: View {
    let title: String
    var detail: String?
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text(title.uppercased()).font(.ui(11, .semibold)).foregroundStyle(Tokens.mutedMoreForeground.color)
                if let detail {
                    Text(detail).font(.ui(12)).foregroundStyle(Tokens.mutedMoreForeground.color).fixedSize(horizontal: false, vertical: true)
                }
            }
            content
        }
    }
}
