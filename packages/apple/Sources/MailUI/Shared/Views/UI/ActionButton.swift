import SwiftUI

/// A button with words. `primary` is filled, `outline` is Newton's pill (Compose), `ghost` is
/// words alone.
struct ActionButton: View {
    enum Variant {
        case primary, outline, ghost, destructive
    }

    let title: String
    var symbol: Symbol?
    var variant: Variant = .outline
    var pending = false
    var wide = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                if pending {
                    ProgressView().controlSize(.small).tint(foreground)
                } else if let symbol {
                    Image(symbol, size: 14)
                }
                Text(title).font(.ui(13, .medium))
            }
            .frame(maxWidth: wide ? .infinity : nil)
            .padding(.horizontal, 14 * Platform.scale)
            .frame(height: 30 * Platform.scale)
            .foregroundStyle(foreground)
            .background(Capsule().fill(background))
            .overlay(Capsule().strokeBorder(border, lineWidth: 1))
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .disabled(pending)
    }

    private var foreground: Color {
        switch variant {
        case .primary: Tokens.primaryForeground.color
        case .outline: Tokens.primary.color
        case .ghost: Tokens.secondaryForeground.color
        case .destructive: Tokens.destructive.color
        }
    }

    private var background: Color {
        variant == .primary ? Tokens.primary.color : .clear
    }

    private var border: Color {
        switch variant {
        case .primary, .ghost: .clear
        case .outline: Tokens.primary.color.opacity(0.7)
        case .destructive: Tokens.destructive.color.opacity(0.6)
        }
    }
}

/// An icon in a thin circle, as Newton's toolbars drew them.
struct IconButton: View {
    let symbol: Symbol
    var help: String
    var tint: Color = Tokens.secondaryForeground.color
    var circled = true
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(symbol, size: 15)
                .foregroundStyle(tint)
                .frame(width: 32 * Platform.scale, height: 32 * Platform.scale)
                .overlay(Circle().strokeBorder(circled ? Tokens.border.color : .clear, lineWidth: 1))
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .help(help)
        .accessibilityLabel(help)
    }
}
