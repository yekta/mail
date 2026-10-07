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

    var body: some View {
        if let action, remove == nil {
            Button(action: action) { content }
                .buttonStyle(.plain)
                .help(help ?? title)
        } else {
            content.help(help ?? title)
        }
    }

    private var content: some View {
        HStack(spacing: 6) {
            if pending {
                ProgressView().controlSize(.mini)
            } else if let symbol {
                Image(symbol, size: 12)
            }
            Text(title).font(.ui(12)).lineLimit(1).truncationMode(.middle)
            if let detail {
                Text(detail).font(.ui(11)).foregroundStyle(Tokens.mutedForeground.color).lineLimit(1)
            }
            if let remove {
                Button(action: remove) { Image(.x, size: 11) }
                    .buttonStyle(.plain)
                    .foregroundStyle(Tokens.mutedForeground.color)
                    .accessibilityLabel("Remove \(title)")
            }
        }
        .padding(.horizontal, 10)
        .frame(height: 24 * Platform.scale)
        .foregroundStyle(variant == .invalid ? Tokens.destructive.color : Tokens.foreground.color)
        .background(Capsule().fill(Tokens.muted.color))
        .overlay(Capsule().strokeBorder(variant == .invalid ? Tokens.destructive.color.opacity(0.6) : .clear, lineWidth: 1))
        .contentShape(Capsule())
    }
}
