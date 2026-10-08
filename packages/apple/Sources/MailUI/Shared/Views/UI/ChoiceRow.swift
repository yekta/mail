import SwiftUI

/// One choice of a short list: a suggestion, a time, a snippet. `highlighted` is the one the
/// arrow keys are on.
struct ChoiceRow: View {
    let title: String
    var detail: String?
    var symbol: Symbol?
    var highlighted = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                if let symbol {
                    Image(symbol, size: 14).foregroundStyle(Tokens.mutedMoreForeground.color)
                }
                Text(title).font(.ui(14)).foregroundStyle(Tokens.foreground.color).lineLimit(1)
                Spacer(minLength: 12)
                if let detail {
                    Text(detail).font(.ui(12)).foregroundStyle(Tokens.mutedMoreForeground.color).lineLimit(1)
                }
            }
            .padding(.horizontal, 10)
            .frame(minHeight: 32 * Platform.scale)
            .background(RoundedRectangle(cornerRadius: Theme.radius).fill(highlighted ? Tokens.accent.color : .clear))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// A card floating over the page, for suggestions and short menus.
struct PopupCard<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) { content }
            .padding(6)
            .background(RoundedRectangle(cornerRadius: Theme.radius + 2).fill(Tokens.popover.color).shadow(color: Tokens.shadow.opacity(Tokens.shadowStrongerOpacity).color, radius: 12, y: 4))
            .overlay(RoundedRectangle(cornerRadius: Theme.radius + 2).strokeBorder(Tokens.border.color, lineWidth: 1))
    }
}
