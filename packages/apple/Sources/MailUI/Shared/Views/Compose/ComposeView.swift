import SwiftUI

/// Writing a message: who it goes to, its subject and its text. Sending waits for the undo delay.
struct ComposeView: View {
    @Environment(MailStore.self) private var store
    @State var compose: Compose
    @State private var to = ""
    @State private var cc = ""
    @State private var bcc = ""
    @FocusState private var focus: Field?

    private enum Field {
        case to, subject, text
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                ActionButton(title: "Cancel", variant: .ghost) { store.compose = nil }
                    .keyboardShortcut(.cancelAction)
                Spacer()
                Text(title).font(.ui(14, .semibold)).foregroundStyle(Tokens.foreground.color)
                Spacer()
                ActionButton(title: "Send", symbol: .send, variant: .primary, action: send)
                    .keyboardShortcut(.return, modifiers: .command)
                    .disabled(Address.list(to + "," + cc + "," + bcc).isEmpty)
            }
            .padding(.horizontal, 14)
            .frame(height: 52)
            rule
            if store.accounts.count > 1 {
                row("From") {
                    Picker("", selection: $compose.draft.accountId) {
                        ForEach(store.accounts) { account in Text(account.address).tag(account.id) }
                    }
                    .labelsHidden()
                    .fixedSize()
                    Spacer()
                }
            }
            row("To") {
                TextField("", text: $to).focused($focus, equals: .to)
                if !compose.showCc {
                    Button("Cc Bcc") { compose.showCc = true }
                        .buttonStyle(.plain)
                        .font(.ui(12))
                        .foregroundStyle(Tokens.mutedForeground.color)
                }
            }
            if compose.showCc {
                row("Cc") { TextField("", text: $cc) }
                row("Bcc") { TextField("", text: $bcc) }
            }
            row("Subject") { TextField("", text: $compose.draft.subject).focused($focus, equals: .subject) }
            TextEditor(text: $compose.draft.text)
                .font(.ui(14))
                .scrollContentBackground(.hidden)
                .focused($focus, equals: .text)
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
        }
        .textFieldStyle(.plain)
        .background(Tokens.card.color)
        .onAppear {
            to = compose.draft.to.map(\.text).joined(separator: ", ")
            cc = compose.draft.cc.map(\.text).joined(separator: ", ")
            bcc = compose.draft.bcc.map(\.text).joined(separator: ", ")
            if compose.draft.accountId.isEmpty { compose.draft.accountId = store.accounts.first?.id ?? "" }
            focus = to.isEmpty ? .to : .text
        }
        #if os(macOS)
        .frame(minWidth: 620, minHeight: 480)
        #endif
    }

    private var title: String {
        compose.draft.inReplyTo == nil ? (compose.draft.subject.hasPrefix("Fwd:") ? "Forward" : "New message") : "Reply"
    }

    private var rule: some View {
        Rectangle().fill(Tokens.border.color).frame(height: 1)
    }

    private func row(_ label: String, @ViewBuilder content: () -> some View) -> some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Text(label).font(.ui(13)).foregroundStyle(Tokens.mutedForeground.color).frame(width: 56, alignment: .leading)
                content()
            }
            .font(.ui(14))
            .padding(.horizontal, 16)
            .frame(height: 40 * Platform.scale)
            rule.padding(.leading, 16)
        }
    }

    private func send() {
        var draft = compose.draft
        draft.to = Address.list(to)
        draft.cc = Address.list(cc)
        draft.bcc = Address.list(bcc)
        store.send(draft)
    }
}
