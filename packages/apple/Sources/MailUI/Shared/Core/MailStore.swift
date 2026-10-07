import AuthenticationServices
import Foundation
import Observation
import SwiftUI

/// A note at the bottom of the window, sometimes with something to do about it.
struct Toast: Identifiable, Equatable {
    let id = UUID()
    let message: String
    var action: String?
    var duration: TimeInterval = 4

    static func == (lhs: Toast, rhs: Toast) -> Bool { lhs.id == rhs.id }
}

/// A message being written.
struct Compose: Identifiable {
    let id = UUID()
    var draft: Draft
    var from: String
    var showCc: Bool
}

/// When a snooze can end.
struct SnoozeChoice: Identifiable {
    let id: String
    let name: String
    let until: Date

    static func choices(now: Date = Date(), calendar: Calendar = .current) -> [SnoozeChoice] {
        let at = { (day: Date, hour: Int) in calendar.date(bySettingHour: hour, minute: 0, second: 0, of: day) ?? day }
        let tomorrow = calendar.date(byAdding: .day, value: 1, to: now) ?? now
        let saturday = calendar.nextDate(after: now, matching: DateComponents(weekday: 7), matchingPolicy: .nextTime) ?? tomorrow
        let monday = calendar.nextDate(after: now, matching: DateComponents(weekday: 2), matchingPolicy: .nextTime) ?? tomorrow
        var choices = [SnoozeChoice(id: "later", name: "Later today", until: now.addingTimeInterval(3 * 3600))]
        if calendar.component(.hour, from: now) < 18 {
            choices.append(SnoozeChoice(id: "evening", name: "This evening", until: at(now, 19)))
        }
        choices += [
            SnoozeChoice(id: "tomorrow", name: "Tomorrow", until: at(tomorrow, 8)),
            SnoozeChoice(id: "weekend", name: "This weekend", until: at(saturday, 9)),
            SnoozeChoice(id: "week", name: "Next week", until: at(monday, 8)),
        ]
        return choices
    }
}

@Observable @MainActor
public final class MailStore {
    let bridge = CoreBridge()

    var started = false
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
    /// Threads whose remote images were allowed.
    var imagesShown: Set<String> = []
    var undoDelay: Int = UserDefaults.standard.object(forKey: "undoDelay") as? Int ?? 10 {
        didSet { UserDefaults.standard.set(undoDelay, forKey: "undoDelay") }
    }
    var appearance = Appearance(rawValue: UserDefaults.standard.string(forKey: "appearance") ?? "") ?? .system {
        didSet { UserDefaults.standard.set(appearance.rawValue, forKey: "appearance") }
    }

    @ObservationIgnored private var toastAction: (() -> Void)?
    @ObservationIgnored private var searchRequest: UInt64 = 0
    @ObservationIgnored private var reloading = false
    @ObservationIgnored private var signIn: ASWebAuthenticationSession?
    @ObservationIgnored private let anchor = SignInAnchor()

    public init() {}

    /// The rows the list shows: the search's while searching.
    var visibleRows: [ThreadRow] { searchRows ?? rows }

    var mailboxName: String {
        if searchRows != nil { return "Search" }
        let parts = mailbox.split(separator: "/").map(String.init)
        let all = unified + accounts.flatMap(\.mailboxes)
        let name = all.first(where: { $0.id == mailbox })?.name ?? "Inbox"
        guard parts.count > 1, let account = accounts.first(where: { $0.id == parts[0] }) else {
            return name == "Inbox" && accounts.count > 1 ? "All Inboxes" : name
        }
        return "\(name) · \(account.address)"
    }

    // MARK: Starting

