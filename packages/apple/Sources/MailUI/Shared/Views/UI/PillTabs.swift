import SwiftUI

/// Tabs on a line, each with an icon, a name and a count; the chosen one filled. With `reorder`,
/// a tab is dragged into another place (on iOS, after holding it) and the new order is given
/// when it is let go. `menu` is what a secondary click on a tab offers.
struct PillTabs<Tab: Identifiable & Hashable>: View where Tab.ID == String {
    let tabs: [Tab]
    let selected: String?
    let symbol: (Tab) -> Symbol
    let title: (Tab) -> String
    let count: (Tab) -> Int
    let pick: (Tab) -> Void
    var reorder: (([String]) -> Void)?
    var menu: (Tab) -> [RowMenuItem] = { _ in [] }

    var body: some View {
        MovableList(
            items: tabs, axis: .horizontal, group: { _ in reorder == nil ? nil : "tabs" },
            rowInset: EdgeInsets(top: 0, leading: 1, bottom: 0, trailing: 1), rowRadius: Theme.radius + 4,
            clicked: pick, moved: moved, menu: menu
        ) { tab in
            Face(symbol: symbol(tab), title: title(tab), count: count(tab), chosen: tab.id == selected)
        }
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
            HStack(spacing: Space.xs + 2) {
                Image(symbol, size: ControlSize.regular.icon - 1).foregroundStyle(foreground)
                Text(title).textStyle(chosen ? .labelStrong : .label, color: foreground).lineLimit(1)
                if count > 0 {
                    Text("\(count)").textStyle(.footnote, color: chosen ? foreground.opacity(0.6) : Tokens.mutedMoreForeground.color)
                }
            }
            .padding(.horizontal, Space.m * Platform.scale)
            .frame(height: ControlSize.regular.height * Platform.scale)
            .background(RoundedRectangle(cornerRadius: Theme.radius + 4).fill(chosen ? Tokens.accentStronger.color : hovered ? Tokens.accent.color : .clear))
            .padding(.horizontal, 1)
            .contentShape(Rectangle())
        }

        private var foreground: Color {
            if chosen { return Tokens.accentForeground.color }
            return hovered ? Tokens.foreground.color : Tokens.mutedForeground.color
        }
    }
}
