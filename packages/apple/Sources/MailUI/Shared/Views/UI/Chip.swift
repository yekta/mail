import SwiftUI

/// A small pill: an address in a field, a file, a thread's label. `invalid` is an address that
/// can't be sent to.
struct Chip: View {
    enum Variant {
        case plain, invalid
    }

    let title: String
    var symbol: Symbol?
    var detail: String?
    var variant: Variant = .plain
    var pending = false
    var help: String?
    var remove: (() -> Void)?
    var action: (() -> Void)?
    @Environment(\.hovered) private var hovered

    var body: some View {
        if let action, remove == nil {
            Button(action: action) { content }
                .buttonStyle(.press)
                .help(help ?? title)
        } else {
            content.help(help ?? title)
        }
    }

    private var content: some View {
        HStack(spacing: Space.xs + 2) {
            ButtonIcon(symbol: symbol, pending: pending, size: 12, tint: variant == .invalid ? Tokens.destructive.color : Tokens.mutedForeground.color)
            Text(title).font(.ui(.caption)).lineLimit(1).truncationMode(.middle)
            if let detail {
                Text(detail).textStyle(.footnote).lineLimit(1)
            }
            if let remove {
                PlainButton(help: "Remove \(title)", action: remove) {
                    Image(.x, size: 11).foregroundStyle(Tokens.mutedMoreForeground.color)
                }
                .padding(.trailing, -Space.s)
            }
        }
        .padding(.horizontal, Space.s + 2)
        .frame(height: Theme.chipHeight * Platform.scale)
        .foregroundStyle(variant == .invalid ? Tokens.destructive.color : Tokens.foreground.color)
        .background(Capsule().fill(hovered && action != nil ? Tokens.accent.color : Tokens.muted.color))
        .overlay(Capsule().strokeBorder(variant == .invalid ? Tokens.destructive.color.opacity(0.6) : .clear, lineWidth: Theme.hairline))
        .contentShape(Capsule())
    }
}
