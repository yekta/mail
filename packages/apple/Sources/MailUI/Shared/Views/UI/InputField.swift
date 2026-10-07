import SwiftUI

/// A text field on a hairline, with its label above.
struct InputField: View {
    let label: String
    @Binding var text: String
    var placeholder = ""
    var secure = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label).font(.ui(11, .medium)).foregroundStyle(Tokens.mutedForeground.color)
            Group {
                if secure {
                    SecureField(placeholder, text: $text)
                } else {
                    TextField(placeholder, text: $text)
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        #endif
                }
            }
            .textFieldStyle(.plain)
            .font(.ui(14))
            .padding(.horizontal, 10)
            .frame(height: 34 * Platform.scale)
            .background(RoundedRectangle(cornerRadius: Theme.radius).fill(Tokens.card.color))
            .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Tokens.input.color, lineWidth: 1))
        }
    }
}
