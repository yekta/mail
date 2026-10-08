import SwiftUI

/// A hairline in the border colour, across or down.
struct Rule: View {
    var vertical = false

    var body: some View {
        Rectangle()
            .fill(Tokens.border.color)
            .frame(width: vertical ? Theme.hairline : nil, height: vertical ? nil : Theme.hairline)
    }
}

extension View {
    /// A hairline along one edge, drawn over the view so it takes no room.
    func rule(_ edge: Edge, inset: CGFloat = 0) -> some View {
        overlay(alignment: Alignment(edge)) {
            Rule(vertical: edge == .leading || edge == .trailing)
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
