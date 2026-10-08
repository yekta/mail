import SwiftUI

/// A line across the window that stays until what it says is done, with the one thing to do
/// about it at its end.
struct Banner: View {
    let symbol: Symbol
    let text: String
    let action: String
    let perform: () -> Void

    var body: some View {
        HStack(spacing: Space.s + 2) {
            Image(symbol, size: 14).foregroundStyle(Tokens.mutedForeground.color)
            Text(text).textStyle(.label, color: Tokens.accentForeground.color).lineLimit(1)
            Spacer()
            ActionButton(title: action, variant: .primary, action: perform)
        }
        .padding(.horizontal, Space.xl)
        .frame(height: 44 * Platform.scale)
        .background(Tokens.accent.color)
        .rule(.bottom)
    }
}
