#if os(iOS)
import SwiftUI
import UIKit

/// The thread list on iOS: a table that only makes the rows on screen, with Newton's swipes —
/// right to archive or mark read, left to snooze or delete.
struct ThreadListIOS: UIViewRepresentable {
    let store: MailStore
    let rows: [ThreadRow]
    let open: (String) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(store: store, open: open) }

    func makeUIView(context: Context) -> UITableView {
        let table = UITableView(frame: .zero, style: .plain)
        table.register(ThreadCell.self, forCellReuseIdentifier: ThreadCell.identifier)
        table.rowHeight = Theme.iosRowHeight
        table.separatorInset = UIEdgeInsets(top: 0, left: 16, bottom: 0, right: 0)
        table.separatorColor = Tokens.border.platform
        table.backgroundColor = Tokens.card.platform
        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        context.coordinator.table = table
        return table
    }

    func updateUIView(_ table: UITableView, context: Context) {
        context.coordinator.open = open
        context.coordinator.update(rows: rows)
    }

    final class Coordinator: NSObject, UITableViewDataSource, UITableViewDelegate {
        let store: MailStore
        var open: (String) -> Void
        weak var table: UITableView?
        private var rows: [ThreadRow] = []
        private var texts: [String: (ThreadRow, RowText)] = [:]

        init(store: MailStore, open: @escaping (String) -> Void) {
            self.store = store
            self.open = open
        }

        func update(rows: [ThreadRow]) {
            guard rows != self.rows else { return }
            self.rows = rows
            table?.reloadData()
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
            (cell as? ThreadCell)?.configure(row: row, text: text(for: row))
            return cell
        }

        func tableView(_ tableView: UITableView, willDisplay cell: UITableViewCell, forRowAt indexPath: IndexPath) {
            if indexPath.row > rows.count - 30 { MainActor.assumeIsolated { store.loadMore() } }
        }

        func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
            tableView.deselectRow(at: indexPath, animated: true)
            open(rows[indexPath.row].id)
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

        func tableView(_ tableView: UITableView, leadingSwipeActionsConfigurationForRowAt indexPath: IndexPath) -> UISwipeActionsConfiguration? {
            let row = rows[indexPath.row]
            let store = store
            return UISwipeActionsConfiguration(actions: [
                swipe("Archive", .archive, Tokens.chart4) { store.act(.archive, on: [row.id]) },
                swipe(row.unread ? "Read" : "Unread", row.unread ? .mailOpen : .mail, Tokens.primary) { store.toggleRead(row.id) },
            ])
        }

        func tableView(_ tableView: UITableView, trailingSwipeActionsConfigurationForRowAt indexPath: IndexPath) -> UISwipeActionsConfiguration? {
            let row = rows[indexPath.row]
            let store = store
            return UISwipeActionsConfiguration(actions: [
                swipe("Delete", .trash, Tokens.destructive) { store.act(.trash, on: [row.id]) },
                swipe("Snooze", .clock, Tokens.chart3) { store.snoozing = [row.id] },
            ])
        }
    }
}

/// One thread on three lines: who wrote and the star, the subject, the snippet and the date.
/// Read threads sit on grey, as Newton showed them.
final class ThreadCell: UITableViewCell {
    static let identifier = "thread"
    private let canvas = RowCanvas()

    override init(style: UITableViewCell.CellStyle, reuseIdentifier: String?) {
        super.init(style: style, reuseIdentifier: reuseIdentifier)
        canvas.isOpaque = false
        canvas.frame = contentView.bounds
        canvas.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        contentView.addSubview(canvas)
        let pressed = UIView()
        pressed.backgroundColor = Tokens.accent.platform
        selectedBackgroundView = pressed
    }

    required init?(coder: NSCoder) { nil }

    func configure(row: ThreadRow, text: RowText) {
        canvas.row = row
        canvas.text = text
        backgroundColor = row.unread ? Tokens.card.platform : Tokens.muted.platform
        canvas.setNeedsDisplay()
        accessibilityLabel = "\(row.unread ? "Unread, " : "")\(row.senders), \(row.subject), \(row.date)"
    }
}

private final class RowCanvas: UIView {
    var row: ThreadRow?
    var text: RowText?

    override func draw(_ rect: CGRect) {
        guard let row, let text else { return }
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
            UIImage.symbol(.paperclip, side: 15).withTintColor(Tokens.mutedForeground.platform, renderingMode: .alwaysOriginal).draw(in: clip)
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
