import SwiftUI

/// A hairline across or down: `border` on the page, `cardBorder` on a card, where `border` is
/// too strong.
struct Rule: View {
    var vertical = false
    var color: ThemeColor = Tokens.border

    var body: some View {
        Rectangle()
            .fill(color.color)
            .frame(width: vertical ? Theme.hairline : nil, height: vertical ? nil : Theme.hairline)
    }
}

extension View {
    /// A hairline along one edge, drawn over the view so it takes no room.
    func rule(_ edge: Edge, inset: CGFloat = 0, color: ThemeColor = Tokens.border) -> some View {
        overlay(alignment: Alignment(edge)) {
            Rule(vertical: edge == .leading || edge == .trailing, color: color)
                .padding(edge == .top || edge == .bottom ? .leading : .top, inset)
        }
    }
}

private extension Alignment {
    init(_ edge: Edge) {
        switch edge {
        case .top: self = .top
        case .bottom: self = .bottom
        case .leading: self = .leading
        case .trailing: self = .trailing
        }
    }
}
