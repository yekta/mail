import SwiftUI

/// A thing in a list with its controls at the end: an account, a snippet, a split. `symbol`
/// or `dot` starts it.
struct ItemRow<Trailing: View>: View {
    let title: String
    var detail: String?
    var symbol: Symbol?
    var dot: Color?
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 0) {
            if let dot {
                Circle().fill(dot).frame(width: 8, height: 8).padding(.trailing, Space.s + 2)
            } else if let symbol {
                Image(symbol, size: 13).foregroundStyle(Tokens.mutedMoreForeground.color).padding(.trailing, Space.s + 2)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(title).textStyle(.body).lineLimit(1).truncationMode(.middle)
                if let detail {
                    Text(detail).textStyle(.caption).lineLimit(1)
                }
            }
            Spacer(minLength: Space.s)
            HStack(spacing: 0) { trailing }
        }
        .frame(minHeight: Theme.rowHeight * Platform.scale)
    }
}

extension ItemRow where Trailing == EmptyView {
    init(title: String, detail: String? = nil, symbol: Symbol? = nil, dot: Color? = nil) {
        self.init(title: title, detail: detail, symbol: symbol, dot: dot, trailing: { EmptyView() })
    }
}

/// A row that goes somewhere: a mailbox in the sidebar, Settings. `selected` is where the app
/// is. `count` is its unread, shown when above zero.
struct NavRow<Trailing: View>: View {
    let title: String
    var symbol: Symbol?
    var dot: Color?
    var count = 0
    var selected = false
    var indent: CGFloat = 0
    let action: () -> Void
    @ViewBuilder var trailing: Trailing

    var body: some View {
        Button(action: action) {
            Face(title: title, symbol: symbol, dot: dot, count: count, selected: selected, indent: indent, trailing: trailing)
        }
        .buttonStyle(.press)
    }

    private struct Face: View {
        let title: String
        let symbol: Symbol?
        let dot: Color?
        let count: Int
        let selected: Bool
        let indent: CGFloat
        let trailing: Trailing
        @Environment(\.hovered) private var hovered

        var body: some View {
            HStack(spacing: Space.m) {
                Group {
                    if let dot {
                        Circle().fill(dot).frame(width: 8, height: 8)
                    } else if let symbol {
                        Image(symbol, size: 15)
                    }
                }
                .frame(width: 18)
                Text(title).textStyle(selected ? .labelStrong : .label, color: Tokens.foreground.color).lineLimit(1).truncationMode(.middle)
                Spacer()
                if count > 0 {
                    Text("\(count)").textStyle(.caption)
                }
                trailing
            }
            .foregroundStyle(Tokens.foreground.color)
            .padding(.leading, 18 + indent)
            .padding(.trailing, Space.l)
            .frame(height: Theme.rowHeight * Platform.scale)
            .background(selected ? Tokens.accent.color : hovered ? Tokens.accent.color.opacity(0.6) : .clear)
            .contentShape(Rectangle())
        }
    }
}

extension NavRow where Trailing == EmptyView {
    init(title: String, symbol: Symbol? = nil, dot: Color? = nil, count: Int = 0, selected: Bool = false, indent: CGFloat = 0, action: @escaping () -> Void) {
        self.init(title: title, symbol: symbol, dot: dot, count: count, selected: selected, indent: indent, action: action, trailing: { EmptyView() })
    }
}
