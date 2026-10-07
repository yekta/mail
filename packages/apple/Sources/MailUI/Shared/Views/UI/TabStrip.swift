import SwiftUI

/// Tabs on a line, each with a count; the chosen one underlined.
struct TabStrip<Tab: Identifiable & Hashable>: View {
    let tabs: [Tab]
    let selected: Tab.ID?
    let title: (Tab) -> String
    let count: (Tab) -> Int
    let pick: (Tab) -> Void

    var body: some View {
        HStack(spacing: 18) {
            ForEach(tabs) { tab in
                let chosen = tab.id == selected
                Button {
                    pick(tab)
                } label: {
                    HStack(spacing: 6) {
                        Text(title(tab)).font(.ui(13, chosen ? .semibold : .regular))
                        if count(tab) > 0 {
                            Text("\(count(tab))").font(.ui(11.5)).foregroundStyle(Tokens.mutedForeground.color)
                        }
                    }
                    .foregroundStyle(chosen ? Tokens.foreground.color : Tokens.secondaryForeground.color)
                    .frame(height: 34 * Platform.scale)
                    .overlay(alignment: .bottom) {
                        Rectangle().fill(chosen ? Tokens.primary.color : .clear).frame(height: 2)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
    }
}
