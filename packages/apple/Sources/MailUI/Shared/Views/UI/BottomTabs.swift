import SwiftUI

/// Tabs across the bottom of the screen, each an icon over its name and count, the chosen one in
/// the text's colour. They share the width equally, so however many there are they fit; a long
/// name is cut short. The page's colour reaches under the home indicator.
struct BottomTabs<Tab: Identifiable & Hashable>: View {
    let tabs: [Tab]
    let selected: Tab.ID?
    let symbol: (Tab) -> Symbol
    let title: (Tab) -> String
    let count: (Tab) -> Int
    let pick: (Tab) -> Void

    var body: some View {
        HStack(spacing: 0) {
            ForEach(tabs) { tab in
                Button {
                    pick(tab)
                } label: {
                    Face(symbol: symbol(tab), title: title(tab), count: count(tab), chosen: tab.id == selected)
                }
                .buttonStyle(.press)
            }
        }
        .padding(.horizontal, Space.xs)
        .background(Tokens.background.color.ignoresSafeArea(edges: .bottom))
        .rule(.top)
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
                    Text(title).textStyle(.footnote, color: foreground).lineLimit(1)
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
            chosen || hovered ? Tokens.foreground.color : Tokens.mutedForeground.color
        }
    }
}
