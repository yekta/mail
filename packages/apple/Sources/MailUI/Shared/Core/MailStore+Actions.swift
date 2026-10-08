import Foundation

/// Threads being labelled, or moved to a label.
struct Labeling: Identifiable {
    let id = UUID()
    let threads: [String]
    let move: Bool
}

/// A question asked before something that can't be taken back.
struct Confirmation: Identifiable {
    let id = UUID()
    let title: String
    let message: String
    let action: String
    let perform: () -> Void
}

/// What the command palette lists.
enum PaletteScope {
    case everything, labels
}

extension MailStore {
    // MARK: Selection

    func toggleSelection(_ thread: String) {
        if selection.contains(thread) {
            selection.remove(thread)
        } else {
            selection.insert(thread)
        }
        selectionAnchor = thread
        selected = thread
    }

    /// Selects the rows from the anchor to this one, as a Shift-click does.
    func extendSelection(to thread: String) {
        let list = visibleRows
        guard let anchor = selectionAnchor ?? selected,
              let from = list.firstIndex(where: { $0.id == anchor }),
              let to = list.firstIndex(where: { $0.id == thread })
        else {
            toggleSelection(thread)
            return
        }
        selection.formUnion(list[min(from, to)...max(from, to)].map(\.id))
        selected = thread
    }

    /// Selects the current row and the one `step` away, as Shift+J/K do.
    func extendSelection(_ step: Int) {
        guard conversation == nil else { return }
        if let selected {
            selection.insert(selected)
            if selectionAnchor == nil { selectionAnchor = selected }
        }
        move(step)
        if let selected { selection.insert(selected) }
    }

    func selectAll() {
        guard conversation == nil else { return }
        selection = Set(visibleRows.map(\.id))
    }

    func clearSelection() {
        selection = []
        selectionAnchor = nil
    }

    // MARK: Commands

    /// Does a command to the threads given, or to the selection or the current thread.
    func run(_ command: ThreadCommand, on given: [String]? = nil) {
        let chosen = given ?? targets
        if command == .trash { deleteDrafts(chosen.filter(Self.isDraft)) }
        let threads = chosen.filter { !Self.isDraft($0) }
        guard let first = threads.first else { return }
        switch command {
        case .archive: act(.archive, on: threads)
        case .inbox: act(.inbox, on: threads)
        case .trash: act(.trash, on: threads)
        case .spam: act(.spam, on: threads)
        case .star: toggleStar(threads)
        case .read: toggleRead(threads)
        case .markRead: act(.read, on: threads)
        case .snooze: snoozing = threads
        case .label: labeling = Labeling(threads: threads, move: false)
        case .move: labeling = Labeling(threads: threads, move: true)
        case .mute: act(isMuted(threads) ? .unmute : .mute, on: threads)
        case .unsubscribe: act(.unsubscribe, on: threads)
        case .block: confirmBlock(threads)
        case .print: printThread(first)
        case .reply: reply(.reply, to: first)
        case .replyAll: reply(.replyAll, to: first)
        case .forward: reply(.forward, to: first)
        }
    }

    /// A saved draft's row, not a thread's: only deleting it applies.
    static func isDraft(_ thread: String) -> Bool {
        thread.hasPrefix("draft:")
    }

    /// The commands that apply to these threads: deleting alone when they are all drafts.
    func applicable(_ commands: [ThreadCommand], to threads: [String]) -> [ThreadCommand] {
        guard !threads.isEmpty, threads.allSatisfy(Self.isDraft) else { return commands }
        return commands.filter { $0 == .trash }
    }

    private func deleteDrafts(_ rows: [String]) {
        guard !rows.isEmpty else { return }
        let leaving = Set(rows)
        let ids = visibleRows.filter { leaving.contains($0.id) }.compactMap(\.draftId)
        selection.subtract(rows)
        if let selected, leaving.contains(selected) { self.selected = nil }
        Task {
            do {
                for id in ids {
                    _ = try await bridge.call("delete_draft", ["draft_id": id], as: Empty.self)
                }
                show(ids.count == 1 ? "Draft deleted." : "\(ids.count) drafts deleted.")
            } catch {
                show(error.localizedDescription)
            }
        }
    }

