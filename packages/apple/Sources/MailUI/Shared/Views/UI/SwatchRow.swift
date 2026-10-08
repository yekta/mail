import SwiftUI

/// The account colours in a row, to pick one. The chosen one wears a ring.
struct SwatchRow: View {
    let selected: String
    let pick: (String) -> Void
    @State private var hovered: String?

    var body: some View {
        HStack(spacing: 0) {
            ForEach(Array(Theme.accountColorNames.enumerated()), id: \.element) { index, name in
                Button { pick(name) } label: {
                    Circle()
                        .fill(Theme.accountColor(name).color)
                        .frame(width: 12, height: 12)
                        .padding(3)
                        .overlay(Circle().strokeBorder(name == selected ? Tokens.foreground.color : .clear, lineWidth: 1.5))
                        .frame(width: 28 * Platform.scale, height: 28 * Platform.scale)
                        .background(RoundedRectangle(cornerRadius: Theme.radius).fill(hovered == name ? Tokens.accent.color : .clear))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .onHover { inside in
                    if inside {
                        hovered = name
                    } else if hovered == name {
                        hovered = nil
                    }
                }
                .help("Colour \(index + 1)")
                .accessibilityLabel("Colour \(index + 1)")
                .accessibilityAddTraits(name == selected ? .isSelected : [])
            }
        }
    }
}
