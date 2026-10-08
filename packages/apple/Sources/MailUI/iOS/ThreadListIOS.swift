#if os(iOS)
import SwiftUI
import UIKit

/// The thread list on iOS: a table that only makes the rows on screen. A swipe to the left
/// deletes, snoozes or archives; one to the right opens the drawer. In edit mode, rows are picked.
struct ThreadListIOS: UIViewRepresentable {
    let store: MailStore
    let rows: [ThreadRow]
    let editing: Bool
    let checked: Set<String>
    /// False while the rows are the mailbox before; when they are the new one's, the list goes to the top.
    let ready: Bool
    let open: (String) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(store: store, open: open) }

    func makeUIView(context: Context) -> UITableView {
        let table = UITableView(frame: .zero, style: .plain)
        table.register(ThreadCell.self, forCellReuseIdentifier: ThreadCell.identifier)
        table.rowHeight = Theme.iosRowHeight
        table.separatorStyle = .none
        table.backgroundColor = Tokens.background.platform
        let bottom = UIView(frame: CGRect(x: 0, y: 0, width: 0, height: Theme.hairline))
        bottom.backgroundColor = Tokens.border.platform
        table.tableFooterView = bottom
        table.allowsMultipleSelectionDuringEditing = true
        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        context.coordinator.table = table
        context.coordinator.keeper = ScrollKeeper(restore: store.listOffset, report: store.noteScroll)
        context.coordinator.keeper?.attach(table)
        return table
    }

    func updateUIView(_ table: UITableView, context: Context) {
        context.coordinator.open = open
        context.coordinator.update(rows: rows, editing: editing, checked: checked, ready: ready)
    }

    @MainActor
    final class Coordinator: NSObject, UITableViewDataSource, UITableViewDelegate {
        let store: MailStore
        var open: (String) -> Void
        weak var table: UITableView?
        private var rows: [ThreadRow] = []
        private var texts: [String: (ThreadRow, RowText)] = [:]
        private var ready = true
        private var prefetch: DispatchWorkItem?
        /// Keeps the list where it was left, across the app being made anew.
        var keeper: ScrollKeeper?

        init(store: MailStore, open: @escaping (String) -> Void) {
            self.store = store
            self.open = open
        }

        func update(rows: [ThreadRow], editing: Bool, checked: Set<String>, ready: Bool) {
            guard let table else { return }
            let switched = ready && !self.ready
            let changed = Self.changes(from: self.rows, to: rows)
            self.rows = rows
            self.ready = ready
            if let changed {
                let visible = Set(table.indexPathsForVisibleRows ?? [])
                let paths = changed.map { IndexPath(row: $0, section: 0) }.filter(visible.contains)
                if !paths.isEmpty { table.reconfigureRows(at: paths) }
            } else {
                table.reloadData()
            }
            if switched { keeper?.top() }
            if table.isEditing != editing { table.setEditing(editing, animated: true) }
            if editing { pick(checked) }
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

        /// Shows the store's selection as the table's.
        private func pick(_ checked: Set<String>) {
            guard let table else { return }
            let shown = Set((table.indexPathsForSelectedRows ?? []).compactMap { rows.indices.contains($0.row) ? rows[$0.row].id : nil })
            guard shown != checked else { return }
            for (index, row) in rows.enumerated() where checked.contains(row.id) != shown.contains(row.id) {
                let path = IndexPath(row: index, section: 0)
                if checked.contains(row.id) {
                    table.selectRow(at: path, animated: false, scrollPosition: .none)
                } else {
                    table.deselectRow(at: path, animated: false)
                }
            }
        }

        private func text(for row: ThreadRow) -> RowText {
            if let (kept, text) = texts[row.id], kept == row { return text }
            let text = RowText(row)
            texts[row.id] = (row, text)
            return text
        }

        func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { rows.count }

        func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
            let cell = tableView.dequeueReusableCell(withIdentifier: ThreadCell.identifier, for: indexPath)
            let row = rows[indexPath.row]
            (cell as? ThreadCell)?.configure(row: row, text: text(for: row), first: indexPath.row == 0)
            return cell
        }

        func tableView(_ tableView: UITableView, willDisplay cell: UITableViewCell, forRowAt indexPath: IndexPath) {
            if indexPath.row > rows.count - 30 { store.loadMore() }
        }

        func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
            let id = rows[indexPath.row].id
            guard !tableView.isEditing else {
                store.selection.insert(id)
                return
            }
            tableView.deselectRow(at: indexPath, animated: true)
            open(id)
        }

        func tableView(_ tableView: UITableView, didDeselectRowAt indexPath: IndexPath) {
            guard tableView.isEditing, rows.indices.contains(indexPath.row) else { return }
            store.selection.remove(rows[indexPath.row].id)
        }

        func scrollViewDidScroll(_ scrollView: UIScrollView) {
            schedulePrefetch()
        }

        /// Asks for the bodies of the rows on screen once the list stops moving.
        private func schedulePrefetch() {
            prefetch?.cancel()
            let work = DispatchWorkItem { [weak self] in
                MainActor.assumeIsolated {
                    guard let self, let table = self.table else { return }
                    let ids = (table.indexPathsForVisibleRows ?? []).compactMap { self.rows.indices.contains($0.row) ? self.rows[$0.row].id : nil }
                    self.store.prefetch(ids)
                }
            }
            prefetch = work
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
        }

        private func swipe(_ title: String, _ symbol: Symbol, _ color: ThemeColor, _ perform: @escaping @MainActor () -> Void) -> UIContextualAction {
            let action = UIContextualAction(style: .normal, title: title) { _, _, done in
                MainActor.assumeIsolated { perform() }
                done(true)
            }
            action.image = UIImage.symbol(symbol, side: 22).withTintColor(.white, renderingMode: .alwaysOriginal)
            action.backgroundColor = color.platform
            return action
        }

        func tableView(_ tableView: UITableView, trailingSwipeActionsConfigurationForRowAt indexPath: IndexPath) -> UISwipeActionsConfiguration? {
            let row = rows[indexPath.row]
            let store = store
            let delete = swipe("Delete", .trash, Tokens.destructive) { store.run(.trash, on: [row.id]) }
            guard row.draftId == nil else { return UISwipeActionsConfiguration(actions: [delete]) }
            return UISwipeActionsConfiguration(actions: [
                delete,
                swipe(store.remindsInsteadOfSnoozing ? "Remind" : "Snooze", .clock, Tokens.warning) { store.run(.snooze, on: [row.id]) },
                swipe("Archive", .archive, Tokens.success) { store.run(.archive, on: [row.id]) },
            ])
        }
    }
}

