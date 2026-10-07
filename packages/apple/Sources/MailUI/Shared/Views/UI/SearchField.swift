import SwiftUI

/// A field that narrows a list under it: the search icon, the words, and the keys that move
/// through the list (↑, ↓, Enter to pick, Esc to leave).
struct SearchField: View {
    let placeholder: String
    @Binding var text: String
    var symbol: Symbol = .search
    var submit: () -> Void = {}
    var move: (Int) -> Void = { _ in }
    var escape: () -> Void = {}
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(symbol, size: 14).foregroundStyle(Tokens.mutedForeground.color)
            TextField(placeholder, text: $text)
                .textFieldStyle(.plain)
                .font(.ui(15))
                .focused($focused)
                .onSubmit(submit)
                .onListKeys(move: move, escape: escape)
                #if os(iOS)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                #endif
        }
        .padding(.horizontal, 12)
        .frame(height: 40 * Platform.scale)
        .overlay(alignment: .bottom) { Rectangle().fill(Tokens.border.color).frame(height: 1) }
        .onAppear { focused = true }
    }
}
