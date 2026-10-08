import SwiftUI

/// The account colours on a line, to pick one. The chosen one wears a ring; each hovers in a
/// wash of its own colour.
struct SwatchRow: View {
    let selected: String
    let pick: (String) -> Void

    var body: some View {
        HStack(spacing: 0) {
            ForEach(Array(Theme.accountColorNames.enumerated()), id: \.element) { index, name in
                PlainButton(help: "Colour \(index + 1)", tint: Theme.accountColor(name), action: { pick(name) }) {
                    Circle()
                        .fill(Theme.accountColor(name).color)
                        .frame(width: 12, height: 12)
                        .padding(3)
                        .overlay(Circle().strokeBorder(name == selected ? Tokens.foreground.color : .clear, lineWidth: 1.5))
                }
                .accessibilityAddTraits(name == selected ? .isSelected : [])
            }
        }
    }
}