/// One thread on three lines: who wrote and the star, the subject, the snippet and the date.
final class ThreadCell: UITableViewCell {
    static let identifier = "thread"
    private let canvas = RowCanvas()

    override init(style: UITableViewCell.CellStyle, reuseIdentifier: String?) {
        super.init(style: style, reuseIdentifier: reuseIdentifier)
        canvas.isOpaque = false
        backgroundColor = .clear
        canvas.frame = contentView.bounds
        canvas.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        contentView.addSubview(canvas)
        let pressed = UIView()
        pressed.backgroundColor = Tokens.accent.platform
        selectedBackgroundView = pressed
    }

    required init?(coder: NSCoder) { nil }

    func configure(row: ThreadRow, text: RowText, first: Bool) {
        canvas.row = row
        canvas.text = text
        canvas.first = first
        canvas.setNeedsDisplay()
        accessibilityLabel = "\(row.unread ? "Unread, " : "")\(row.senders), \(row.subject), \(row.date)"
    }
}

private final class RowCanvas: UIView {
    var row: ThreadRow?
    var text: RowText?
    var first = false

    override func draw(_ rect: CGRect) {
        guard let row, let text else { return }
        if !first {
            Tokens.border.platform.setFill()
            UIRectFill(CGRect(x: 0, y: 0, width: bounds.width, height: Theme.hairline))
        }
        Theme.accountColor(row.color).platform.setFill()
        UIRectFill(CGRect(x: 0, y: 0, width: Theme.accountBarWidth, height: bounds.height))
        let left: CGFloat = 16
        let right = bounds.width - 16
        let star = CGRect(x: right - 18, y: 14, width: 18, height: 18)
        var x = left
        if row.unread {
            Tokens.primary.platform.setFill()
            UIBezierPath(ovalIn: CGRect(x: left, y: 20, width: 8, height: 8)).fill()
            x += 15
        }
        draw(text.senders, x: x, y: 12, width: star.minX - 10 - x)
        let starColor = row.starred ? Tokens.star.platform : Tokens.input.platform
        UIImage.symbol(row.starred ? .starFilled : .star, side: star.width).withTintColor(starColor, renderingMode: .alwaysOriginal).draw(in: star)
        var subjectWidth = right - left
        if row.attachment {
            let clip = CGRect(x: right - 15, y: 38, width: 15, height: 15)
            UIImage.symbol(.paperclip, side: 15).withTintColor(Tokens.mutedMoreForeground.platform, renderingMode: .alwaysOriginal).draw(in: clip)
            subjectWidth -= 22
        }
        draw(text.subject, x: left, y: 36, width: subjectWidth)
        let dateWidth = ceil(text.date.size().width)
        draw(text.date, x: right - dateWidth, y: 60, width: dateWidth + 1)
        draw(text.snippet, x: left, y: 59, width: right - dateWidth - 10 - left)
    }

    private func draw(_ text: NSAttributedString, x: CGFloat, y: CGFloat, width: CGFloat) {
        guard width > 8 else { return }
        text.draw(with: CGRect(x: x, y: y, width: width, height: 24), options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine], context: nil)
    }
}
#endif
