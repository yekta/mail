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
                IOSShell(start: store.conversation.map { [$0.id] } ?? [])
                    .ignoresSafeArea()
            }
        }
        .overlay(alignment: .bottom) { ToastView() }
        .sheet(item: $store.compose) { compose in ComposeView(compose: compose).environment(store) }
        .mailSheets(store)
    }
}

/// The drawer with the mailboxes, over the navigation stack of the mailbox on screen and the
/// threads opened in it, starting on the thread the app was left on.
struct IOSShell: View {
    @Environment(MailStore.self) private var store
    @State private var path: [String]
    @State private var editing = false
    @State private var drawerOpen = false

    init(start: [String]) {
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
                    .navigationDestination(for: String.self) { ThreadScreenIOS(thread: $0) }
            }
            .tint(Tokens.primary.color)
            .environment(store)
            .preferredColorScheme(store.appearance.scheme)
        }
        .onChange(of: store.requestedThread) { _, thread in
            guard let thread else { return }
            store.requestedThread = nil
            drawerOpen = false
            path = [thread]
        }
        .onChange(of: store.baseMailbox) {
            path = []
            editing = false
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

/// A mailbox's threads, with search and compose; in edit mode, threads to act on together.
struct MailboxScreen: View {
    @Environment(MailStore.self) private var store
    @Binding var path: [String]
    @Binding var editing: Bool
    let openDrawer: () -> Void
    @State private var query = ""

    var body: some View {
        VStack(spacing: 0) {
            ListHeader()
            // The table stays through a change of mailbox, with the empty state over it when there
            // is nothing to show, so it isn't made anew and the page never goes blank between.
            ThreadListIOS(store: store, rows: store.visibleRows, editing: editing, checked: store.selection, ready: store.listReady) { thread in
                let draft = store.row(thread)?.draftId != nil
                store.open(thread)
                if !draft { path.append(thread) }
            }
            .ignoresSafeArea(edges: .bottom)
            .overlay {
                if store.listEmpty { EmptyList().background(Tokens.background.color) }
            }
            if !editing { MailboxTabBar() }
        }
        .navigationTitle(editing ? "\(store.selection.count) Selected" : store.mailboxName)
        .navigationBarTitleDisplayMode(.inline)
        .navigationBarBackButtonHidden(editing)
        .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .automatic))
        .onAppear { query = store.searchQuery }
        .onChange(of: query) { _, text in store.search(text) }
        .onChange(of: store.searchQuery) { _, text in if text.isEmpty { query = "" } }
        .onChange(of: editing) { _, on in if !on { store.clearSelection() } }
        .toolbar {
            if editing {
                editingToolbar
            } else {
                toolbar
            }
        }
        .refreshable { await store.refresh() }
    }

    @ToolbarContentBuilder private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .topBarLeading) {
            Button(action: openDrawer) { Image(.menu, size: 18) }.accessibilityLabel("Mailboxes")
        }
        ToolbarItemGroup(placement: .topBarTrailing) {
            Menu {
                Button { editing = true } label: { Label { Text("Select") } icon: { Image(.squareCheck, size: 15) } }
                Button { store.toggleFilter(.unread) } label: {
                    Label { Text(store.filter == .unread ? "Show everything" : "Unread only") } icon: { Image(.listFilter, size: 15) }
                }
                Button { store.toggleFilter(.starred) } label: {
                    Label { Text(store.filter == .starred ? "Show everything" : "Starred only") } icon: { Image(.star, size: 15) }
                }
                Button { store.showPalette() } label: { Label { Text("Commands") } icon: { Image(.command, size: 15) } }
                if store.searchRows == nil, !store.rows.isEmpty {
                    Divider()
                    Button(action: store.archiveAll) { Label { Text("Get Me To Zero") } icon: { Image(.archive, size: 15) } }
                }
            } label: {
                Image(.ellipsis, size: 18)
            }
            .accessibilityLabel("More")
            Button(action: store.newMessage) { Image(.squarePen, size: 18) }.accessibilityLabel("Compose")
        }
    }

    @ToolbarContentBuilder private var editingToolbar: some ToolbarContent {
        let threads = Array(store.selection)
        ToolbarItem(placement: .topBarLeading) {
            Button(store.selection.count == store.visibleRows.count ? "Select None" : "Select All") {
                store.selection.count == store.visibleRows.count ? store.clearSelection() : store.selectAll()
            }
        }
        ToolbarItem(placement: .topBarTrailing) {
            Button("Done") { editing = false }.fontWeight(.semibold)
        }
        ToolbarItemGroup(placement: .bottomBar) {
            CommandBar(threads: threads)
                .disabled(threads.isEmpty)
        }
    }
}

/// The mailboxes kept as tabs, along the bottom; gone while the search field is up.
struct MailboxTabBar: View {
    @Environment(MailStore.self) private var store
    @Environment(\.isSearching) private var searching

    var body: some View {
        if !store.tabs.isEmpty, !searching {
            BottomTabs(
                tabs: store.tabs, selected: store.searchRows == nil ? store.baseMailbox : nil,
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
            Button { store.run(.trash, on: threads) } label: { Image(.trash, size: 18) }.accessibilityLabel("Delete")
        } else {
            commands
        }
    }

    @ViewBuilder private var commands: some View {
        ForEach([ThreadCommand.archive, .trash, .snooze]) { command in
            Button { store.run(command, on: threads) } label: { Image(command.symbol, size: 18) }
                .accessibilityLabel(command.title(store, threads))
            Spacer()
        }
        Menu {
            ThreadCommandButtons(commands: [.label, .move], threads: threads)
        } label: {
            Image(.tag, size: 18)
        }
        .accessibilityLabel("Label")
        Spacer()
        Menu {
            ThreadCommandButtons(commands: [.read] + ThreadCommand.more.filter { threads.count == 1 || $0 != .print }, threads: threads)
            if reply {
                Divider()
                ThreadCommandButtons(commands: [.replyAll, .forward], threads: threads)
            }
        } label: {
            Image(.ellipsis, size: 18)
        }
        .accessibilityLabel("More")
        if reply {
            Spacer()
            Button { store.run(.reply, on: threads) } label: { Image(.reply, size: 18) }.accessibilityLabel("Reply")
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
