#if os(macOS)
import AppKit
import SwiftUI

/// The thread list on the Mac: a table that only makes the rows on screen, each drawn in one
/// pass. Hovering a row shows what can be done to it, as Newton did.
struct ThreadListMac: NSViewRepresentable {
    let store: MailStore
    let rows: [ThreadRow]
    let selected: String?

    func makeCoordinator() -> Coordinator { Coordinator(store: store) }

    func makeNSView(context: Context) -> NSScrollView {
        let table = HoverTableView()
        table.addTableColumn(NSTableColumn(identifier: NSUserInterfaceItemIdentifier("thread")))
        table.headerView = nil
        table.rowHeight = Theme.macRowHeight
        table.intercellSpacing = .zero
        table.backgroundColor = .clear
        table.selectionHighlightStyle = .none
        table.style = .plain
        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        table.target = context.coordinator
        table.action = #selector(Coordinator.clicked)
        table.focusRingType = .none

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        context.coordinator.table = table
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.update(rows: rows, selected: selected)
    }

    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        let store: MailStore
        weak var table: HoverTableView?
        private var rows: [ThreadRow] = []
        private var texts: [String: (ThreadRow, RowText)] = [:]
        private var selected: String?

        init(store: MailStore) {
            self.store = store
        }

        func update(rows: [ThreadRow], selected: String?) {
            guard let table else { return }
            let changed = rows != self.rows
            let moved = selected != self.selected
            self.rows = rows
            self.selected = selected
            if changed {
                table.reloadData()
            } else if moved {
                table.enumerateAvailableRowViews { view, _ in
                    guard let view = view as? ThreadRowView else { return }
                    view.isCurrent = view.row?.id == selected
                }
            }
            if moved, let selected, let index = rows.firstIndex(where: { $0.id == selected }) {
                table.scrollRowToVisible(index)
            }
        }

        private func text(for row: ThreadRow) -> RowText {
            if let (kept, text) = texts[row.id], kept == row { return text }
            let text = RowText(row)
            texts[row.id] = (row, text)
            return text
        }

        func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

        func tableView(_ tableView: NSTableView, rowViewForRow index: Int) -> NSTableRowView? {
            let identifier = NSUserInterfaceItemIdentifier("row")
            let view = tableView.makeView(withIdentifier: identifier, owner: nil) as? ThreadRowView ?? ThreadRowView()
            view.identifier = identifier
            let row = rows[index]
            view.configure(row: row, text: text(for: row), current: row.id == selected, hovering: (tableView as? HoverTableView)?.hovered == index)
            if index > rows.count - 30 { DispatchQueue.main.async { self.store.loadMore() } }
            return view
        }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? { nil }

        @MainActor @objc func clicked() {
            guard let table, table.clickedRow >= 0, table.clickedRow < rows.count else { return }
            let row = rows[table.clickedRow]
            guard let view = table.rowView(atRow: table.clickedRow, makeIfNecessary: false) as? ThreadRowView,
                  let event = NSApp.currentEvent
            else { return }
            let point = view.convert(event.locationInWindow, from: nil)
            switch view.hit(point) {
            case .star: store.toggleStar(row.id)
            case .archive: store.act(.archive, on: [row.id])
            case .trash: store.act(.trash, on: [row.id])
            case .snooze: store.snoozing = [row.id]
            case .read: store.toggleRead(row.id)
            case .none: store.open(row.id)
            }
        }
    }
}

/// Tells its rows when the pointer is over them.
final class HoverTableView: NSTableView {
    var hovered = -1 {
        didSet {
            guard hovered != oldValue else { return }
            for index in [oldValue, hovered] where index >= 0 && index < numberOfRows {
                (rowView(atRow: index, makeIfNecessary: false) as? ThreadRowView)?.hovering = index == hovered
            }
        }
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: bounds, options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self))
    }

    override func mouseMoved(with event: NSEvent) {
        hovered = row(at: convert(event.locationInWindow, from: nil))
    }

    override func mouseExited(with event: NSEvent) {
        hovered = -1
    }
}

/// One thread on one line: the account's bar, unread dot, senders, subject and snippet, date, star.
final class ThreadRowView: NSTableRowView {
    enum Hit {
        case star, archive, trash, snooze, read, none
    }

