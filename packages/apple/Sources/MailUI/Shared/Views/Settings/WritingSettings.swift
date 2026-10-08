import SwiftUI

/// A signature for each account. While one is empty, the provider's is used, and shown greyed.
struct SignatureSettings: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        VStack(alignment: .leading, spacing: Space.l) {
            ForEach(store.accounts) { account in SignatureEditor(account: account) }
        }
    }
}

private struct SignatureEditor: View {
    @Environment(MailStore.self) private var store
    let account: AccountView
    @State private var text = ""
    @State private var loaded = false
    @State private var edited = false

    private var key: String { "signature:\(account.id)" }

    private var provided: String? {
        account.identities.first?.signature.flatMap { $0.isEmpty ? nil : $0 }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            InputField(label: account.address, text: Binding(get: { text }, set: { text = $0; edited = true }), placeholder: provided ?? "No signature", lines: 4)
            if provided != nil, text.isEmpty {
                Notice(text: "The signature kept by \(account.provider == "gmail" ? "Gmail" : "your provider") is used.")
            }
        }
        .onAppear {
            guard !loaded else { return }
            text = store.preferences[key]?.string ?? ""
            loaded = true
        }
        .task(id: text) {
            try? await Task.sleep(for: .milliseconds(800))
            guard !Task.isCancelled else { return }
            keep()
        }
        .onDisappear(perform: keep)
    }

    private func keep() {
        guard loaded, edited else { return }
        edited = false
        store.setPreference(key, text.isEmpty ? nil : .string(text))
    }
}

/// The snippets: add, edit, remove.
struct SnippetSettings: View {
    @Environment(MailStore.self) private var store
    /// The snippet being written, new or not.
    @State private var editing: Snippet?

    var body: some View {
        let snippets = Snippet.all(in: store.preferences)
        VStack(alignment: .leading, spacing: Space.s + 2) {
            ForEach(snippets) { snippet in
                if editing?.id == snippet.id {
                    SnippetForm(snippet: snippet) { editing = nil }
                } else {
                    ItemRow(title: snippet.name, detail: snippet.text.replacingOccurrences(of: "\n", with: " ")) {
                        IconButton(symbol: .pencil, help: "Edit") { editing = snippet }
                        IconButton(symbol: .trash, help: "Remove") { store.setPreference(snippet.key, nil) }
                    }
                }
            }
            if let editing, !snippets.contains(where: { $0.id == editing.id }) {
                SnippetForm(snippet: editing) { self.editing = nil }
            } else if editing == nil {
                ActionButton(title: "Add snippet", symbol: .plus, variant: .outline) {
                    editing = Snippet(id: UUID().uuidString.lowercased(), name: "", text: "")
                }
            }
        }
    }
}

private struct SnippetForm: View {
    @Environment(MailStore.self) private var store
    @State var snippet: Snippet
    let done: () -> Void

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: Space.m) {
                InputField(label: "Name", text: $snippet.name, placeholder: "thanks", autofocus: snippet.name.isEmpty)
                InputField(label: "Text", text: $snippet.text, placeholder: "Thanks {first_name}!", lines: 4)
                FormButtons(saveDisabled: snippet.name.trimmingCharacters(in: .whitespaces).isEmpty, onSave: save, onCancel: done)
            }
        }
    }

    private func save() {
        snippet.name = snippet.name.trimmingCharacters(in: .whitespaces)
        store.setPreference(snippet.key, snippet.value)
        done()
    }
}
