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
        VStack(alignment: .leading, spacing: 0) {
            Text(store.remindsInsteadOfSnoozing ? "Remind me if nobody answers" : "Snooze until")
                .font(.ui(13, .semibold))
                .foregroundStyle(Tokens.foreground.color)
                .padding(.horizontal, 16)
                .padding(.top, 16)
                .padding(.bottom, 4)
            SearchField(
                placeholder: "Tomorrow 9am, in 2 hours, fri…", text: $text, symbol: .clock,
                submit: { pick(highlighted) }, move: { highlighted = max(0, min(highlighted + $0, choices.count - 1)) },
                escape: { store.snoozing = nil }
            )
            VStack(spacing: 2) {
                ForEach(Array(choices.enumerated()), id: \.element.id) { index, choice in
                    ChoiceRow(title: choice.name, detail: choice.label, highlighted: index == highlighted) { pick(index) }
                }
                if choices.isEmpty, !text.isEmpty {
                    Text("That isn't a time I know.")
                        .font(.ui(13))
                        .foregroundStyle(Tokens.mutedMoreForeground.color)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(10)
                }
            }
            .padding(8)
            Spacer(minLength: 0)
        }
        .frame(minWidth: 360, minHeight: 300, alignment: .top)
        .background(Tokens.popover.color)
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
