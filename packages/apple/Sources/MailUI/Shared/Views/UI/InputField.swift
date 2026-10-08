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
        VStack(alignment: .leading, spacing: Space.xs) {
            Text(label).textStyle(.footnote).fontWeight(.medium)
            Group {
                if secure {
                    SecureField(placeholder, text: $text)
                } else if lines > 1 {
                    TextEditor(text: $text)
                        .scrollContentBackground(.hidden)
                        .overlay(alignment: .topLeading) {
                            if text.isEmpty {
                                Text(placeholder).foregroundStyle(Tokens.mutedMoreForeground.color).padding(.leading, 5).allowsHitTesting(false)
                            }
                        }
                        .padding(.vertical, Space.s)
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
            .font(.ui(.body))
            .foregroundStyle(Tokens.foreground.color)
            .padding(.horizontal, Space.s + 2)
            .frame(height: (lines > 1 ? CGFloat(lines) * 18 + 16 : Theme.fieldHeight) * Platform.scale)
            .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Tokens.card.color))
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Tokens.input.color, lineWidth: Theme.hairline))
        }
        .onAppear {
            guard autofocus else { return }
            DispatchQueue.main.async { focused = true }
        }
    }
}
