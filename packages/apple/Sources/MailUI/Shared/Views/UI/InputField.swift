import SwiftUI

/// A text field on a hairline, with its label above. With `lines` above one it is a text box
/// that many lines tall, where Return starts a new line.
struct InputField: View {
    let label: String
    @Binding var text: String
    var placeholder = ""
    var secure = false
    var lines = 1
    var autofocus = false
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label).font(.ui(11, .medium)).foregroundStyle(Tokens.mutedForeground.color)
            Group {
                if secure {
                    SecureField(placeholder, text: $text)
                } else if lines > 1 {
                    TextEditor(text: $text)
                        .scrollContentBackground(.hidden)
                        .overlay(alignment: .topLeading) {
                            if text.isEmpty {
                                Text(placeholder).foregroundStyle(Tokens.mutedForeground.color).padding(.leading, 5).allowsHitTesting(false)
                            }
                        }
                        .padding(.vertical, 8)
                } else {
                    TextField(placeholder, text: $text)
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        #endif
                }
            }
            .textFieldStyle(.plain)
            .focused($focused)
            .font(.ui(14))
            .padding(.horizontal, 10)
            .frame(height: (lines > 1 ? CGFloat(lines) * 18 + 16 : 34) * Platform.scale)
            .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Tokens.card.color))
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Tokens.input.color, lineWidth: 1))
        }
        .onAppear {
            guard autofocus else { return }
            DispatchQueue.main.async { focused = true }
        }
    }
}
