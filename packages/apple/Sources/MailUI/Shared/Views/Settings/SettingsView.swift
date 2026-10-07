import SwiftUI

/// Accounts, appearance, the undo delay and the server.
struct SettingsView: View {
    @Environment(MailStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var adding = false

    var body: some View {
        @Bindable var store = store
        ScrollView {
            VStack(alignment: .leading, spacing: 26) {
                HStack {
                    Text("Settings").font(.ui(20, .semibold))
                    Spacer()
                    IconButton(symbol: .x, help: "Close", circled: false) { dismiss() }
                }
                section("Accounts") {
                    ForEach(store.accounts) { account in
                        HStack(spacing: 10) {
                            Circle().fill(Theme.accountColor(account.color).color).frame(width: 8, height: 8)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(account.address).font(.ui(14))
                                Text(status(account)).font(.ui(12)).foregroundStyle(Tokens.mutedForeground.color)
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
                section("Appearance") {
                    Picker("", selection: $store.appearance) {
                        ForEach(Appearance.allCases) { appearance in Text(appearance.name).tag(appearance) }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .fixedSize()
                }
                section("Undo send") {
                    Picker("", selection: $store.undoDelay) {
                        Text("Off").tag(0)
                        ForEach([5, 10, 20, 30], id: \.self) { seconds in Text("\(seconds) s").tag(seconds) }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .fixedSize()
                }
                section("Server") {
                    Text(store.server).font(.ui(13)).foregroundStyle(Tokens.secondaryForeground.color).textSelection(.enabled)
                    Text(connectionText).font(.ui(12)).foregroundStyle(Tokens.mutedForeground.color)
                }
                ActionButton(title: "Sign out", symbol: .logOut, variant: .destructive) {
                    store.signOut()
                    dismiss()
                }
            }
            .foregroundStyle(Tokens.foreground.color)
            .padding(28)
            .frame(maxWidth: 520, alignment: .leading)
        }
        .background(Tokens.background.color)
        #if os(macOS)
        .frame(minWidth: 520, minHeight: 560)
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

    private func section(_ title: String, @ViewBuilder content: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title.uppercased()).font(.ui(11, .semibold)).foregroundStyle(Tokens.mutedForeground.color)
            content()
        }
    }
}