    public func start(defaultServer: String) {
        guard !started else { return }
        started = true
        bridge.onEvent = { [weak self] event in self?.handle(event) }
        try? FileManager.default.createDirectory(at: Platform.dataFolder, withIntermediateDirectories: true)
        let config: [String: Any] = ["data_dir": Platform.dataFolder.path, "server_url": defaultServer]
        guard bridge.start(config: config) else {
            show("The mail store couldn't open.")
            return
        }
        Task { await refresh() }
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
        case .changed(let mailboxes, let threads):
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
            }
        case .sent:
            show("Sent.")
        case .sendFailed(_, let error, let draft):
            show(error, action: "Edit") { [weak self] in self?.compose = Compose(draft: draft, from: "", showCc: !draft.cc.isEmpty) }
        case .searchResults(let request, let found):
            guard request == searchRequest, searchRows != nil else { return }
            searchRows = found
        case .error(let message):
            show(message)
        case .other:
            break
        }
    }

    func reloadMailboxes() async {
        guard let found = try? await bridge.call("mailboxes", as: Mailboxes.self) else { return }
        unified = found.unified
        accounts = found.accounts
    }

    func reloadThreads() async {
        guard !reloading else { return }
        reloading = true
        defer { reloading = false }
        let limit = max(200, rows.count)
        guard let page = try? await bridge.call("threads", ["mailbox": mailbox, "limit": limit], as: ThreadPage.self) else { return }
        rows = page.rows
        total = page.total
    }

    func loadMore() {
        guard rows.count < total, !reloading else { return }
        Task {
            reloading = true
            defer { reloading = false }
            let page = try? await bridge.call("threads", ["mailbox": mailbox, "offset": rows.count, "limit": 200], as: ThreadPage.self)
            guard let page else { return }
            rows += page.rows
            total = page.total
        }
    }

    func select(mailbox: String) {
        self.mailbox = mailbox
        conversation = nil
        searchRows = nil
        searchQuery = ""
        rows = []
        Task { await reloadThreads() }
    }

    // MARK: Threads

    func open(_ thread: String) {
        selected = thread
        Task {
            do {
                conversation = try await bridge.call("open_thread", ["thread": thread, "images": imagesShown.contains(thread)], as: Conversation.self)
            } catch {
                show(error.localizedDescription)
            }
        }
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

    func act(_ action: Action, on threads: [String], until: Date? = nil) {
        guard !threads.isEmpty else { return }
        let leaving: Set<Action> = [.archive, .trash, .spam, .snooze, .inbox]
        if leaving.contains(action), let open = conversation?.id, threads.contains(open) {
            moveAfter(open)
        } else if leaving.contains(action), let selected, threads.contains(selected) {
            self.selected = neighbour(of: selected)
        }
        var fields: [String: Any] = ["action": action.rawValue, "threads": threads]
        if let until { fields["until"] = Int64(until.timeIntervalSince1970 * 1000) }
        Task {
            do {
                _ = try await bridge.call("act", fields, as: Empty.self)
            } catch {
                show(error.localizedDescription)
            }
            let undoable: [Action: (String, Action)] = [.archive: ("Archived.", .inbox), .trash: ("Moved to Trash.", .inbox)]
            if let (message, undo) = undoable[action] {
                show(message, action: "Undo") { self.act(undo, on: threads) }
            } else if action == .snooze, let until {
                show("Snoozed until \(until.formatted(date: .abbreviated, time: .shortened)).")
            }
        }
    }

    func toggleStar(_ thread: String) {
        let starred = visibleRows.first(where: { $0.id == thread })?.starred ?? conversation?.starred ?? false
        act(starred ? .unstar : .star, on: [thread])
    }

    func toggleRead(_ thread: String) {
        let unread = visibleRows.first(where: { $0.id == thread })?.unread ?? false
        act(unread ? .read : .unread, on: [thread])
        if !unread, conversation?.id == thread { conversation = nil }
    }

    private func neighbour(of thread: String) -> String? {
        let list = visibleRows
        guard let index = list.firstIndex(where: { $0.id == thread }) else { return nil }
        if index + 1 < list.count { return list[index + 1].id }
        return index > 0 ? list[index - 1].id : nil
    }

    /// Opens the next thread after one that leaves the list, as Newton did, or goes back to it.
    private func moveAfter(_ thread: String) {
        guard let next = neighbour(of: thread) else {
            conversation = nil
            return
        }
        #if os(macOS)
        open(next)
        #else
        conversation = nil
        selected = next
        #endif
    }

    /// Moves the keyboard's selection, or the open thread, by `step` rows.
    func move(_ step: Int) {
        let list = visibleRows
        guard !list.isEmpty else { return }
        let index = current.flatMap { id in list.firstIndex(where: { $0.id == id }) } ?? (step > 0 ? -1 : list.count)
        let next = list[min(max(index + step, 0), list.count - 1)].id
        if conversation != nil {
            open(next)
        } else {
            selected = next
        }
        if index + step >= list.count - 20 { loadMore() }
    }

    // MARK: Writing

    func newMessage() {
        let account = accounts.first
        compose = Compose(draft: Draft(accountId: account?.id ?? "", to: [], subject: "", text: ""), from: account?.address ?? "", showCc: false)
    }

    func reply(_ kind: ReplyKind, to thread: String? = nil) {
        guard let thread = thread ?? current else { return }
        Task {
            do {
                let reply = try await bridge.call("reply_draft", ["thread": thread, "kind": kind.rawValue], as: DraftReply.self)
                compose = Compose(draft: reply.draft, from: reply.from, showCc: !reply.draft.cc.isEmpty)
            } catch {
                show(error.localizedDescription)
            }
        }
    }

    func send(_ draft: Draft) {
        compose = nil
        Task {
            do {
                let sent = try await bridge.call("send", ["draft": CoreBridge.object(draft), "delay": undoDelay], as: SendReply.self)
                guard undoDelay > 0 else { return }
                show("Sending…", action: "Undo", duration: TimeInterval(undoDelay)) { self.undoSend(sent.opId) }
            } catch {
                show(error.localizedDescription)
                compose = Compose(draft: draft, from: "", showCc: !draft.cc.isEmpty)
            }
        }
    }

    private func undoSend(_ opId: String) {
        Task {
            do {
                let cancelled = try await bridge.call("cancel_send", ["op_id": opId], as: CancelReply.self)
                compose = Compose(draft: cancelled.draft, from: accounts.first(where: { $0.id == cancelled.draft.accountId })?.address ?? "", showCc: !cancelled.draft.cc.isEmpty)
            } catch {
                show(error.localizedDescription)
            }
        }
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
