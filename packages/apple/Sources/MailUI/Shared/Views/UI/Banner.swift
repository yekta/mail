import SwiftUI

/// A line across the window that stays until what it says is done, with the one thing to do
/// about it at its end.
struct Banner: View {
    let symbol: Symbol
    let text: String
    let action: String
    let perform: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Image(symbol, size: 14).foregroundStyle(Tokens.secondaryForeground.color)
            Text(text).font(.ui(13)).foregroundStyle(Tokens.accentForeground.color).lineLimit(1)
            Spacer()
            ActionButton(title: action, variant: .primary, action: perform)
        }
        .padding(.horizontal, 20)
        .frame(height: 44 * Platform.scale)
        .background(Tokens.accent.color)
        .overlay(alignment: .bottom) { Rectangle().fill(Tokens.border.color).frame(height: 1) }
    }
}
