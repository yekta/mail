#if os(macOS)
import AppKit
import SwiftUI

/// The thread list on the Mac: a table that only makes the rows on screen, each drawn in one
/// pass. It spans the window, so the whole page scrolls, and draws its rows on the page in the
/// centred column, framed by a hairline, with one between the rows. Hovering a row shows what can be done to it, as Newton did. ⌘-click picks rows,
/// Shift-click picks the rows up to one.
struct ThreadListMac: NSViewRepresentable {
    let store: MailStore
    let rows: [ThreadRow]
    let selected: String?
    let checked: Set<String>
    let topInset: CGFloat
    /// False under an open thread: the list keeps its place, and gives up the keyboard.
    let shown: Bool
    /// False while the rows are the mailbox before; when they are the new one's, the list goes to the top.
    let ready: Bool

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
        scroll.automaticallyAdjustsContentInsets = false
        pad(scroll)
        scroll.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(
            context.coordinator, selector: #selector(Coordinator.scrolled), name: NSView.boundsDidChangeNotification, object: scroll.contentView
        )
        context.coordinator.table = table
        context.coordinator.keeper = ScrollKeeper(restore: store.listOffset, report: store.noteScroll)
        context.coordinator.keeper?.attach(scroll)
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        if scroll.contentInsets.top != topInset { pad(scroll) }
        if scroll.isHidden == shown { scroll.isHidden = !shown }
        context.coordinator.update(rows: rows, selected: selected, checked: checked, ready: ready)
    }

    /// Pads the rows, not the scroll view, so the padding scrolls with them and the scroller
    /// runs the full height.
    private func pad(_ scroll: NSScrollView) {
        scroll.contentInsets = NSEdgeInsets(top: topInset, left: 0, bottom: Self.bottomInset, right: 0)
        scroll.scrollerInsets = NSEdgeInsets(top: -topInset, left: 0, bottom: -Self.bottomInset, right: 0)
    }

    private static let bottomInset = Space.l

    static func dismantleNSView(_ scroll: NSScrollView, coordinator: Coordinator) {
        NotificationCenter.default.removeObserver(coordinator)
    }

    @MainActor
    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        let store: MailStore
        weak var table: HoverTableView?
        private var rows: [ThreadRow] = []
        private var texts: [String: (ThreadRow, RowText)] = [:]
        private var selected: String?
        private var checked: Set<String> = []
        private var ready = true
        private var prefetch: DispatchWorkItem?
        /// The row last clicked: it is where the pointer is, so the list doesn't move to show it.
        private var clickedRow: String?
        /// Keeps the list where it was left, across the window and the app being made anew.
        var keeper: ScrollKeeper?

        init(store: MailStore) {
            self.store = store
        }

        func update(rows: [ThreadRow], selected: String?, checked: Set<String>, ready: Bool) {
            guard let table else { return }
            let moved = selected != self.selected
            let picked = checked != self.checked
            let switched = ready && !self.ready
            let changed = Self.changes(from: self.rows, to: rows)
            self.rows = rows
            self.selected = selected
            self.checked = checked
            self.ready = ready
            if let changed {
                for index in changed {
                    guard let view = table.rowView(atRow: index, makeIfNecessary: false) as? ThreadRowView else { continue }
                    configure(view, at: index)
                }
            } else {
                table.reloadData()
            }
            if switched { keeper?.top() }
            if changed != nil, moved || picked {
                table.enumerateAvailableRowViews { view, _ in
                    guard let view = view as? ThreadRowView, let id = view.row?.id else { return }
                    view.isCurrent = id == selected
                    view.isChecked = checked.contains(id)
                }
            }
            if moved, let selected, selected != clickedRow, store.conversation == nil,
               let index = rows.firstIndex(where: { $0.id == selected }) {
                table.scrollRowToVisible(index)
            }
            if moved { clickedRow = nil }
            if changed != [] { schedulePrefetch() }
        }

        /// The rows that changed when the list holds the same threads in the same order; nil
        /// when it doesn't, and the whole table is read again.
        private static func changes(from old: [ThreadRow], to new: [ThreadRow]) -> IndexSet? {
            guard old != new else { return [] }
            guard old.count == new.count else { return nil }
            var changed = IndexSet()
            for index in new.indices where old[index] != new[index] {
                guard old[index].id == new[index].id else { return nil }
                changed.insert(index)
            }
            return changed
        }

        private func text(for row: ThreadRow) -> RowText {
            if let (kept, text) = texts[row.id], kept == row { return text }
            let text = RowText(row)
            texts[row.id] = (row, text)
            return text
        }

        private func configure(_ view: ThreadRowView, at index: Int) {
            let row = rows[index]
            view.configure(
                row: row, text: text(for: row), current: row.id == selected, checked: checked.contains(row.id),
                hovering: table?.hovered == index, first: index == 0, last: index == rows.count - 1
            )
        }

        func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

        func tableView(_ tableView: NSTableView, rowViewForRow index: Int) -> NSTableRowView? {
            let identifier = NSUserInterfaceItemIdentifier("row")
            let view = tableView.makeView(withIdentifier: identifier, owner: nil) as? ThreadRowView ?? ThreadRowView()
            view.identifier = identifier
            configure(view, at: index)
            if index > rows.count - 30 { DispatchQueue.main.async { self.store.loadMore() } }
            return view
        }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? { nil }

        @objc func scrolled() {
            schedulePrefetch()
        }

        /// Asks for the bodies of the rows on screen once the list stops moving.
        private func schedulePrefetch() {
            prefetch?.cancel()
            let work = DispatchWorkItem { [weak self] in
                MainActor.assumeIsolated {
                    guard let self, let table = self.table else { return }
                    let visible = table.rows(in: table.visibleRect)
                    let range = max(visible.location, 0)..<min(visible.location + visible.length, self.rows.count)
                    guard !range.isEmpty else { return }
                    self.store.prefetch(self.rows[range].map(\.id))
                }
            }
            prefetch = work
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
        }

        @objc func clicked() {
            guard let table, table.clickedRow >= 0, table.clickedRow < rows.count else { return }
            let row = rows[table.clickedRow]
            guard let view = table.rowView(atRow: table.clickedRow, makeIfNecessary: false) as? ThreadRowView,
                  let event = NSApp.currentEvent
            else { return }
            let point = view.convert(event.locationInWindow, from: nil)
            guard view.column.contains(point) else { return }
            if event.modifierFlags.contains(.command) {
                store.toggleSelection(row.id)
                return
            }
            if event.modifierFlags.contains(.shift) {
                store.extendSelection(to: row.id)
                return
            }
            let threads = store.targets(for: row.id)
            switch view.hit(point) {
            case .star: store.run(.star, on: threads)
            case .archive: store.run(.archive, on: threads)
            case .trash: store.run(.trash, on: threads)
            case .snooze: store.run(.snooze, on: threads)
            case .read: store.run(.read, on: threads)
            case .none:
                clickedRow = row.id
                store.clearSelection()
                store.open(row.id)
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
        let point = convert(event.locationInWindow, from: nil)
        let index = row(at: point)
        guard index >= 0, let view = rowView(atRow: index, makeIfNecessary: false) as? ThreadRowView,
              view.column.contains(convert(point, to: view))
        else {
            hovered = -1
            return
        }
        hovered = index
        view.hoveredHit = view.hit(convert(point, to: view))
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
    var isChecked = false { didSet { if isChecked != oldValue { needsDisplay = true } } }
    var hovering = false {
        didSet {
            guard hovering != oldValue else { return }
            if !hovering { hoveredHit = .none }
            needsDisplay = true
        }
    }
    /// The action or the star the pointer is over, lit as a button would be.
    var hoveredHit = Hit.none { didSet { if hoveredHit != oldValue { needsDisplay = true } } }
    private var isFirst = false
    private var isLast = false

    private static let sendersX: CGFloat = 36
    private static let sendersWidth: CGFloat = 190
    private static let iconSide: CGFloat = 16

    func configure(row: ThreadRow, text: RowText, current: Bool, checked: Bool, hovering: Bool, first: Bool, last: Bool) {
        self.row = row
        self.text = text
        self.isCurrent = current
        self.isChecked = checked
        self.hovering = hovering
        self.isFirst = first
        self.isLast = last
        needsDisplay = true
    }

    override var isFlipped: Bool { true }

    /// Where the row is drawn: the column's width, centred, with the page's margin either side.
    var column: NSRect {
        let width = max(min(bounds.width - 48, Theme.cardWidth), 0)
        return NSRect(x: ((bounds.width - width) / 2).rounded(), y: 0, width: width, height: bounds.height)
    }

    private var starRect: NSRect {
        NSRect(x: column.maxX - 40, y: (bounds.height - Self.iconSide) / 2, width: Self.iconSide, height: Self.iconSide)
    }

    private var actions: [(Hit, Symbol, NSRect)] {
        guard let row else { return [] }
        let kinds: [(Hit, Symbol)] = row.draftId != nil
            ? [(.trash, .trash)]
            : [(.read, row.unread ? .mailOpen : .mail), (.snooze, .clock), (.trash, .trash), (.archive, .archive)]
        return kinds.enumerated().map { index, kind in
            let x = starRect.minX - 34 - CGFloat(index) * 30
            return (kind.0, kind.1, NSRect(x: x, y: (bounds.height - Self.iconSide) / 2, width: Self.iconSide, height: Self.iconSide))
        }
    }

    func hit(_ point: NSPoint) -> Hit {
        if row?.draftId == nil, starRect.insetBy(dx: -8, dy: -12).contains(point) { return .star }
        guard hovering else { return .none }
        return actions.first(where: { $0.2.insetBy(dx: -7, dy: -12).contains(point) })?.0 ?? .none
    }

    override func drawBackground(in dirtyRect: NSRect) {
        let column = column
        if isCurrent || hovering {
            Tokens.accent.platform.setFill()
            column.fill()
        }
        if isChecked {
            Tokens.primary.opacity(Tokens.colorTintOpacity).platform.setFill()
            column.fill()
        }
        // The frame around the list, and the line above every row but the first: the first
        // row's top line is the frame's.
        Tokens.border.platform.setFill()
        NSRect(x: column.minX, y: 0, width: 1, height: column.height).fill()
        NSRect(x: column.maxX - 1, y: 0, width: 1, height: column.height).fill()
        NSRect(x: column.minX, y: 0, width: column.width, height: 1).fill()
        if isLast {
            NSRect(x: column.minX, y: column.maxY - 1, width: column.width, height: 1).fill()
        }
        if let row {
            Theme.accountColor(row.color).platform.setFill()
            let bottom: CGFloat = isLast ? 1 : 0
            NSRect(x: column.minX, y: 1, width: Theme.accountBarWidth, height: column.height - 1 - bottom).fill()
        }
    }

    override func drawSelection(in dirtyRect: NSRect) {}

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard let row, let text else { return }
        let left = column.minX
        let middle = bounds.height / 2
        if isChecked {
            Self.drawSymbol(.squareCheck, in: NSRect(x: left + 13, y: middle - 8, width: 16, height: 16), color: Tokens.primary.platform)
        } else if row.unread {
            Tokens.primary.platform.setFill()
            NSBezierPath(ovalIn: NSRect(x: left + 18, y: middle - 3.5, width: 7, height: 7)).fill()
        }
        Self.drawLine(text.senders, x: left + Self.sendersX, width: Self.sendersWidth, middle: middle)
        var x = left + Self.sendersX + Self.sendersWidth + 14
        if row.attachment {
            Self.drawSymbol(.paperclip, in: NSRect(x: x, y: middle - 7, width: 14, height: 14), color: Tokens.mutedMoreForeground.platform)
        }
        x += 24
        let trailing: CGFloat
        if hovering {
            trailing = (actions.last?.2.minX ?? starRect.minX) - 16
            for (kind, symbol, rect) in actions {
                let lit = hoveredHit == kind
                if lit { Self.drawHover(around: rect, color: Tokens.accentStronger.platform) }
                Self.drawSymbol(symbol, in: rect, color: lit ? Tokens.foreground.platform : Tokens.mutedForeground.platform)
            }
        } else {
            let dateWidth = ceil(text.date.size().width)
            Self.drawLine(text.date, x: starRect.minX - 18 - dateWidth, width: dateWidth + 2, middle: middle)
            trailing = starRect.minX - 34 - dateWidth
        }
        Self.drawLine(text.line, x: x, width: max(trailing - x, 0), middle: middle)
        guard row.draftId == nil else { return }
        let starLit = hoveredHit == .star
        if starLit { Self.drawHover(around: starRect, color: Tokens.star.opacity(Tokens.colorTintOpacity).platform) }
        let starColor = row.starred || starLit ? Tokens.star.platform : Tokens.input.platform
        Self.drawSymbol(row.starred ? .starFilled : .star, in: starRect, color: starColor)
    }

    /// The circle a hovered icon sits on, as tall as the smallest control.
    private static func drawHover(around rect: NSRect, color: NSColor) {
        let inset = (rect.width - ControlSize.small.height) / 2
        color.setFill()
        NSBezierPath(ovalIn: rect.insetBy(dx: inset, dy: inset)).fill()
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
