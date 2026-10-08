import SwiftUI

/// When something should happen: the usual times, or what the user writes ("tomorrow 9am",
/// "in 2 hours", "fri"), read by the core.
struct TimePicker: View {
    @Environment(MailStore.self) private var store
    let title: String
    let pick: (TimeChoice) -> Void
    @State private var text = ""
    @State private var choices: [TimeChoice] = []
    @State private var highlighted = 0

    var body: some View {
        Sheet(title: title) {
            VStack(spacing: 0) {
                SearchField(
                    placeholder: "Tomorrow 9am, in 2 hours, Friday…", text: $text, symbol: .clock,
                    submit: choose, move: { highlighted = highlighted.moved(by: $0, in: choices.count) }
                )
                ChoiceList(items: choices, highlighted: highlighted, empty: text.isEmpty ? nil : "That isn't a time I can read.") { index, choice in
                    ChoiceRow(title: choice.name, detail: choice.label, highlighted: index == highlighted) { pick(choice) }
                }
            }
        }
        .task(id: text) {
            if !text.isEmpty { try? await Task.sleep(for: .milliseconds(120)) }
            guard !Task.isCancelled else { return }
            choices = await store.parseTime(text)
            highlighted = 0
        }
    }

    private func choose() {
        guard choices.indices.contains(highlighted) else { return }
        pick(choices[highlighted])
    }
}

/// The snippets to put in, narrowed by name as the user types.
struct SnippetPicker: View {
    let snippets: [Snippet]
    let pick: (Snippet) -> Void
    @State private var query = ""
    @State private var highlighted = 0

    private var matches: [Snippet] {
        guard !query.isEmpty else { return snippets }
        return snippets.filter { $0.name.localizedCaseInsensitiveContains(query) || $0.text.localizedCaseInsensitiveContains(query) }
    }

    var body: some View {
        let matches = matches
        Sheet(title: "Snippets") {
            VStack(spacing: 0) {
                SearchField(
                    placeholder: "Find a snippet", text: $query, symbol: .zap,
                    submit: { if matches.indices.contains(highlighted) { pick(matches[highlighted]) } },
                    move: { highlighted = highlighted.moved(by: $0, in: matches.count) }
                )
                ChoiceList(items: matches, highlighted: highlighted, empty: empty) { index, snippet in
                    ChoiceRow(title: snippet.name, detail: snippet.text.replacingOccurrences(of: "\n", with: " "), highlighted: index == highlighted) {
                        pick(snippet)
                    }
                }
            }
        }
        .onChange(of: query) { highlighted = 0 }
    }

    private var empty: String {
        snippets.isEmpty ? "Add snippets in Settings, then type ; and a name to put one in." : "No snippet by that name."
    }
}
