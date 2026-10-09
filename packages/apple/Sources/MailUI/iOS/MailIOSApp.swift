#if os(iOS)
import SwiftUI

/// The iOS app: a mailbox's threads, then a thread, in one navigation stack, with the mailboxes
/// kept as tabs along the bottom and every mailbox in the drawer.
public struct MailIOSApp: App {
    @State private var store = MailStore()

    public init() {
        Notifier.shared.install()
    }

    private var defaultServer: String {
        Bundle.main.object(forInfoDictionaryKey: "MailServerURL") as? String ?? "http://localhost:3000"
    }

    public var body: some Scene {
        WindowGroup {
            IOSRoot()
                .environment(store)
                .preferredColorScheme(store.appearance.scheme)
                .onAppear { store.start(defaultServer: defaultServer) }
                .onOpenURL { url in Task { await store.finishSignIn(url) } }
        }
    }
}

struct IOSRoot: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        @Bindable var store = store
        Group {
            if !store.booted {
                // The page's colour alone, until the core says where the app was left.
                Tokens.background.color.ignoresSafeArea()
            } else if !store.signedIn {
                OnboardingView()
            } else {
                IOSShell(start: (store.searchRows == nil ? [] : [.search]) + (store.conversation.map { [.thread($0.id)] } ?? []))
                    .ignoresSafeArea()
            }
        }
        .overlay(alignment: .bottom) { ToastView() }
        .sheet(item: $store.compose) { compose in ComposeView(compose: compose).environment(store) }
        .mailSheets(store)
    }
}

/// A page over the mailbox: the search, or a thread.
enum Route: Hashable {
    case search
    case thread(String)
}

/// The drawer with the mailboxes, over the navigation stack of the mailbox on screen and the
/// pages opened over it, starting where the app was left.
struct IOSShell: View {
    @Environment(MailStore.self) private var store
    @State private var path: [Route]
    @State private var editing = false
    @State private var drawerOpen = false

    init(start: [Route]) {
        _path = State(initialValue: start)
    }

    var body: some View {
        DrawerView(isOpen: $drawerOpen, enabled: path.isEmpty && !editing) {
            SidebarView(showSettings: showSettings, picked: { _ in close() })
                .environment(store)
                .preferredColorScheme(store.appearance.scheme)
        } content: {
            NavigationStack(path: $path) {
                MailboxScreen(path: $path, editing: $editing, openDrawer: { drawerOpen = true })
                    .navigationDestination(for: Route.self) { route in
                        switch route {
                        case .search: SearchScreen(path: $path)
                        case .thread(let thread): ThreadScreenIOS(thread: thread)
                        }
                    }
            }
            .tint(Tokens.primary.color)
            .environment(store)
            .preferredColorScheme(store.appearance.scheme)
        }
        .onChange(of: store.requestedThread) { _, thread in
            guard let thread else { return }
            store.requestedThread = nil
            drawerOpen = false
            path = [.thread(thread)]
        }
        .onChange(of: store.baseMailbox) {
            path = []
            editing = false
        }
        .onChange(of: path) { _, path in
            // Leaving the search page ends the search.
            guard !path.contains(.search), !store.searchQuery.isEmpty else { return }
            store.endSearch()
        }
    }

    private func close() {
        drawerOpen = false
        path = []
    }

    private func showSettings() {
        drawerOpen = false
        store.settingsOpen = true
    }
}

/// A mailbox's threads under our own bar, with Compose floating over them; in edit mode,
/// threads to act on together. Search is a page of its own.
struct MailboxScreen: View {
    @Environment(MailStore.self) private var store
    @Binding var path: [Route]
    @Binding var editing: Bool
    let openDrawer: () -> Void

    var body: some View {
        // The table stays through a change of mailbox, with the empty state over it when there
        // is nothing to show, so it isn't made anew and the page never goes blank between.
        ThreadListIOS(
            store: store, rows: store.rows, editing: editing, checked: store.selection, ready: store.listReady,
            headerHeight: ListHeader.shows(store) ? ListHeader.height : 0,
            open: { thread in openThread(thread, path: $path, store: store) },
            header: { ListHeader().environment(store) }
        )
        .overlay {
            if store.rows.isEmpty, store.listReady { EmptyList().background(Tokens.background.color) }
        }
        .overlay(alignment: .bottomTrailing) {
            if !editing {
                FloatingButton(symbol: .squarePen, help: "Compose", action: store.newMessage).padding(Space.l)
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) { top }
        .safeAreaInset(edge: .bottom, spacing: 0) { if !editing { MailboxTabBar() } }
        .toolbar(.hidden, for: .navigationBar)
        .navigationTitle(store.baseMailboxName)
        .onChange(of: editing) { _, on in if !on { store.clearSelection() } }
        .toolbar {
            if editing {
                ToolbarItemGroup(placement: .bottomBar) {
                    CommandBar(threads: Array(store.selection))
                        .disabled(store.selection.isEmpty)
                }
                .clearGlass()
            }
        }
        .refreshable { await store.refresh() }
    }

    @ViewBuilder private var top: some View {
        if editing { editingBar } else { bar }
    }

    private var bar: some View {
        HeaderBar(inset: Space.s) {
            IconButton(symbol: .menu, help: "Mailboxes", circled: false, action: openDrawer)
        } center: {
            Text(store.baseMailboxName).textStyle(.subheading).lineLimit(1)
        } trailing: {
            IconButton(symbol: .search, help: "Search", circled: false) { path.append(.search) }
        }
        .topBar()
    }

    private var editingBar: some View {
        let all = store.selection.count == store.rows.count
        return HeaderBar(inset: Space.s) {
            ActionButton(title: all ? "Select None" : "Select All", variant: .ghost) { all ? store.clearSelection() : store.selectAll() }
        } center: {
            Text("\(store.selection.count) Selected").textStyle(.subheading).lineLimit(1)
        } trailing: {
            ActionButton(title: "Done", variant: .ghost) { editing = false }
        }
        .topBar()
    }
}

/// Opens a thread as a page over the list; a draft opens in compose instead.
@MainActor private func openThread(_ thread: String, path: Binding<[Route]>, store: MailStore) {
    let draft = store.row(thread)?.draftId != nil
    store.open(thread)
    if !draft { path.wrappedValue.append(.thread(thread)) }
}

/// Search as a page of its own: back and the field in the bar, the matches under them.
struct SearchScreen: View {
    @Environment(MailStore.self) private var store
    @Binding var path: [Route]
    @State private var query = ""

