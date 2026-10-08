import SwiftUI

/// Someone the thread is with: who they are, a way to write to them, and the latest threads with
/// them. Beside the thread on a wide Mac window; a sheet on iOS. `openThread` makes the threads
/// open on a click. In a sheet, `closeSheet` closes it before compose opens.
struct PersonView: View {
    @Environment(MailStore.self) private var store
    let email: String
    var closeSheet: (() -> Void)?
    var openThread: ((String) -> Void)?
    @State private var person: Person?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Avatar(initials: person?.initials ?? String(email.prefix(1)).uppercased(), email: email, size: 52)
                VStack(alignment: .leading, spacing: 3) {
                    Text(name).font(.ui(17, .semibold)).foregroundStyle(Tokens.foreground.color)
                    Text(email).font(.ui(13)).foregroundStyle(Tokens.mutedMoreForeground.color).textSelection(.enabled)
                }
                ActionButton(title: "New message", symbol: .squarePen, variant: .outline, action: write)
                if let threads = person?.threads, !threads.isEmpty {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("RECENT").font(.ui(11, .semibold)).foregroundStyle(Tokens.mutedMoreForeground.color).padding(.bottom, 6)
                        ForEach(threads) { row in
                            ChoiceRow(title: row.subject.isEmpty ? "(no subject)" : row.subject, detail: row.date) { openThread?(row.id) }
                                .disabled(openThread == nil)
                        }
                    }
                    .padding(.top, 8)
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Tokens.card.color)
        .task(id: email) { person = await store.person(email) }
    }

    private func write() {
        let address = Address(name: person?.name.isEmpty == false ? person?.name : nil, email: email)
        guard let closeSheet else {
            store.write(to: address)
            return
        }
        closeSheet()
        // A sheet can't show while another is still going away.
        Task {
            try? await Task.sleep(for: .milliseconds(350))
            store.write(to: address)
        }
    }

    private var name: String {
        guard let person, !person.name.isEmpty else { return email }
        return person.name
    }
}