    private(set) var row: ThreadRow?
    private var text: RowText?
    var isCurrent = false { didSet { if isCurrent != oldValue { needsDisplay = true } } }
    var hovering = false { didSet { if hovering != oldValue { needsDisplay = true } } }

    private static let sendersX: CGFloat = 36
    private static let sendersWidth: CGFloat = 190
    private static let iconSide: CGFloat = 16

    func configure(row: ThreadRow, text: RowText, current: Bool, hovering: Bool) {
        self.row = row
        self.text = text
        self.isCurrent = current
        self.hovering = hovering
        needsDisplay = true
    }

    override var isFlipped: Bool { true }

    private var starRect: NSRect {
        NSRect(x: bounds.maxX - 40, y: (bounds.height - Self.iconSide) / 2, width: Self.iconSide, height: Self.iconSide)
    }

    private var actions: [(Hit, Symbol, NSRect)] {
        guard let row else { return [] }
        let kinds: [(Hit, Symbol)] = [(.read, row.unread ? .mailOpen : .mail), (.snooze, .clock), (.trash, .trash), (.archive, .archive)]
        return kinds.enumerated().map { index, kind in
            let x = starRect.minX - 34 - CGFloat(index) * 30
            return (kind.0, kind.1, NSRect(x: x, y: (bounds.height - Self.iconSide) / 2, width: Self.iconSide, height: Self.iconSide))
        }
    }

    func hit(_ point: NSPoint) -> Hit {
        if starRect.insetBy(dx: -8, dy: -12).contains(point) { return .star }
        guard hovering else { return .none }
        return actions.first(where: { $0.2.insetBy(dx: -7, dy: -12).contains(point) })?.0 ?? .none
    }

    override func drawBackground(in dirtyRect: NSRect) {
        let fill = isCurrent || hovering ? Tokens.accent.platform : Tokens.card.platform
        fill.setFill()
        bounds.fill()
        Tokens.border.platform.setFill()
        NSRect(x: 0, y: bounds.maxY - 1, width: bounds.width, height: 1).fill()
        if let row {
            Theme.accountColor(row.color).platform.setFill()
            NSRect(x: 0, y: 0, width: Theme.accountBarWidth, height: bounds.height).fill()
        }
    }

    override func drawSelection(in dirtyRect: NSRect) {}

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard let row, let text else { return }
        let middle = bounds.height / 2
        if row.unread {
            Tokens.primary.platform.setFill()
            NSBezierPath(ovalIn: NSRect(x: 18, y: middle - 3.5, width: 7, height: 7)).fill()
        }
        Self.drawLine(text.senders, x: Self.sendersX, width: Self.sendersWidth, middle: middle)
        var x = Self.sendersX + Self.sendersWidth + 14
        if row.attachment {
            Self.drawSymbol(.paperclip, in: NSRect(x: x, y: middle - 7, width: 14, height: 14), color: Tokens.mutedForeground.platform)
        }
        x += 24
        let trailing: CGFloat
        if hovering {
            trailing = (actions.last?.2.minX ?? starRect.minX) - 16
            for (_, symbol, rect) in actions {
                Self.drawSymbol(symbol, in: rect, color: Tokens.secondaryForeground.platform)
            }
        } else {
            let dateWidth = ceil(text.date.size().width)
            Self.drawLine(text.date, x: starRect.minX - 18 - dateWidth, width: dateWidth + 2, middle: middle)
            trailing = starRect.minX - 34 - dateWidth
        }
        Self.drawLine(text.line, x: x, width: max(trailing - x, 0), middle: middle)
        let starColor = row.starred ? Tokens.star.platform : Tokens.input.platform
        Self.drawSymbol(row.starred ? .starFilled : .star, in: starRect, color: starColor)
    }

    private static func drawLine(_ text: NSAttributedString, x: CGFloat, width: CGFloat, middle: CGFloat) {
        guard width > 8 else { return }
        let height = ceil(text.size().height)
        text.draw(with: NSRect(x: x, y: middle - height / 2, width: width, height: height), options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
    }

    private static func drawSymbol(_ symbol: Symbol, in rect: NSRect, color: NSColor) {
        let image = NSImage.symbol(symbol, side: rect.width)
        let tinted = NSImage(size: rect.size, flipped: false) { frame in
            image.draw(in: frame)
            color.set()
            frame.fill(using: .sourceAtop)
            return true
        }
        tinted.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
    }
}
#endif
