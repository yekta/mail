import SwiftUI

/// One choice of a short list: a suggestion, a time, a snippet. `highlighted` is the one the
/// arrow keys are on.
struct ChoiceRow: View {
    let title: String
    var detail: String?
    var symbol: Symbol?
    var highlighted = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Face(title: title, detail: detail, symbol: symbol, highlighted: highlighted)
        }
        .buttonStyle(.press)
    }

    private struct Face: View {
        let title: String
        let detail: String?
        let symbol: Symbol?
        let highlighted: Bool
        @Environment(\.hovered) private var hovered

        var body: some View {
            HStack(spacing: Space.s + 2) {
                if let symbol {
                    Image(symbol, size: 14).foregroundStyle(Tokens.mutedMoreForeground.color)
                }
                Text(title).textStyle(.body).lineLimit(1)
                Spacer(minLength: Space.m)
                if let detail {
                    Text(detail).textStyle(.caption).lineLimit(1)
                }
            }
            .padding(.horizontal, Space.s + 2)
            .frame(minHeight: Theme.rowHeight * Platform.scale)
            .background(RoundedRectangle(cornerRadius: Theme.radius).fill(highlighted || hovered ? Tokens.accent.color : .clear))
            .contentShape(Rectangle())
        }
    }
}

/// Choices under a field, one highlighted, scrolled to as the keys move; `empty` when there
/// are none.
struct ChoiceList<Item: Identifiable, Row: View>: View {
    let items: [Item]
    let highlighted: Int
    var empty: String?
    @ViewBuilder let row: (Int, Item) -> Row

    var body: some View {
        ScrollViewReader { scroller in
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                        row(index, item).id(item.id)
                    }
                    if items.isEmpty, let empty {
                        Text(empty).textStyle(.label, color: Tokens.mutedMoreForeground.color)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(Space.s + 2)
                    }
                }
                .padding(Space.s)
            }
            .onChange(of: highlighted) { _, index in
                guard items.indices.contains(index) else { return }
                scroller.scrollTo(items[index].id)
            }
        }
    }
}

extension Int {
    /// The index `step` away, kept inside a list of `count`.
    func moved(by step: Int, in count: Int) -> Int {
        Swift.max(0, Swift.min(self + step, count - 1))
    }
}

/// A card floating over the page, for suggestions and short menus.
struct PopupCard<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) { content }
            .padding(Space.xs + 2)
            .background(
                RoundedRectangle(cornerRadius: Theme.cardRadius).fill(Tokens.popover.color)
                    .shadow(color: Tokens.shadow.opacity(Tokens.shadowStrongerOpacity).color, radius: 12, y: 4)
            )
            .overlay(RoundedRectangle(cornerRadius: Theme.cardRadius).strokeBorder(Tokens.border.color, lineWidth: Theme.hairline))
    }
}