    func isMuted(_ threads: [String]) -> Bool {
        guard let conversation, threads == [conversation.id] else { return false }
        return conversation.muted
    }

    /// Whether the list is of mail the user sent, where a snooze is a reminder.
    var remindsInsteadOfSnoozing: Bool {
        baseMailbox.split(separator: "/").last == "sent"
    }

    private func confirmBlock(_ threads: [String]) {
        let senders = threads.count == 1 ? row(threads[0])?.senders ?? conversation?.participants : nil
        let who = senders.map { "Block \($0)?" } ?? "Block these senders?"
        confirmation = Confirmation(
            title: who, message: "New mail from them goes straight to the trash, and so does this.", action: "Block"
        ) { [weak self] in self?.act(.block, on: threads) }
    }

    /// Archives everything in the mailbox on screen, after asking: Get Me To Zero.
    func archiveAll() {
        guard searchRows == nil else { return }
        var fields: [String: Any] = ["mailbox": mailbox]
        if let filter { fields["filter"] = filter.rawValue }
        let what = switch filter {
        case .unread: "every unread thread"
        case .starred: "every starred thread"
        case nil: "everything"
        }
        confirmation = Confirmation(
            title: "Archive \(what) in \(shownName)?", message: "Every thread shown here goes to the archive.", action: "Archive all"
        ) { [weak self] in
            guard let self else { return }
            clearSelection()
            Task {
                do {
                    self.report(try await self.bridge.call("archive_all", fields, as: ActReply.self))
                } catch {
                    self.show(error.localizedDescription)
                }
            }
        }
    }

    /// The list's name as shown: a split's own name ("Important"), else the mailbox's.
    var shownName: String {
        guard !splits.isEmpty else { return mailboxName }
        return splits.first(where: { $0.mailbox == mailbox })?.name ?? splits[0].name
    }

    func printThread(_ thread: String) {
        Task {
            do {
                Printer.print(html: try await bridge.call("print_thread", ["thread": thread], as: HTMLReply.self).html)
            } catch {
                show(error.localizedDescription)
            }
        }
    }

    func showPalette(_ scope: PaletteScope = .everything) {
        guard !sheetOpen else { return }
        palette = scope
    }

    // MARK: Labels

    func account(of thread: String) -> String? {
        if let conversation, conversation.id == thread { return conversation.accountId }
        return row(thread)?.accountId
    }

    /// An account's labels, from its mailboxes.
    func labels(of account: String) -> [Mailbox] {
        accounts.first(where: { $0.id == account })?.mailboxes.filter { Self.labelID($0.id) != nil } ?? []
    }

    /// The label of a label's mailbox, `<account>/label/<label>`.
    static func labelID(_ mailbox: String) -> String? {
        guard let range = mailbox.range(of: "/label/") else { return nil }
        return String(mailbox[range.upperBound...])
    }

    /// Adds an account's label to its threads, takes it off, or moves them to it.
    func apply(label: String, account: String, to labeling: Labeling, remove: Bool = false) {
        self.labeling = nil
        let threads = threads(labeling.threads, in: account)
        act(labeling.move ? .move : remove ? .removeLabel : .addLabel, on: threads, label: label)
    }

    private func threads(_ threads: [String], in account: String) -> [String] {
        let wanted = Set(threads)
        var found = Set(visibleRows.lazy.filter { wanted.contains($0.id) && $0.accountId == account }.map(\.id))
        if let conversation, conversation.accountId == account { found.insert(conversation.id) }
        return threads.filter(found.contains)
    }

    func createLabel(_ name: String, account: String, for labeling: Labeling) {
        self.labeling = nil
        Task {
            do {
                let made = try await bridge.call("create_label", ["account": account, "name": name], as: IDReply.self)
                apply(label: made.id, account: account, to: labeling)
            } catch {
                show(error.localizedDescription)
            }
        }
    }

    /// The times a snooze can be until: the usual ones, or what `text` says.
    func parseTime(_ text: String) async -> [TimeChoice] {
        (try? await bridge.call("parse_time", ["text": text], as: TimeChoices.self).choices) ?? []
    }
}
