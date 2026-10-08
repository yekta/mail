import SwiftUI

/// A few choices on a line, one of them chosen: the appearance, the undo delay.
struct Segmented<Option: Hashable>: View {
    let options: [Option]
    @Binding var selection: Option
    let title: (Option) -> String

    var body: some View {
        HStack(spacing: 0) {
            ForEach(options, id: \.self) { option in
                Button {
                    selection = option
                } label: {
                    Segment(title: title(option), chosen: option == selection)
                }
                .buttonStyle(.press)
            }
        }
        .padding(3)
        .background(RoundedRectangle(cornerRadius: Theme.radius + 2).fill(Tokens.muted.color))
        .overlay(RoundedRectangle(cornerRadius: Theme.radius + 2).strokeBorder(Tokens.border.color, lineWidth: Theme.hairline))
    }
}

private struct Segment: View {
    let title: String
    let chosen: Bool
    @Environment(\.hovered) private var hovered

    var body: some View {
        Text(title)
            .textStyle(chosen ? .labelStrong : .label, color: chosen || hovered ? Tokens.foreground.color : Tokens.mutedForeground.color)
            .padding(.horizontal, Space.m)
            .frame(height: ControlSize.small.height * Platform.scale)
            .background(RoundedRectangle(cornerRadius: Theme.radius - 1).fill(chosen ? Tokens.accentStronger.color : hovered ? Tokens.accent.color : .clear))
            .contentShape(RoundedRectangle(cornerRadius: Theme.radius - 1))
    }
}

/// A choice from a short list, under a label: which label, which sender. `none` is the
/// choice of nothing, shown when `selection` is nil.
struct Dropdown<Option: Hashable>: View {
    var label: String?
    let options: [Option]
    @Binding var selection: Option?
    var none: String?
    let title: (Option) -> String

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            if let label {
                Text(label).textStyle(.footnote, color: Tokens.mutedMoreForeground.color).fontWeight(.medium)
            }
            Menu {
                if let none {
                    Button(none) { selection = nil }
                }
                ForEach(options, id: \.self) { option in
                    Button {
                        selection = option
                    } label: {
                        if option == selection {
                            Label(title(option), systemImage: "checkmark")
                        } else {
                            Text(title(option))
                        }
                    }
                }
            } label: {
                DropdownFace(text: selection.map(title) ?? none ?? "")
            }
            .menuStyle(.button)
            .buttonStyle(.press)
            .menuIndicator(.hidden)
        }
    }
}

private struct DropdownFace: View {
    let text: String
    @Environment(\.hovered) private var hovered

    var body: some View {
        HStack(spacing: Space.s) {
            Text(text).textStyle(.body).lineLimit(1)
            Spacer(minLength: Space.s)
            Image(.chevronDown, size: 12).foregroundStyle(Tokens.mutedMoreForeground.color)
        }
        .padding(.horizontal, Space.s + 2)
        .frame(height: Theme.fieldHeight * Platform.scale)
        .background(RoundedRectangle(cornerRadius: Theme.radius).fill(hovered ? Tokens.accent.color : Tokens.card.color))
        .overlay(RoundedRectangle(cornerRadius: Theme.radius).strokeBorder(Tokens.input.color, lineWidth: Theme.hairline))
        .contentShape(RoundedRectangle(cornerRadius: Theme.radius))
    }
}

/// More under a line that opens it: Advanced.
struct Disclosure<Content: View>: View {
    let title: String
    @State private var open = false
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            ActionButton(title: title, symbol: open ? .chevronDown : .chevronRight, variant: .ghost, size: .small) {
                withAnimation(.easeOut(duration: 0.15)) { open.toggle() }
            }
            if open {
                content
            }
        }
    }
}
