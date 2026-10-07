import SwiftUI

/// The labels of the threads' account, to add or take off (l) or to move the threads to (v);
/// what is typed narrows them, or names a new one.
struct LabelPicker: View {
    @Environment(MailStore.self) private var store
    let labeling: Labeling
    @State private var query = ""
    @State private var highlighted = 0

    private enum Choice: Identifiable {
        case label(Mailbox)
        case create(String)

        var id: String {
            switch self {
            case .label(let mailbox): mailbox.id
            case .create(let name): "create/\(name)"
            }
        }
    }

    private var account: String? { labeling.threads.first.flatMap(store.account(of:)) }

    /// The labels the open thread has, to take off.
    private var applied: Set<String> {
        guard !labeling.move, let conversation = store.conversation, labeling.threads == [conversation.id] else { return [] }
        return Set(conversation.labels.map(\.id))
    }

    private var choices: [Choice] {
        let labels = account.map(store.labels(of:)) ?? []
        let typed = query.trimmingCharacters(in: .whitespaces)
        let found = typed.isEmpty ? labels : labels.filter { $0.name.localizedCaseInsensitiveContains(typed) }
        var choices = found.map(Choice.label)
        if !typed.isEmpty, !labels.contains(where: { $0.name.caseInsensitiveCompare(typed) == .orderedSame }) {
            choices.append(.create(typed))
        }
        return choices
    }

    var body: some View {
        let choices = choices
        VStack(alignment: .leading, spacing: 0) {
            Text(labeling.move ? "Move to" : "Label")
                .font(.ui(13, .semibold))
                .foregroundStyle(Tokens.foreground.color)
                .padding(.horizontal, 16)
                .padding(.top, 16)
                .padding(.bottom, 4)
            SearchField(
                placeholder: "Find or create a label", text: $query, symbol: .tag,
                submit: { pick(choices, highlighted) }, move: { highlighted = max(0, min(highlighted + $0, choices.count - 1)) },
                escape: { store.labeling = nil }
            )
            ScrollViewReader { scroller in
                ScrollView {
                    VStack(spacing: 2) {
                        ForEach(Array(choices.enumerated()), id: \.element.id) { index, choice in
                            row(choice, highlighted: index == highlighted) { pick(choices, index) }.id(choice.id)
                        }
                        if choices.isEmpty {
                            Text("No labels yet. Type a name to make one.")
                                .font(.ui(13))
                                .foregroundStyle(Tokens.mutedForeground.color)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(10)
                        }
                    }
                    .padding(8)
                }
                .onChange(of: highlighted) { _, index in
                    guard choices.indices.contains(index) else { return }
                    scroller.scrollTo(choices[index].id)
                }
            }
        }
        #if os(macOS)
        .frame(width: 400, height: 380, alignment: .top)
        #endif
        .background(Tokens.popover.color)
        .onChange(of: query) { highlighted = 0 }
    }

    @ViewBuilder
    private func row(_ choice: Choice, highlighted: Bool, action: @escaping () -> Void) -> some View {
        switch choice {
        case .label(let mailbox):
            let on = MailStore.labelID(mailbox.id).map(applied.contains) ?? false
            ChoiceRow(title: mailbox.name, detail: on ? "Remove" : nil, symbol: on ? .check : .tag, highlighted: highlighted, action: action)
        case .create(let name):
            ChoiceRow(title: "Create label “\(name)”", symbol: .plus, highlighted: highlighted, action: action)
        }
    }

    private func pick(_ choices: [Choice], _ index: Int) {
        guard choices.indices.contains(index) else { return }
        switch choices[index] {
        case .label(let mailbox):
            guard let label = MailStore.labelID(mailbox.id), let account else { return }
            store.apply(label: label, account: account, to: labeling, remove: applied.contains(label))
        case .create(let name):
            guard let account else { return }
            store.createLabel(name, account: account, for: labeling)
        }
    }
}