    var body: some View {
        ThreadListIOS(
            store: store, rows: store.searchRows ?? [], editing: false, checked: [], ready: true, keepsScroll: false,
            headerHeight: 0, open: { thread in openThread(thread, path: $path, store: store) }, header: { EmptyView() }
        )
        .overlay {
            if store.searchRows?.isEmpty == true { EmptyList().background(Tokens.background.color) }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            HeaderBar {
                IconButton(symbol: .arrowLeft, help: "Back", circled: false) { path.removeAll { $0 == .search } }
                SearchField(placeholder: "Search", text: $query, style: .bar, submit: { store.search(query) }, clear: store.endSearch)
            } trailing: {
                EmptyView()
            }
            .topBar()
        }
        .toolbar(.hidden, for: .navigationBar)
        .navigationTitle("Search")
        .onAppear { query = store.searchQuery }
        .onChange(of: query) { _, text in store.search(text) }
    }
}

extension View {
    /// A bar of ours at the top of a page: on the page's colour, up to the top of the screen, with
    /// a hairline under it.
    func topBar() -> some View {
        background(Tokens.background.color.ignoresSafeArea(edges: .top)).rule(.bottom)
    }
}

/// The mailboxes kept as tabs, along the bottom.
struct MailboxTabBar: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        if !store.tabs.isEmpty {
            BottomTabs(
                tabs: store.tabs, selected: store.baseMailbox,
                symbol: { Symbol.named($0.symbol) }, title: { store.tabName($0, shownIn: store.listAccount) }, count: \.unread,
                pick: { store.select(mailbox: $0.id) }
            )
        }
    }
}

/// What can be done to threads along the bottom: the usual few, and a menu of the rest.
struct CommandBar: View {
    @Environment(MailStore.self) private var store
    let threads: [String]
    var reply = false

    var body: some View {
        if store.applicable([.archive], to: threads).isEmpty {
            Spacer()
            Button { store.run(.trash, on: threads) } label: { Image(.trash, size: 18).glassFace() }.accessibilityLabel("Delete")
        } else {
            commands
        }
    }

    @ViewBuilder private var commands: some View {
        ForEach([ThreadCommand.archive, .trash, .snooze]) { command in
            Button { store.run(command, on: threads) } label: { Image(command.symbol, size: 18).glassFace() }
                .accessibilityLabel(command.title(store, threads))
            Spacer()
        }
        Menu {
            ThreadCommandButtons(commands: [.label, .move], threads: threads)
        } label: {
            Image(.tag, size: 18).glassFace()
        }
        .menuStyle(.button).buttonStyle(.plain)
        .accessibilityLabel("Label")
        Spacer()
        Menu {
            ThreadCommandButtons(commands: [.read] + ThreadCommand.more.filter { threads.count == 1 || $0 != .print }, threads: threads)
            if reply {
                Divider()
                ThreadCommandButtons(commands: [.replyAll, .forward], threads: threads)
            }
        } label: {
            Image(.ellipsis, size: 18).glassFace()
        }
        .menuStyle(.button).buttonStyle(.plain)
        .accessibilityLabel("More")
        if reply {
            Spacer()
            Button { store.run(.reply, on: threads) } label: { Image(.reply, size: 18).glassFace() }.accessibilityLabel("Reply")
        }
    }
}

extension ToolbarContent {
    /// The bar's controls without the system's frosted pill, so each wears its own clear glass.
    @ToolbarContentBuilder func clearGlass() -> some ToolbarContent {
        if #available(iOS 26, *) {
            sharedBackgroundVisibility(.hidden)
        } else {
            self
        }
    }
}

extension View {
    /// A bar control's face on clear glass: the mail shows through it as it scrolls under.
    @ViewBuilder func glassFace() -> some View {
        if #available(iOS 26, *) {
            padding(.horizontal, Space.m).frame(minWidth: Theme.fieldHeight, minHeight: Theme.fieldHeight)
                .glassEffect(.clear.interactive(), in: .capsule)
        } else {
            self
        }
    }
}

/// A thread, with what can be done to it along the bottom.
struct ThreadScreenIOS: View {
    @Environment(MailStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let thread: String

    var body: some View {
        Group {
            if let conversation = store.conversation, conversation.id == thread {
                ThreadScreen(conversation: conversation)
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItemGroup(placement: .bottomBar) {
                CommandBar(threads: [thread], reply: true)
            }
            .clearGlass()
        }
        .onChange(of: store.conversation?.id) { _, open in
            if open == nil { dismiss() }
        }
        .onDisappear {
            if store.conversation?.id == thread { store.close() }
        }
    }
}
#endif
