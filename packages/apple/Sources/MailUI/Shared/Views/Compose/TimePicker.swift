import SwiftUI

/// When something should happen: the usual times, or what the user writes ("tomorrow 9am",
/// "in 2 hours", "fri"), read by the core.
struct TimePicker: View {
    @Environment(MailStore.self) private var store
    let title: String
    let pick: (TimeChoice) -> Void
    let cancel: () -> Void
    @State private var text = ""
    @State private var choices: [TimeChoice] = []
    @State private var highlighted = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title).font(.ui(14, .semibold)).foregroundStyle(Tokens.foreground.color)
            InputField(label: "When", text: $text, placeholder: "Tomorrow 9am, in 2 hours, Friday…", autofocus: true)
                .onSubmit(choose)
                .onKeyPress(.downArrow) { move(1) }
                .onKeyPress(.upArrow) { move(-1) }
            VStack(spacing: 2) {
                ForEach(Array(choices.enumerated()), id: \.element.id) { index, choice in
                    ChoiceRow(title: choice.name, detail: choice.label, highlighted: index == highlighted) { pick(choice) }
                }
                if choices.isEmpty, !text.isEmpty {
                    Text("That isn't a time I can read.").font(.ui(13)).foregroundStyle(Tokens.mutedMoreForeground.color).padding(.vertical, 8)
                }
            }
            HStack {
                Spacer()
                ActionButton(title: "Cancel", variant: .ghost, action: cancel).keyboardShortcut(.cancelAction)
            }
        }
        .padding(20)
        .frame(minWidth: 340)
        .background(Tokens.popover.color)
        .task(id: text) {
            if !text.isEmpty { try? await Task.sleep(for: .milliseconds(120)) }
            guard !Task.isCancelled else { return }
            choices = await store.parseTime(text)
            highlighted = 0
        }
    }

    private func move(_ step: Int) -> KeyPress.Result {
        guard !choices.isEmpty else { return .ignored }
        highlighted = min(max(highlighted + step, 0), choices.count - 1)
        return .handled
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
    let cancel: () -> Void
    @State private var query = ""
    @State private var highlighted = 0

    private var matches: [Snippet] {
        guard !query.isEmpty else { return snippets }
        return snippets.filter { $0.name.localizedCaseInsensitiveContains(query) || $0.text.localizedCaseInsensitiveContains(query) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Snippets").font(.ui(14, .semibold)).foregroundStyle(Tokens.foreground.color)
            if snippets.isEmpty {
                Text("Add snippets in Settings, then type ; and a name to put one in.")
                    .font(.ui(13)).foregroundStyle(Tokens.mutedMoreForeground.color)
            } else {
                InputField(label: "Find", text: $query, placeholder: "Name", autofocus: true)
                    .onSubmit {
                        guard matches.indices.contains(highlighted) else { return }
                        pick(matches[highlighted])
                    }
                    .onKeyPress(.downArrow) { move(1) }
                    .onKeyPress(.upArrow) { move(-1) }
                    .onChange(of: query) { highlighted = 0 }
                ScrollView {
                    VStack(spacing: 2) {
                        ForEach(Array(matches.enumerated()), id: \.element.id) { index, snippet in
                            ChoiceRow(title: snippet.name, detail: snippet.text.replacingOccurrences(of: "\n", with: " "), highlighted: index == highlighted) {
                                pick(snippet)
                            }
                        }
                    }
                }
                .frame(maxHeight: 280)
            }
            HStack {
                Spacer()
                ActionButton(title: "Cancel", variant: .ghost, action: cancel).keyboardShortcut(.cancelAction)
            }
        }
        .padding(20)
        .frame(minWidth: 380)
        .background(Tokens.popover.color)
    }

    private func move(_ step: Int) -> KeyPress.Result {
        guard !matches.isEmpty else { return .ignored }
        highlighted = min(max(highlighted + step, 0), matches.count - 1)
        return .handled
    }
}
