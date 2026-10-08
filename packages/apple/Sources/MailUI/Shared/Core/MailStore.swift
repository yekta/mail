import AuthenticationServices
import Foundation
import Observation
import SwiftUI

#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// A note at the bottom of the window, sometimes with something to do about it.
struct Toast: Identifiable, Equatable {
    let id = UUID()
    let message: String
    var action: String?
    var duration: TimeInterval = 4

    static func == (lhs: Toast, rhs: Toast) -> Bool { lhs.id == rhs.id }
}

/// A message being written.
struct Compose: Identifiable, Codable, Equatable {
    /// What is typed in the address fields and not yet an address.
    struct Typing: Codable, Hashable {
        var to = ""
        var cc = ""
        var bcc = ""
    }

    let id = UUID()
    var draft: Draft
    var from: String
    var showCc: Bool
    /// The saved draft it is kept as, once it is.
    var draftId: String?
    /// The thread it answers, to archive along with sending.
    var thread: String?
    /// When to bring the thread back if nobody answers.
    var remindAt: TimeChoice?
    /// Closing keeps it as a draft even unchanged: a message brought back from sending.
    var keep = false
    var typing: Typing?

    enum CodingKeys: String, CodingKey {
        case draft, from, showCc, draftId, thread, remindAt, keep, typing
    }

    /// Whether anything was written in it.
    var hasContent: Bool {
        !draft.to.isEmpty || !draft.cc.isEmpty || !draft.bcc.isEmpty || !draft.subject.isEmpty || !draft.text.isEmpty
            || !draft.attachments.isEmpty
    }
}

@Observable @MainActor
public final class MailStore {
    let bridge = CoreBridge()
    #if os(macOS)
    let updater = AppUpdater()
    #endif

    var started = false
    /// The core answered where the app was left: until then the window shows nothing.
    var booted = false
    var signedIn = false
    var connection = "offline"
    var server = ""
    var unified: [Mailbox] = []
    var accounts: [AccountView] = []
    var mailbox = "inbox"
    var rows: [ThreadRow] = []
    var total = 0
    /// The thread on screen, when one is open.
    var conversation: Conversation?
    /// The row the keyboard is on (Mac).
    var selected: String?
    var searchQuery = ""
    /// The matches of the search, while searching.
    var searchRows: [ThreadRow]?
    var toast: Toast?
    var compose: Compose?
    /// The threads to snooze, while the snooze choices are shown.
    var snoozing: [String]?
    /// The tabs over a split inbox; empty without Split Inbox.
    var splits: [SplitTab] = []
    /// What the list is narrowed to.
    var filter: Filter?
    /// The threads picked to act on together.
    var selection: Set<String> = []
    /// Where a Shift-click or Shift+J/K selection grows from.
    var selectionAnchor: String?
    /// The threads being labelled or moved, while the labels are shown.
    var labeling: Labeling?
    /// What the command palette shows, while it is open.
    var palette: PaletteScope?
    var shortcutsOpen = false
    var settingsOpen = false
    /// A question to answer before something that can't be taken back.
    var confirmation: Confirmation?
    /// A thread a notification asked to show, for iOS to push.
    var requestedThread: String?
    /// Threads whose remote images were allowed.
    var imagesShown: Set<String> = []
    /// The mailboxes over the page (Mac).
    var sidebarOpen = false
    /// How far the list and the open thread are scrolled, as their views report it; a view made
    /// anew goes back to it.
    @ObservationIgnored var listOffset: CGFloat = 0
    @ObservationIgnored var threadOffset: CGFloat = 0
    var undoDelay: Int = UserDefaults.standard.object(forKey: "undoDelay") as? Int ?? 10 {
        didSet { UserDefaults.standard.set(undoDelay, forKey: "undoDelay") }
    }
    var appearance = Appearance(rawValue: UserDefaults.standard.string(forKey: "appearance") ?? "") ?? .system {
        didSet { UserDefaults.standard.set(appearance.rawValue, forKey: "appearance") }
    }

