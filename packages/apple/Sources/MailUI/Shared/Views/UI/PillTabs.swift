import SwiftUI

/// Pills on a line, each with an icon, a name and a count; the chosen one filled. With `reorder`,
/// dragging a pill (on iOS, after holding it) moves it, and the new order is given when it is let
/// go. `menu` is what a secondary click on a pill offers.
struct PillTabs<Tab: Identifiable & Hashable, Menu: View>: View where Tab.ID == String {
    let tabs: [Tab]
    let selected: String?
    let symbol: (Tab) -> Symbol
    let title: (Tab) -> String
    let count: (Tab) -> Int
    let pick: (Tab) -> Void
    var reorder: (([String]) -> Void)?
    @ViewBuilder let menu: (Tab) -> Menu

    private struct Drag {
        let id: String
        var order: [String]
        let startCenter: CGFloat
        var translation: CGFloat = 0
    }

    @State private var widths: [String: CGFloat] = [:]
    @State private var drag: Drag?

    var body: some View {
        let order = drag?.order ?? tabs.map(\.id)
        let shown = order.compactMap { id in tabs.first(where: { $0.id == id }) }
        HStack(spacing: 0) {
            ForEach(shown) { tab in
                Button {
                    pick(tab)
                } label: {
                    Face(symbol: symbol(tab), title: title(tab), count: count(tab), chosen: tab.id == selected)
                }
                .buttonStyle(.press)
                .contextMenu { menu(tab) }
                .background(GeometryReader { geometry in
                    Color.clear.preference(key: WidthsKey.self, value: [tab.id: geometry.size.width])
                })
                .offset(x: drag?.id == tab.id ? offset : 0)
                .zIndex(drag?.id == tab.id ? 1 : 0)
                .simultaneousGesture(dragGesture(tab), including: reorder == nil ? .subviews : .all)
            }
        }
        .onPreferenceChange(WidthsKey.self) { widths = $0 }
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
            .background(Capsule().fill(chosen ? Tokens.accent.color : hovered ? Tokens.accentStronger.color : .clear))
            .padding(.horizontal, 1)
            .contentShape(Rectangle())
        }

        private var foreground: Color {
            if chosen { return Tokens.accentForeground.color }
            return hovered ? Tokens.foreground.color : Tokens.mutedForeground.color
        }
    }

    /// Where the dragged pill is drawn, from where the cursor took it to where its slot is now.
    private var offset: CGFloat {
        guard let drag else { return 0 }
        return drag.startCenter + drag.translation - center(of: drag.id, in: drag.order)
    }

    private func center(of id: String, in order: [String]) -> CGFloat {
        var x: CGFloat = 0
        for other in order {
            let width = widths[other] ?? 0
            if other == id { return x + width / 2 }
            x += width
        }
        return x
    }

    private func moved(_ tab: Tab, by translation: CGFloat) {
        if drag == nil {
            let order = tabs.map(\.id)
            drag = Drag(id: tab.id, order: order, startCenter: center(of: tab.id, in: order))
        }
        guard var moving = drag, moving.id == tab.id else { return }
        moving.translation = translation
        var order = moving.order
        guard let from = order.firstIndex(of: tab.id) else { return }
        let x = moving.startCenter + translation
        if from + 1 < order.count, x > center(of: order[from + 1], in: order) {
            order.swapAt(from, from + 1)
        } else if from > 0, x < center(of: order[from - 1], in: order) {
            order.swapAt(from, from - 1)
        }
        moving.order = order
        guard order != drag?.order else {
            drag = moving
            return
        }
        withAnimation(.easeOut(duration: 0.15)) { drag = moving }
    }

    private func ended() {
        guard let drag else { return }
        if drag.order != tabs.map(\.id) { reorder?(drag.order) }
        self.drag = nil
    }

    #if os(macOS)
    private func dragGesture(_ tab: Tab) -> some Gesture {
        DragGesture(minimumDistance: 4)
            .onChanged { moved(tab, by: $0.translation.width) }
            .onEnded { _ in ended() }
    }
    #else
    private func dragGesture(_ tab: Tab) -> some Gesture {
        LongPressGesture(minimumDuration: 0.25)
            .sequenced(before: DragGesture(minimumDistance: 0))
            .onChanged { value in
                guard case .second(true, let drag?) = value else { return }
                moved(tab, by: drag.translation.width)
            }
            .onEnded { _ in ended() }
    }
    #endif
}

private struct WidthsKey: PreferenceKey {
    static let defaultValue: [String: CGFloat] = [:]
    static func reduce(value: inout [String: CGFloat], nextValue: () -> [String: CGFloat]) {
        value.merge(nextValue(), uniquingKeysWith: { _, last in last })
    }
}
