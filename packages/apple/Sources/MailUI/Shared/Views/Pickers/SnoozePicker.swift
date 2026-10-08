import SwiftUI

/// When a snooze ends, or when a reminder comes: what the field says ("tomorrow 9am", "in 2
/// hours"), read by the core as it is typed, or the usual times.
struct SnoozePicker: View {
    @Environment(MailStore.self) private var store
    let threads: [String]
    @State private var text = ""
    @State private var choices: [TimeChoice] = []
    @State private var highlighted = 0

    var body: some View {
        Sheet(title: store.remindsInsteadOfSnoozing ? "Remind me if nobody answers" : "Snooze until") {
            VStack(spacing: 0) {
                SearchField(
                    placeholder: "Tomorrow 9am, in 2 hours, fri…", text: $text, symbol: .clock,
                    submit: { pick(highlighted) }, move: { highlighted = highlighted.moved(by: $0, in: choices.count) },
                    escape: { store.snoozing = nil }
                )
                ChoiceList(items: choices, highlighted: highlighted, empty: text.isEmpty ? nil : "That isn't a time I know.") { index, choice in
                    ChoiceRow(title: choice.name, detail: choice.label, highlighted: index == highlighted) { pick(index) }
                }
            }
        }
        .task(id: text) {
            if !text.isEmpty {
                try? await Task.sleep(for: .milliseconds(120))
                guard !Task.isCancelled else { return }
            }
            let found = await store.parseTime(text)
            guard !Task.isCancelled else { return }
            choices = found
            highlighted = 0
        }
    }

    private func pick(_ index: Int) {
        guard choices.indices.contains(index) else { return }
        store.snoozing = nil
        store.act(.snooze, on: threads, until: choices[index].date)
    }
}