    // Compose, the thread page and preferences.
    /// The synced preferences, by key.
    var preferences: [String: JSONValue] = [:]
    var preferencesLoaded = false
    /// Attachments being downloaded, as `<message>/<index>`.
    var downloading: Set<String> = []
    /// Messages of the open thread unfolded by a click or the keyboard.
    var unfoldedMessages: Set<String> = []
    /// The message of the open thread that n and p are on.
    var focusedMessage: String?
    /// Each compose sheet's last save, so the next save and the send wait for it.
    @ObservationIgnored private var draftSaves: [UUID: Task<String?, Never>] = [:]
    @ObservationIgnored private var uiSave: Task<Void, Never>?

    @ObservationIgnored private var toastAction: (() -> Void)?
    @ObservationIgnored private var searchRequest: UInt64 = 0
    @ObservationIgnored private var reloading = false
    /// A change came while the list was being read: read it again after.
    @ObservationIgnored private var staleWhileReloading = false
    @ObservationIgnored private var signIn: ASWebAuthenticationSession?
    @ObservationIgnored private let anchor = SignInAnchor()
    /// The threads whose bodies were asked for ahead of time.
    @ObservationIgnored var prefetched: Set<String> = []

    public init() {}

    /// The rows the list shows: the search's while searching.
    var visibleRows: [ThreadRow] { searchRows ?? rows }

    /// The mailbox without its split: `inbox` for `inbox:other`.
    var baseMailbox: String {
        let parts = mailbox.split(separator: "/", omittingEmptySubsequences: false)
        guard let last = parts.last, last.hasPrefix("inbox:") else { return mailbox }
        return (parts.dropLast() + ["inbox"]).joined(separator: "/")
    }

    var mailboxName: String {
        if searchRows != nil { return "Search" }
        let parts = baseMailbox.split(separator: "/").map(String.init)
        let all = unified + accounts.flatMap(\.mailboxes)
        let name = all.first(where: { $0.id == baseMailbox })?.name ?? "Inbox"
        guard parts.count > 1, let account = accounts.first(where: { $0.id == parts[0] }) else {
            return name == "Inbox" && accounts.count > 1 ? "All Inboxes" : name
        }
        return "\(name) · \(account.address)"
    }

    /// Whether a sheet, the palette or a question is up, when the window's keys don't apply.
    var sheetOpen: Bool {
        compose != nil || snoozing != nil || labeling != nil || palette != nil || shortcutsOpen || settingsOpen || confirmation != nil
    }

    // MARK: Starting

    public func start(defaultServer: String) {
        guard !started else { return }
        started = true
        bridge.onEvent = { [weak self] event in self?.handle(event) }
        Notifier.shared.open = { [weak self] thread in self?.show(thread: thread) }
        let demo = ProcessInfo.processInfo.arguments.contains("--demo")
        let folder = demo ? Platform.dataFolder.appendingPathComponent("Demo", isDirectory: true) : Platform.dataFolder
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let config: [String: Any] = ["data_dir": folder.path, "server_url": defaultServer, "demo": demo]
        guard bridge.start(config: config) else {
            booted = true
            show("The mail store couldn't open.")
            return
        }
        Outgoing.prune()
        #if os(macOS)
        if !demo { updater.start() }
        #endif
        saveWhenLeaving()
        Task { await boot() }
    }

    /// Shows the screen the app was left on, all of it at once.
    private func boot() async {
        defer {
            booted = true
            trackUi()
        }
        guard let boot = try? await bridge.call("boot", as: Boot.self) else { return }
        let ui = boot.ui
        signedIn = boot.status.signedIn
        connection = boot.status.connection
        server = boot.status.server ?? ""
        unified = boot.mailboxes.unified
        accounts = boot.mailboxes.accounts
        preferences = Dictionary(boot.preferences.values.map { ($0.key, $0.value) }, uniquingKeysWith: { _, last in last })
        preferencesLoaded = true
        mailbox = ui.mailbox
        filter = ui.filter
        rows = boot.page.rows
        total = boot.page.total
        splits = boot.page.splits
        selected = ui.selected
        selection = Set(ui.selection)
        searchQuery = ui.search
        searchRows = boot.search
        conversation = boot.thread
        unfoldedMessages = Set(ui.unfolded)
        focusedMessage = ui.focused
        if ui.images, let open = boot.thread?.id { imagesShown.insert(open) }
        listOffset = CGFloat(ui.listOffset)
        threadOffset = CGFloat(ui.threadOffset)
        sidebarOpen = ui.sidebar
        if var restored = ui.compose {
            restored.keep = restored.keep || restored.hasContent
            compose = restored
        }
        Notifier.shared.setBadge(unified.first(where: { $0.id == "inbox" })?.unread ?? 0)
        if !accounts.isEmpty { Notifier.shared.askPermission() }
        guard !searchQuery.isEmpty else { return }
        search(searchQuery)
    }

