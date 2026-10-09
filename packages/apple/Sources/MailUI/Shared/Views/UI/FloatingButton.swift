import SwiftUI

/// An icon on a circle over a page's corner: Compose over the list on iOS. The page's colour
/// with its border and a little shadow, so it stands off the list scrolling under it.
struct FloatingButton: View {
    static let side: CGFloat = 56

    let symbol: Symbol
    let help: String
    let action: () -> Void

    var body: some View {
        Button(action: action) { Face(symbol: symbol) }
            .buttonStyle(.press)
            .accessibilityLabel(help)
    }

    private struct Face: View {
        let symbol: Symbol
        @Environment(\.hovered) private var hovered

        var body: some View {
            Image(symbol, size: 22)
                .foregroundStyle(Tokens.foreground.color)
                .frame(width: FloatingButton.side, height: FloatingButton.side)
                .background(
                    Circle()
                        .fill(hovered ? Tokens.accent.color : Tokens.background.color)
                        .shadow(color: Tokens.shadow.opacity(Tokens.shadowOpacity).color, radius: 8, y: 2)
                )
                .overlay(Circle().strokeBorder(Tokens.border.color, lineWidth: Theme.hairline))
        }
    }
}
