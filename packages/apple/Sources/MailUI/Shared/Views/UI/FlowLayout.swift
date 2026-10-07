import SwiftUI

/// Lays its views out in lines, wrapping as text does. The last one takes the rest of its line,
/// for a text field after an address field's chips.
struct FlowLayout: Layout {
    var spacing: CGFloat = 6
    var lineSpacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        let frames = place(subviews, width: width)
        let height = frames.map(\.maxY).max() ?? 0
        let used = frames.map(\.maxX).max() ?? 0
        return CGSize(width: proposal.width.map { $0.isFinite ? $0 : used } ?? used, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let frames = place(subviews, width: bounds.width)
        for (subview, frame) in zip(subviews, frames) {
            subview.place(at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY), proposal: ProposedViewSize(frame.size))
        }
    }

    private func place(_ subviews: Subviews, width: CGFloat) -> [CGRect] {
        var frames: [CGRect] = []
        var x: CGFloat = 0
        var y: CGFloat = 0
        var lineHeight: CGFloat = 0
        for (index, subview) in subviews.enumerated() {
            var size = subview.sizeThatFits(.unspecified)
            if x > 0, x + size.width > width {
                x = 0
                y += lineHeight + lineSpacing
                lineHeight = 0
            }
            if index == subviews.count - 1, width.isFinite {
                size.width = max(size.width, width - x)
            }
            size.width = min(size.width, width)
            frames.append(CGRect(origin: CGPoint(x: x, y: y), size: size))
            x += size.width + spacing
            lineHeight = max(lineHeight, size.height)
        }
        // Each view sits in the middle of its line.
        return frames.map { frame in
            let line = frames.filter { abs($0.minY - frame.minY) < 0.5 }
            let height = line.map(\.height).max() ?? frame.height
            return frame.offsetBy(dx: 0, dy: (height - frame.height) / 2)
        }
    }
}
