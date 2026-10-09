import SwiftUI

/// Tabs across the bottom of the screen, each an icon over its name and count, the chosen one in
/// the text's colour. They share the width equally, so however many there are they fit; a long
/// name is cut short. With `reorder`, a tab held and dragged goes to another place, and the new
/// order is given when it is let go. The page's colour reaches under the home indicator.
struct BottomTabs<Tab: Identifiable & Hashable>: View {
    let tabs: [Tab]
    let selected: Tab.ID?
    let symbol: (Tab) -> Symbol
    let title: (Tab) -> String
    let count: (Tab) -> Int
    let pick: (Tab) -> Void
    var reorder: (([Tab.ID]) -> Void)?

    var body: some View {
        MovableList(
            items: tabs, axis: .horizontal, inset: Space.xs, fills: true, group: { _ in reorder == nil ? nil : "tabs" },
            rowInset: EdgeInsets(top: Space.xs, leading: 0, bottom: Space.xs, trailing: 0), rowRadius: Theme.radius + 4,
            clicked: pick, moved: moved
        ) { tab in
            Face(symbol: symbol(tab), title: title(tab), count: count(tab), chosen: tab.id == selected)
        }
        .background(Tokens.background.color.ignoresSafeArea(edges: .bottom))
        .rule(.top)
    }

    private func moved(_ tab: Tab, to index: Int) {
        var ids = tabs.map(\.id).filter { $0 != tab.id }
        ids.insert(tab.id, at: min(index, ids.count))
        reorder?(ids)
    }

    private struct Face: View {
        let symbol: Symbol
        let title: String
        let count: Int
        let chosen: Bool
        @Environment(\.hovered) private var hovered

        var body: some View {
            VStack(spacing: Space.xs - 1) {
                Image(symbol, size: 18).foregroundStyle(foreground)
                HStack(spacing: Space.xs - 1) {
                    Text(title).textStyle(chosen ? .footnoteStrong : .footnote, color: foreground).lineLimit(1)
                    if count > 0 {
                        Text("\(count)").textStyle(.footnote).lineLimit(1).fixedSize()
                    }
                }
            }
            .padding(.horizontal, Space.xs)
            .padding(.top, Space.m)
            .padding(.bottom, Space.s)
            .frame(maxWidth: .infinity)
            .contentShape(Rectangle())
        }

        private var foreground: Color {
            chosen || hovered ? Tokens.foreground.color : Tokens.mutedMoreForeground.color
        }
    }
}
