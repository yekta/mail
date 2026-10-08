import SwiftUI

/// How tall a control is. Nothing clickable is under `small`; `regular` is the desktop's usual.
enum ControlSize {
    case small, regular, large

    var height: CGFloat {
        switch self {
        case .small: 28
        case .regular: 36
        case .large: 44
        }
    }

    var icon: CGFloat {
        switch self {
        case .small: 13
        case .regular: 15
        case .large: 17
        }
    }

    var text: TextStyle {
        switch self {
        case .small: .labelStrong
        case .regular: .labelStrong
        case .large: .bodyStrong
        }
    }
}

private struct HoveredKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    /// Whether the pointer is over the button the view is the face of.
    var hovered: Bool {
        get { self[HoveredKey.self] }
        set { self[HoveredKey.self] = newValue }
    }
}

/// How every button answers the pointer, a press and being disabled, so none does it a way of
/// its own. The face reads `hovered` from the environment and lights up at once.
struct PressStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        Pressed(configuration: configuration)
    }

    private struct Pressed: View {
        let configuration: Configuration
        @Environment(\.isEnabled) private var enabled
        @State private var hovering = false

        var body: some View {
            configuration.label
                .environment(\.hovered, hovering && enabled)
                .opacity(enabled ? (configuration.isPressed ? 0.6 : 1) : 0.4)
                .onHover { hovering = $0 }
        }
    }
}

extension ButtonStyle where Self == PressStyle {
    static var press: PressStyle { PressStyle() }
}

/// The icon of a button, or the spinner in its place while the button is pending.
struct ButtonIcon: View {
    var symbol: Symbol?
    var pending = false
    var size: CGFloat = 15
    var tint: Color

    var body: some View {
        if pending {
            ProgressView().controlSize(.small).tint(tint).frame(width: size, height: size)
        } else if let symbol {
            Image(symbol, size: size).foregroundStyle(tint)
        }
    }
}

/// A button with words. `primary` is filled, `outline` is Newton's pill (Compose), `ghost` is
/// words alone, `destructive` a pill in red. `pending` shows the spinner where the icon goes
/// and ignores clicks until it is over.
struct ActionButton: View {
    enum Variant {
        case primary, outline, ghost, destructive
    }

    let title: String
    var symbol: Symbol?
    var variant: Variant = .outline
    var size: ControlSize = .regular
    var pending = false
    var wide = false
    let action: () -> Void

    var body: some View {
        Button(action: { if !pending { action() } }) {
            Face(title: title, symbol: symbol, variant: variant, size: size, pending: pending, wide: wide)
        }
        .buttonStyle(.press)
    }

    private struct Face: View {
        let title: String
        let symbol: Symbol?
        let variant: Variant
        let size: ControlSize
        let pending: Bool
        let wide: Bool
        @Environment(\.hovered) private var hovered

        var body: some View {
            HStack(spacing: Space.xs + 2) {
                ButtonIcon(symbol: symbol, pending: pending, size: size.icon - 1, tint: foreground)
                Text(title).textStyle(size.text, color: foreground)
            }
            .frame(maxWidth: wide ? .infinity : nil)
            .padding(.horizontal, (size.height / 2) * Platform.scale)
            .frame(height: size.height * Platform.scale)
            .background(Capsule().fill(background))
            .overlay(Capsule().strokeBorder(border, lineWidth: Theme.hairline))
            .contentShape(Capsule())
        }

        private var foreground: Color {
            switch variant {
            case .primary: Tokens.primaryForeground.color
            case .outline: Tokens.primary.color
            case .ghost: hovered ? Tokens.foreground.color : Tokens.mutedForeground.color
            case .destructive: Tokens.destructive.color
            }
        }

        private var background: Color {
            switch variant {
            case .primary: hovered ? Tokens.primary.color.opacity(0.88) : Tokens.primary.color
            case .outline, .ghost, .destructive: hovered ? Tokens.accent.color : .clear
            }
        }

        private var border: Color {
            switch variant {
            case .primary, .ghost: .clear
            case .outline: Tokens.primary.color.opacity(0.7)
            case .destructive: Tokens.destructive.color.opacity(0.6)
            }
        }
    }
}

/// An icon in a thin circle, as Newton's toolbars drew them. `active` is lit in the primary
/// colour: a reminder set, a filter on. `quiet` is drawn fainter, for a control that should
/// not call for attention.
struct IconButton: View {
    let symbol: Symbol
    var help: String
    var size: ControlSize = .regular
    var active = false
    var circled = true
    var quiet = false
    var pending = false
    let action: () -> Void

    var body: some View {
        Button(action: { if !pending { action() } }) {
            IconCircle(symbol: symbol, size: size, active: active, circled: circled, quiet: quiet, pending: pending)
        }
        .buttonStyle(.press)
        .help(help)
        .accessibilityLabel(help)
    }
}

/// An icon in a thin circle, like `IconButton`, that opens a menu.
struct IconMenu<Content: View>: View {
    let symbol: Symbol
    var help: String
    var size: ControlSize = .regular
    var circled = true
    @ViewBuilder let content: Content

    var body: some View {
        Menu {
            content
        } label: {
            IconCircle(symbol: symbol, size: size, circled: circled)
        }
        .menuStyle(.button)
        .buttonStyle(.press)
        .menuIndicator(.hidden)
        .fixedSize()
        .help(help)
        .accessibilityLabel(help)
    }
}

/// The face of an icon button and an icon menu.
struct IconCircle: View {
    let symbol: Symbol
    var size: ControlSize = .regular
    var active = false
    var circled = true
    var quiet = false
    var pending = false
    @Environment(\.hovered) private var hovered

    var body: some View {
        ButtonIcon(symbol: symbol, pending: pending, size: size.icon, tint: tint)
            .frame(width: size.height * Platform.scale, height: size.height * Platform.scale)
            .background(Circle().fill(hovered ? Tokens.accent.color : .clear))
            .overlay(Circle().strokeBorder(circled ? Tokens.border.color : .clear, lineWidth: Theme.hairline))
            .contentShape(Circle())
    }

    private var tint: Color {
        if active { return Tokens.primary.color }
        if quiet { return hovered ? Tokens.mutedForeground.color : Tokens.mutedMoreForeground.color }
        return hovered ? Tokens.foreground.color : Tokens.mutedForeground.color
    }
}

/// A button that is only its content: a star, a sender's name, a word in a toast. For what no
/// other button fits; it still hovers, presses and disables as they do, and is never under
/// the smallest control in height.
struct PlainButton<Label: View>: View {
    var help: String?
    let action: () -> Void
    @ViewBuilder let label: Label

    @ViewBuilder var body: some View {
        let button = Button(action: action) { Face(label: label) }.buttonStyle(.press)
        if let help {
            button.help(help).accessibilityLabel(help)
        } else {
            button
        }
    }

    private struct Face: View {
        let label: Label
        @Environment(\.hovered) private var hovered

        var body: some View {
            label
                .padding(.horizontal, Space.xs)
                .frame(minWidth: ControlSize.small.height, minHeight: ControlSize.small.height)
                .background(RoundedRectangle(cornerRadius: Theme.radius).fill(hovered ? Tokens.accent.color : .clear))
                .contentShape(Rectangle())
        }
    }
}