    // MARK: Where the app is

    /// Where the app is, as the core keeps it between starts.
    private var uiState: UiState {
        UiState(
            mailbox: mailbox, filter: filter, thread: conversation?.id, selected: selected, rows: rows.count,
            listOffset: Double(listOffset), threadOffset: Double(threadOffset), selection: Array(selection),
            search: searchQuery, compose: compose, sidebar: sidebarOpen, unfolded: Array(unfoldedMessages),
            focused: focusedMessage, images: conversation.map { imagesShown.contains($0.id) } ?? false
        )
    }

    /// Saves a little after the state changes, however much changes in the meantime.
    private func trackUi() {
        withObservationTracking {
            _ = uiState
        } onChange: {
            Task { @MainActor in
                self.scheduleUiSave()
                self.trackUi()
            }
        }
    }

    private func scheduleUiSave() {
        guard uiSave == nil else { return }
        uiSave = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            self?.saveUi()
        }
    }

    func saveUi() {
        guard booted else { return }
        uiSave?.cancel()
        uiSave = nil
        bridge.send("save_ui", ["ui": CoreBridge.object(uiState)])
    }

    /// Saves and waits for it to be written, before the app quits.
    func saveUiNow() async {
        guard booted else { return }
        uiSave?.cancel()
        uiSave = nil
        _ = try? await bridge.call("save_ui", ["ui": CoreBridge.object(uiState)], as: Empty.self)
    }

    /// The compose view reports what it holds as it is written.
    func noteCompose(_ live: Compose) {
        guard compose?.id == live.id, compose != live else { return }
        compose = live
    }

    /// The list reports where it is scrolled to.
    func noteScroll(_ offset: CGFloat) {
        guard offset != listOffset else { return }
        listOffset = offset
        scheduleUiSave()
    }

    /// The open thread reports where it is scrolled to.
    func noteThreadScroll(_ offset: CGFloat) {
        guard offset != threadOffset else { return }
        threadOffset = offset
        scheduleUiSave()
    }

    /// Saves at once as the app goes to the background or quits.
    private func saveWhenLeaving() {
        #if os(macOS)
        let leaving = [NSApplication.didResignActiveNotification, NSApplication.willTerminateNotification]
        #else
        let leaving = [UIApplication.willResignActiveNotification, UIApplication.didEnterBackgroundNotification]
        #endif
        for name in leaving {
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.saveUi() }
            }
        }
    }

    func refresh() async {
        if let status = try? await bridge.call("status", as: Status.self) {
            signedIn = status.signedIn
            connection = status.connection
            server = status.server ?? ""
        }
        await reloadMailboxes()
        await reloadThreads()
    }

    private func handle(_ event: CoreEvent) {
        switch event {
        case .changed(let mailboxes, let threads, let preferences):
            prefetched.subtract(threads)
            if preferences { preferencesChanged() }
            Task {
                if mailboxes { await reloadMailboxes() }
                await reloadThreads()
                if let open = conversation?.id, threads.contains(open) { await reopen() }
            }
        case .connection(let state, _):
            connection = state
            signedIn = state != "signed_out"
            if state == "signed_out" {
                conversation = nil
                rows = []
                unified = []
                accounts = []
                selection = []
                Notifier.shared.setBadge(0)
            }
        case .sent:
            show("Sent.")
        case .sendFailed(_, let error, let draft, let draftId):
            show(error, action: "Edit") { [weak self] in
                self?.compose = Compose(draft: draft, from: "", showCc: !draft.cc.isEmpty, draftId: draftId, keep: true)
            }
        case .searchResults(let request, let found):
            guard request == searchRequest, searchRows != nil else { return }
            searchRows = found
        case .newMail(let messages):
            Notifier.shared.notify(messages)
        case .error(let message):
            show(message)
        case .other:
            break
        }
    }

    func reloadMailboxes() async {
        guard let found = try? await bridge.call("mailboxes", as: Mailboxes.self) else { return }
        if found.unified != unified { unified = found.unified }
        if found.accounts != accounts { accounts = found.accounts }
        Notifier.shared.setBadge(unified.first(where: { $0.id == "inbox" })?.unread ?? 0)
        if !accounts.isEmpty { Notifier.shared.askPermission() }
    }

    private func page(offset: Int, limit: Int) async -> ThreadPage? {
        var fields: [String: Any] = ["mailbox": mailbox, "offset": offset, "limit": limit]
        if let filter { fields["filter"] = filter.rawValue }
        return try? await bridge.call("threads", fields, as: ThreadPage.self)
    }

    /// Reads the list again. What didn't change isn't set, so the views it would redraw don't.
    func reloadThreads() async {
        guard !reloading else {
            staleWhileReloading = true
            return
        }
        reloading = true
        repeat {
            staleWhileReloading = false
            let asked = (mailbox, filter)
            guard let page = await page(offset: 0, limit: max(200, rows.count)), asked == (mailbox, filter) else { continue }
            let oldIndex = selected.flatMap { id in rows.firstIndex(where: { $0.id == id }) }
            if page.rows != rows { rows = page.rows }
            if page.total != total { total = page.total }
            if page.splits != splits { splits = page.splits }
            guard searchRows == nil else { continue }
            if !selection.isEmpty {
                let kept = selection.intersection(rows.lazy.map(\.id))
                if kept != selection { selection = kept }
            }
            if let selected, let oldIndex, !rows.contains(where: { $0.id == selected }) {
                self.selected = rows.isEmpty ? nil : rows[min(oldIndex, rows.count - 1)].id
            }
        } while staleWhileReloading
        reloading = false
    }

    func loadMore() {
        guard rows.count < total, !reloading else { return }
        reloading = true
        let asked = (mailbox, filter)
        let offset = rows.count
        Task {
            if let page = await page(offset: offset, limit: 200), asked == (mailbox, filter), offset == rows.count {
                rows += page.rows
                total = page.total
            }
            reloading = false
            guard staleWhileReloading else { return }
            await reloadThreads()
        }
    }

    func select(mailbox: String) {
        self.mailbox = mailbox
        conversation = nil
        searchRows = nil
        searchQuery = ""
        rows = []
        selection = []
        selectionAnchor = nil
        selected = nil
        listOffset = 0
        Task { await reloadThreads() }
    }

    /// One of the mailboxes every account has (`inbox`, `sent`, ...), in the account on screen
    /// when one is.
    func go(to kind: String) {
        let parts = mailbox.split(separator: "/")
        select(mailbox: parts.count > 1 ? "\(parts[0])/\(kind)" : kind)
    }

    /// Shows only unread or starred threads, or everything again.
    func toggleFilter(_ filter: Filter) {
        setFilter(self.filter == filter ? nil : filter)
    }

    func setFilter(_ filter: Filter?) {
        guard filter != self.filter else { return }
        self.filter = filter
        selection = []
        rows = []
        listOffset = 0
        Task { await reloadThreads() }
    }

    /// The next or previous tab of a split inbox.
    func moveSplit(_ step: Int) {
        guard !splits.isEmpty else { return }
        let index = splits.firstIndex(where: { $0.mailbox == mailbox }) ?? 0
        let next = splits[(index + step + splits.count) % splits.count].mailbox
        guard next != mailbox else { return }
        select(mailbox: next)
    }

    // MARK: Threads

    func row(_ thread: String) -> ThreadRow? {
        visibleRows.first(where: { $0.id == thread })
    }

    func open(_ thread: String) {
        selected = thread
        if let draft = row(thread)?.draftId {
            openDraft(draft)
            return
        }
        if conversation?.id != thread {
            unfoldedMessages = []
            focusedMessage = nil
            threadOffset = 0
        }
        Task {
            do {
                conversation = try await bridge.call("open_thread", ["thread": thread, "images": imagesShown.contains(thread)], as: Conversation.self)
            } catch {
                show(error.localizedDescription)
            }
        }
    }

    /// Opens the thread of a notification.
    func show(thread: String) {
        open(thread)
        #if os(iOS)
        requestedThread = thread
        #endif
    }

    private func reopen() async {
        guard let open = conversation?.id else { return }
        let fresh = try? await bridge.call("open_thread", ["thread": open, "images": imagesShown.contains(open)], as: Conversation.self)
        guard conversation?.id == open, let fresh else { return }
        conversation = fresh
    }

    func showImages() {
        guard let open = conversation?.id else { return }
        imagesShown.insert(open)
        Task { await reopen() }
    }

    func close() {
        conversation = nil
    }

    /// The thread an action from the keyboard or the toolbar is for: the open one, else the selected one.
    var current: String? { conversation?.id ?? selected }

    /// The threads an action is for: the selection over the list, else the current thread.
    var targets: [String] {
        guard conversation == nil, !selection.isEmpty else { return current.map { [$0] } ?? [] }
        return visibleRows.map(\.id).filter(selection.contains)
    }

    /// The threads an action on a row is for: the whole selection when the row is in it.
    func targets(for thread: String) -> [String] {
        selection.contains(thread) ? targets : [thread]
    }

    func act(_ action: Action, on given: [String], until: Date? = nil, label: String? = nil) {
        let threads = given.filter { !Self.isDraft($0) }
        guard !threads.isEmpty else { return }
        let leaving: Set<Action> = [.archive, .trash, .spam, .snooze, .inbox, .move, .mute, .block]
        if leaving.contains(action) {
            stepAway(from: Set(threads))
            selection.subtract(threads)
        }
        var fields: [String: Any] = ["action": action.rawValue, "threads": threads]
        if let until { fields["until"] = Int64(until.timeIntervalSince1970 * 1000) }
        if let label { fields["label"] = label }
        Task {
            do {
                report(try await bridge.call("act", fields, as: ActReply.self))
            } catch {
                show(error.localizedDescription)
            }
        }
    }

    /// Shows what an action did, with Undo when the core can take it back.
    func report(_ reply: ActReply) {
        if let url = reply.url.flatMap(URL.init(string:)) { Platform.open(url) }
        guard let message = reply.message else { return }
        guard reply.undo else {
            show(message)
            return
        }
        show(message, action: "Undo") { [weak self] in self?.undo() }
    }

    /// Takes back the last action that could be.
    func undo() {
        Task {
            do {
                show(try await bridge.call("undo", as: MessageReply.self).message)
            } catch {
                show(error.localizedDescription)
            }
        }
    }

    func toggleStar(_ thread: String) {
        toggleStar([thread])
    }

    func toggleStar(_ threads: [String]) {
        act(allStarred(threads) ? .unstar : .star, on: threads)
    }

    func toggleRead(_ thread: String) {
        toggleRead([thread])
    }

    func toggleRead(_ threads: [String]) {
        let unread = anyUnread(threads)
        act(unread ? .read : .unread, on: threads)
        if !unread, let open = conversation?.id, threads.contains(open) { conversation = nil }
    }

    func allStarred(_ threads: [String]) -> Bool {
        if threads.count == 1, conversation?.id == threads[0] { return conversation?.starred ?? false }
        let wanted = Set(threads)
        let found = visibleRows.filter { wanted.contains($0.id) }
        return !found.isEmpty && found.allSatisfy(\.starred)
    }

    func anyUnread(_ threads: [String]) -> Bool {
        let wanted = Set(threads)
        return visibleRows.contains { wanted.contains($0.id) && $0.unread }
    }

    /// The row after the given one that isn't leaving with it, else the one before.
    private func neighbour(of thread: String, leaving: Set<String>) -> String? {
        let list = visibleRows
        guard let index = list.firstIndex(where: { $0.id == thread }) else { return nil }
        if let after = list[(index + 1)...].first(where: { !leaving.contains($0.id) }) { return after.id }
        return list[..<index].last(where: { !leaving.contains($0.id) })?.id
    }

    /// Opens the next thread after ones that leave the list, as Newton did, or goes back to it.
    private func stepAway(from leaving: Set<String>) {
        if let opened = conversation?.id, leaving.contains(opened) {
            guard let next = neighbour(of: opened, leaving: leaving) else {
                conversation = nil
                return
            }
            #if os(macOS)
            open(next)
            #else
            conversation = nil
            selected = next
            #endif
        } else if let selected, leaving.contains(selected) {
            self.selected = neighbour(of: selected, leaving: leaving)
        }
    }

    /// Moves the keyboard's selection, or the open thread, by `step` rows.
    func move(_ step: Int) {
        let list = visibleRows
        guard !list.isEmpty else { return }
        let index = current.flatMap { id in list.firstIndex(where: { $0.id == id }) } ?? (step > 0 ? -1 : list.count)
        let nextIndex = min(max(index + step, 0), list.count - 1)
        let next = list[nextIndex].id
        if conversation != nil {
            open(next)
        } else {
            selected = next
        }
        prefetch(list[max(nextIndex - 2, 0)...min(nextIndex + 4, list.count - 1)].map(\.id))
        if index + step >= list.count - 20 { loadMore() }
    }

    /// Asks the core for the bodies of threads about to be opened.
    func prefetch(_ threads: [String]) {
        let fresh = threads.filter { !prefetched.contains($0) }
        guard !fresh.isEmpty else { return }
        prefetched.formUnion(fresh)
        bridge.send("prefetch", ["threads": fresh])
    }

    // MARK: Writing

    /// A new message from the first account.
    func newMessage() {
        startDraft(to: [])
    }

    /// A new message to someone, from the contact pane.
    func write(to address: Address) {
        startDraft(to: [address])
    }

    private func startDraft(to: [Address]) {
        Task {
            do {
                let made = try await bridge.call("new_draft", as: DraftReply.self)
                var draft = made.draft
                draft.to = to
                compose = Compose(draft: draft, from: made.from, showCc: false)
            } catch {
                show(error.localizedDescription)
            }
        }
    }

    func reply(_ kind: ReplyKind, to thread: String? = nil) {
        guard let thread = thread ?? current else { return }
        Task {
            do {
                let reply = try await bridge.call("reply_draft", ["thread": thread, "kind": kind.rawValue], as: DraftReply.self)
                compose = Compose(draft: reply.draft, from: reply.from, showCc: !reply.draft.cc.isEmpty, thread: thread)
            } catch {
                show(error.localizedDescription)
            }
        }
    }

    /// Opens a saved draft to write on.
    func openDraft(_ id: String) {
        Task {
            do {
                let opened = try await bridge.call("open_draft", ["id": id], as: DraftReply.self)
                let thread = conversation?.draftId == id ? conversation?.id : nil
                compose = Compose(
                    draft: opened.draft, from: opened.from, showCc: !opened.draft.cc.isEmpty || !opened.draft.bcc.isEmpty,
                    draftId: opened.id ?? id, thread: thread
                )
            } catch {
                show(error.localizedDescription)
            }
        }
    }

    /// Keeps what a compose sheet holds, after any save of it still on its way, and answers the
    /// saved draft's id.
    @discardableResult
    func saveDraft(_ compose: Compose) async -> String? {
        let previous = draftSaves[compose.id]
        let save = Task { () -> String? in
            let id = await previous?.value ?? compose.draftId
            var fields: [String: Any] = ["draft": CoreBridge.object(compose.draft)]
            if let id { fields["id"] = id }
            return (try? await bridge.call("save_draft", fields, as: IDReply.self).id) ?? id
        }
        draftSaves[compose.id] = save
        return await save.value
    }

    /// Closes the compose sheet. What was written is kept as a draft.
    func closeCompose(_ compose: Compose, keep: Bool) {
        self.compose = nil
        Task {
            if keep {
                await saveDraft(compose)
                show("Draft saved.")
            }
            draftSaves[compose.id] = nil
        }
    }

    func discardDraft(_ compose: Compose) {
        self.compose = nil
        Task {
            let id = await draftSaves[compose.id]?.value ?? compose.draftId
            draftSaves[compose.id] = nil
            if let id { _ = try? await bridge.call("delete_draft", ["id": id], as: Empty.self) }
            show("Draft discarded.")
        }
    }

    /// Sends after the undo delay, or at `sendAt`; archives the thread it answers with `archive`.
    func send(_ compose: Compose, at sendAt: Date? = nil, archive: Bool = false) {
        self.compose = nil
        if archive, let thread = compose.thread { act(.archive, on: [thread]) }
        Task {
            let draftId = await draftSaves[compose.id]?.value ?? compose.draftId
            draftSaves[compose.id] = nil
            var fields: [String: Any] = ["draft": CoreBridge.object(compose.draft), "delay": undoDelay]
            if let sendAt { fields["send_at"] = Int64(sendAt.timeIntervalSince1970 * 1000) }
            if let remind = compose.remindAt { fields["remind_at"] = remind.until }
            if let draftId { fields["draft_id"] = draftId }
            do {
                let sent = try await bridge.call("send", fields, as: SendReply.self)
                if let sendAt {
                    show("Sending \(sendAt.formatted(date: .abbreviated, time: .shortened)).", action: "Undo") { self.undoSend(sent.opId) }
                } else if undoDelay > 0 {
                    show("Sending…", action: "Undo", duration: TimeInterval(undoDelay)) { self.undoSend(sent.opId) }
                }
            } catch {
                show(error.localizedDescription)
                var again = compose
                again.draftId = draftId
                self.compose = again
            }
        }
    }

    private func undoSend(_ opId: String) {
        Task {
            do {
                let cancelled = try await bridge.call("cancel_send", ["op_id": opId], as: CancelReply.self)
                let from = accounts.first(where: { $0.id == cancelled.draft.accountId })?.address ?? ""
                compose = Compose(
                    draft: cancelled.draft, from: cancelled.draft.from?.email ?? from, showCc: !cancelled.draft.cc.isEmpty, draftId: cancelled.id, keep: true
                )
            } catch {
                show(error.localizedDescription)
            }
        }
    }

    /// The people an address field could mean, best first.
    func contacts(_ query: String) async -> [Address] {
        (try? await bridge.call("contacts", ["query": query], as: ContactList.self).contacts) ?? []
    }

    // The thread page and the contact pane.

    /// An attachment as a file on this device, downloaded first if it has to be.
    func openAttachment(message: String, index: Int) async -> URL? {
        let key = "\(message)/\(index)"
        downloading.insert(key)
        defer { downloading.remove(key) }
        do {
            let found = try await bridge.call("open_attachment", ["message": message, "index": index], as: PathReply.self)
            return URL(fileURLWithPath: found.path)
        } catch {
            show(error.localizedDescription)
            return nil
        }
    }

    func person(_ email: String) async -> Person? {
        try? await bridge.call("person", ["email": email], as: Person.self)
    }

    /// Unfolds every message of the open thread.
    func expandAllMessages() {
        guard let conversation else { return }
        unfoldedMessages.formUnion(conversation.messages.map(\.id))
    }

    /// Goes to the next (1) or the previous (-1) message of the open thread, unfolding it.
    func moveMessage(_ step: Int) {
        guard let messages = conversation?.messages, !messages.isEmpty else { return }
        let index = focusedMessage.flatMap { id in messages.firstIndex(where: { $0.id == id }) } ?? (step > 0 ? -1 : messages.count)
        let next = messages[min(max(index + step, 0), messages.count - 1)]
        unfoldedMessages.insert(next.id)
        focusedMessage = next.id
    }

    // MARK: Preferences

    /// Reads the synced preferences again: when one changed, or when a view needs them.
    func preferencesChanged() {
        Task { await loadPreferences() }
    }

    func loadPreferences() async {
        guard let list = try? await bridge.call("preferences", as: PreferenceList.self) else { return }
        preferences = Dictionary(list.values.map { ($0.key, $0.value) }, uniquingKeysWith: { _, last in last })
        preferencesLoaded = true
    }

    /// Changes a preference here at once and on the user's other devices after. Nil removes it.
    func setPreference(_ key: String, _ value: JSONValue?) {
        preferences[key] = value
        let fields: [String: Any] = ["key": key, "value": value?.any ?? NSNull()]
        Task {
            do {
                _ = try await bridge.call("set_preference", fields, as: Empty.self)
                if key == "remote_images" { await reopen() }
            } catch {
                show(error.localizedDescription)
                await loadPreferences()
            }
        }
    }

    /// What goes under a message from an identity: the user's own signature for the account, or
    /// else the provider's.
    func signature(account: String, email: String?) -> String? {
        if let own = preferences["signature:\(account)"]?.string, !own.isEmpty { return own }
        let identities = accounts.first(where: { $0.id == account })?.identities ?? []
        let identity = identities.first(where: { $0.email == email }) ?? identities.first
        return identity?.signature.flatMap { $0.isEmpty ? nil : $0 }
    }

    // MARK: Search

    func search(_ query: String) {
        searchQuery = query
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty else {
            searchRows = nil
            return
        }
        Task {
            guard let found = try? await bridge.call("search", ["query": query], as: SearchReply.self) else { return }
            guard searchQuery == query else { return }
            searchRequest = found.request
            searchRows = found.rows
        }
    }

    func endSearch() {
        searchQuery = ""
        searchRows = nil
    }

    // MARK: Accounts

    func signInWithGoogle() {
        Task {
            do {
                let start = try await bridge.call("sign_in_google", as: URLReply.self)
                guard let url = URL(string: start.url) else { return }
                let session = ASWebAuthenticationSession(url: url, callbackURLScheme: "mailapp") { callback, error in
                    Task { @MainActor in
                        guard let callback else {
                            if let error, (error as? ASWebAuthenticationSessionError)?.code != .canceledLogin {
                                self.show(error.localizedDescription)
                            }
                            return
                        }
                        await self.finishSignIn(callback)
                    }
                }
                session.presentationContextProvider = anchor
                session.prefersEphemeralWebBrowserSession = false
                signIn = session
                session.start()
            } catch {
                show(error.localizedDescription)
            }
        }
    }

    /// Finishes a sign-in from the `mailapp://auth` URL the browser came back with.
    public func finishSignIn(_ url: URL) async {
        do {
            _ = try await bridge.call("finish_sign_in", ["url": url.absoluteString], as: Empty.self)
            await refresh()
        } catch {
            show(error.localizedDescription)
        }
    }

    func addJmap(url: String, username: String, password: String) async throws {
        _ = try await bridge.call("add_jmap_account", ["url": url, "username": username, "password": password], as: Empty.self)
        await refresh()
    }

    func setServer(_ url: String) async throws {
        _ = try await bridge.call("set_server", ["url": url], as: Empty.self)
        await refresh()
    }

    func removeAccount(_ id: String) {
        Task {
            _ = try? await bridge.call("remove_account", ["account": id], as: Empty.self)
            show("Removing the account…")
        }
    }

    func signOut() {
        Task {
            _ = try? await bridge.call("sign_out", as: Empty.self)
            await refresh()
        }
    }

    // MARK: Toasts

    func show(_ message: String, action: String? = nil, duration: TimeInterval = 4, perform: (() -> Void)? = nil) {
        toast = Toast(message: message, action: action, duration: duration)
        toastAction = perform
    }

    func toastTapped() {
        let action = toastAction
        toast = nil
        toastAction = nil
        action?()
    }

    func dismissToast(_ toast: Toast) {
        guard self.toast == toast else { return }
        self.toast = nil
        toastAction = nil
    }
}

/// Where the Google sign-in sheet is shown.
final class SignInAnchor: NSObject, ASWebAuthenticationPresentationContextProviding {
    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        #if os(macOS)
        NSApp.keyWindow ?? NSApp.windows.first ?? ASPresentationAnchor()
        #else
        UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows).first(where: \.isKeyWindow) ?? ASPresentationAnchor()
        #endif
    }
}
