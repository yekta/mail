import SwiftUI

/// Tabs on a line, each with a count; the chosen one underlined.
struct TabStrip<Tab: Identifiable & Hashable>: View {
    let tabs: [Tab]
    let selected: Tab.ID?
    let title: (Tab) -> String
    let count: (Tab) -> Int
    let pick: (Tab) -> Void

    var body: some View {
        HStack(spacing: 0) {
            ForEach(tabs) { tab in
                Button {
                    pick(tab)
                } label: {
                    Face(title: title(tab), count: count(tab), chosen: tab.id == selected)
                }
                .buttonStyle(.press)
            }
        }
    }

    private struct Face: View {
        let title: String
        let count: Int
        let chosen: Bool
        @Environment(\.hovered) private var hovered

        var body: some View {
            HStack(spacing: Space.xs + 2) {
                Text(title).font((chosen ? TextStyle.labelStrong : .label).font)
                if count > 0 {
                    Text("\(count)").textStyle(.footnote)
                }
            }
            .foregroundStyle(chosen || hovered ? Tokens.foreground.color : Tokens.mutedForeground.color)
            .padding(.horizontal, Space.s + 1)
            .frame(height: Theme.rowHeight * Platform.scale)
            .overlay(alignment: .bottom) {
                Rectangle().fill(chosen ? Tokens.primary.color : .clear).frame(height: 2)
            }
            .contentShape(Rectangle())
        }
    }
}
