import SwiftUI

/// To, Cc or Bcc: the addresses as chips, a field to type more, and below it the people it could
/// be. ↑ and ↓ move between them; Tab or Return picks one. A comma, a semicolon or a paste of
/// many addresses makes chips of them; what can't be sent to is shown in red.
struct AddressField<Accessory: View>: View {
    @Environment(MailStore.self) private var store
    let label: String
    @Binding var addresses: [Address]
    @Binding var text: String
    var autofocus = false
    @ViewBuilder var accessory: Accessory
    @FocusState private var focused: Bool
    @State private var suggestions: [Address] = []
    @State private var highlighted = 0

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .top, spacing: 10) {
                Text(label).font(.ui(13)).foregroundStyle(Tokens.mutedForeground.color)
                    .frame(width: 56, height: 24 * Platform.scale, alignment: .leading)
                FlowLayout(spacing: 6, lineSpacing: 6) {
                    ForEach(Array(addresses.enumerated()), id: \.offset) { index, address in
                        Chip(
                            title: address.name ?? address.email, variant: address.isValid ? .plain : .invalid,
                            help: address.isValid ? address.text : "“\(address.email)” isn't an address",
                            remove: { addresses.remove(at: index) }
                        )
                    }
                    TextField("", text: $text)
                        .textFieldStyle(.plain)
                        .font(.ui(14))
                        .focused($focused)
                        .frame(minWidth: 120)
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.emailAddress)
                        #endif
                        .onKeyPress(.downArrow) { moveHighlight(1) }
                        .onKeyPress(.upArrow) { moveHighlight(-1) }
                        .onKeyPress(.tab) { pick() ? .handled : .ignored }
                        .onKeyPress(.return) { pick() ? .handled : .ignored }
                        .onKeyPress(.escape) {
                            guard !suggestions.isEmpty else { return .ignored }
                            suggestions = []
                            return .handled
                        }
                        .onKeyPress(.delete) {
                            guard text.isEmpty, !addresses.isEmpty else { return .ignored }
                            addresses.removeLast()
                            return .handled
                        }
                        .onSubmit { _ = pick() }
                }
                accessory
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            .frame(minHeight: 40 * Platform.scale)
            Rectangle().fill(Tokens.border.color).frame(height: 1).padding(.leading, 16)
        }
        // Hangs below the field, over what follows it.
        .overlay(alignment: .bottomLeading) {
            Color.clear.frame(height: 0).overlay(alignment: .topLeading) {
                if focused, !suggestions.isEmpty {
                    PopupCard {
                        ForEach(Array(suggestions.enumerated()), id: \.offset) { index, address in
                            ChoiceRow(title: address.name ?? address.email, detail: address.name == nil ? nil : address.email, highlighted: index == highlighted) {
                                add(address)
                            }
                        }
                    }
                    .frame(maxWidth: 380)
                    .padding(.leading, 82)
                    .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .zIndex(focused ? 1 : 0)
        .onChange(of: text) { _, typed in split(typed) }
        .task(id: text) { await suggest() }
        .onAppear {
            guard autofocus else { return }
            DispatchQueue.main.async { focused = true }
        }
    }

    /// Makes chips of what is before a comma, a semicolon or a line break.
    private func split(_ typed: String) {
        guard typed.contains(where: { ",;\n".contains($0) }) else { return }
        var parts = typed.components(separatedBy: CharacterSet(charactersIn: ",;\n"))
        let rest = parts.removeLast()
        parts.forEach { add(text: $0) }
        text = rest.trimmingCharacters(in: .whitespaces)
    }

    private func suggest() async {
        let query = text.trimmingCharacters(in: .whitespaces)
        guard !query.isEmpty else {
            suggestions = []
            return
        }
        try? await Task.sleep(for: .milliseconds(60))
        guard !Task.isCancelled else { return }
        let taken = Set(addresses.map { $0.email.lowercased() })
        suggestions = await store.contacts(query).filter { !taken.contains($0.email.lowercased()) }
        highlighted = 0
    }

    private func moveHighlight(_ step: Int) -> KeyPress.Result {
        guard !suggestions.isEmpty else { return .ignored }
        highlighted = min(max(highlighted + step, 0), suggestions.count - 1)
        return .handled
    }

    /// Takes the highlighted suggestion, or what was typed. False when there was nothing.
    private func pick() -> Bool {
        if suggestions.indices.contains(highlighted) {
            add(suggestions[highlighted])
            return true
        }
        guard !text.trimmingCharacters(in: .whitespaces).isEmpty else { return false }
        add(text: text)
        text = ""
        return true
    }

    private func add(_ address: Address) {
        if !addresses.contains(where: { $0.email.caseInsensitiveCompare(address.email) == .orderedSame }) {
            addresses.append(address)
        }
        text = ""
        suggestions = []
    }

    private func add(text piece: String) {
        Address.typed(piece).forEach { address in
            guard !addresses.contains(where: { $0.email.caseInsensitiveCompare(address.email) == .orderedSame }) else { return }
            addresses.append(address)
        }
    }
}

extension AddressField where Accessory == EmptyView {
    init(label: String, addresses: Binding<[Address]>, text: Binding<String>, autofocus: Bool = false) {
        self.init(label: label, addresses: addresses, text: text, autofocus: autofocus, accessory: { EmptyView() })
    }
}

extension Address {
    /// Reads typed or pasted text as addresses. What can't be read is kept as written, to be
    /// shown as wrong.
    static func typed(_ text: String) -> [Address] {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return [] }
        if let address = Address.parse(text) { return [address] }
        let words = text.split(whereSeparator: \.isWhitespace).map(String.init)
        if words.count > 1, words.allSatisfy({ $0.contains("@") }) {
            return words.map { Address.parse($0) ?? Address(name: nil, email: $0) }
        }
        return [Address(name: nil, email: text)]
    }
}
