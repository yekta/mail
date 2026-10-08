import SwiftUI

/// The first screen: add a mail account, which also signs in.
struct OnboardingView: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        ScrollView {
            VStack(spacing: Space.xxl) {
                VStack(spacing: Space.s + 2) {
                    Image(.send, size: 34)
                        .foregroundStyle(Tokens.primaryForeground.color)
                        .frame(width: 72 * Platform.scale, height: 72 * Platform.scale)
                        .background(Circle().fill(Tokens.primary.color))
                    Text("Mail").textStyle(.display)
                    Text("All your email, calm and fast.").textStyle(.body, color: Tokens.mutedMoreForeground.color)
                }
                .padding(.top, 48)
                AddAccountForm()
            }
            .frame(maxWidth: 360)
            .padding(Space.xl)
            .frame(maxWidth: .infinity)
        }
        .background(Tokens.background.color)
    }
}

/// Google, or a JMAP server with a password; the server this app talks to under Advanced.
struct AddAccountForm: View {
    @Environment(MailStore.self) private var store
    @State private var url = ""
    @State private var email = ""
    @State private var password = ""
    @State private var server = ""
    @State private var adding = false
    @State private var error: String?

    var body: some View {
        VStack(spacing: Space.l + 2) {
            ActionButton(title: "Continue with Google", symbol: .mail, variant: .primary, wide: true, action: store.signInWithGoogle)
            HStack(spacing: Space.s + 2) {
                Rule()
                Text("or a JMAP server").textStyle(.caption).fixedSize()
                Rule()
            }
            VStack(spacing: Space.m) {
                InputField(label: "Server", text: $url, placeholder: "https://api.fastmail.com")
                InputField(label: "Email", text: $email, placeholder: "you@example.com")
                InputField(label: "Password", text: $password, placeholder: "App password", secure: true)
            }
            if let error {
                Notice(text: error, tone: .error)
            }
            ActionButton(title: "Add account", variant: .outline, pending: adding, wide: true, action: addJmap)
                .disabled(url.isEmpty || email.isEmpty || password.isEmpty)
            Disclosure(title: "Advanced") {
                VStack(spacing: Space.s + 2) {
                    InputField(label: "Mail server for this app", text: $server, placeholder: "https://mail.example.com")
                    ActionButton(title: "Use this server", variant: .ghost, action: saveServer)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .onAppear { server = store.server }
    }

    private func addJmap() {
        adding = true
        error = nil
        Task {
            do {
                try await store.addJmap(url: url, username: email, password: password)
                password = ""
            } catch {
                self.error = error.localizedDescription
            }
            adding = false
        }
    }

    private func saveServer() {
        Task {
            do {
                try await store.setServer(server)
                error = nil
            } catch {
                self.error = error.localizedDescription
            }
        }
    }
}
