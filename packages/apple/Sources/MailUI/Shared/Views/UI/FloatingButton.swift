import SwiftUI

/// An icon on a circle of clear glass, floating over a page's corner: Compose over the list on
/// iOS. The page shows through it as it scrolls under. Before glass, a card with a shadow.
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
            let icon = Image(symbol, size: 22)
                .foregroundStyle(Tokens.foreground.color)
                .frame(width: FloatingButton.side, height: FloatingButton.side)
            if #available(iOS 26, macOS 26, *) {
                icon.glassEffect(.clear.interactive(), in: .circle)
            } else {
                icon
                    .background(Circle().fill(hovered ? Tokens.accent.color : Tokens.card.color))
                    .overlay(Circle().strokeBorder(Tokens.border.color, lineWidth: Theme.hairline))
                    .shadow(color: Tokens.shadow.opacity(Tokens.shadowOpacity).color, radius: 12, y: 4)
            }
        }
    }
}
