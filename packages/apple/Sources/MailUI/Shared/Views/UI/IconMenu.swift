import SwiftUI

/// An icon in a thin circle, like `IconButton`, that opens a menu.
struct IconMenu<Content: View>: View {
    let symbol: Symbol
    var help: String
    var circled = true
    @ViewBuilder let content: Content

    var body: some View {
        Menu {
            content
        } label: {
            Image(symbol, size: 15)
                .foregroundStyle(Tokens.secondaryForeground.color)
                .frame(width: 32 * Platform.scale, height: 32 * Platform.scale)
                .overlay(Circle().strokeBorder(circled ? Tokens.border.color : .clear, lineWidth: 1))
                .contentShape(Circle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .help(help)
        .accessibilityLabel(help)
    }
}
