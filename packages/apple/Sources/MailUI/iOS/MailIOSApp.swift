#if os(iOS)
import SwiftUI

/// The iOS app: mailboxes, then a mailbox's threads, then a thread, in one navigation stack.
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

enum Route: Hashable {
    case mailbox(String)
    case thread(String)
}

struct IOSRoot: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        @Bindable var store = store
        Group {
            if !store.booted {
                // The page's colour alone, until the core says where the app was left.
                Tokens.card.color.ignoresSafeArea()
            } else if !store.signedIn {
                OnboardingView()
            } else {
                IOSStack(start: restored)
            }
        }
        .overlay(alignment: .bottom) { ToastView() }
        .sheet(item: $store.compose) { compose in ComposeView(compose: compose).environment(store) }
        .mailSheets(store)
    }

    /// The screens the app was left on: the mailbox, and the thread open in it.
    private var restored: [Route] {
        guard let open = store.conversation?.id else { return [.mailbox(store.mailbox)] }
        return [.mailbox(store.mailbox), .thread(open)]
    }
}

/// The navigation stack, starting on the screens the app was left on.
struct IOSStack: View {
    @Environment(MailStore.self) private var store
    @State private var path: [Route]

    init(start: [Route]) {
        _path = State(initialValue: start)
    }

    var body: some View {
        NavigationStack(path: $path) {
            SidebarView(showSettings: { store.settingsOpen = true }, picked: { path.append(.mailbox($0)) })
                .navigationTitle("Mailboxes")
                .navigationBarTitleDisplayMode(.inline)
                .toolbarBackground(Tokens.card.color, for: .navigationBar)
                .navigationDestination(for: Route.self) { route in
                    switch route {
                    case .mailbox: MailboxScreen(path: $path)
                    case .thread(let id): ThreadScreenIOS(thread: id)
                    }
                }
        }
        .tint(Tokens.primary.color)
        .onChange(of: store.requestedThread) { _, thread in
            guard let thread else { return }
            store.requestedThread = nil
            path = [.mailbox(store.mailbox), .thread(thread)]
        }
        .onChange(of: store.baseMailbox) {
            path.removeAll { if case .thread = $0 { true } else { false } }
        }
    }
}

/// A mailbox's threads, with search and compose; in edit mode, threads to act on together.
struct MailboxScreen: View {
    @Environment(MailStore.self) private var store
    @Binding var path: [Route]
    @State private var query = ""
    @State private var editing = false

    var body: some View {
        VStack(spacing: 0) {
            ListHeader()
            if store.visibleRows.isEmpty {
                VStack(spacing: 12) {
                    Image(store.searchRows != nil ? .search : .inbox, size: 30).foregroundStyle(Tokens.mutedMostForeground.color)
                    Text(emptyMessage).font(.ui(15)).foregroundStyle(Tokens.mutedMoreForeground.color)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Tokens.card.color)
            } else {
                ThreadListIOS(store: store, rows: store.visibleRows, editing: editing, checked: store.selection) { thread in
                    let draft = store.row(thread)?.draftId != nil
                    store.open(thread)
                    if !draft { path.append(.thread(thread)) }
                }
                .ignoresSafeArea(edges: .bottom)
            }
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

    private var emptyMessage: String {
        if store.searchRows != nil { return "Nothing matches." }
        if store.filter == .unread { return "Nothing unread." }
        if store.filter == .starred { return "Nothing starred." }
        return store.baseMailbox.hasSuffix("inbox") ? "All done. Enjoy the quiet." : "Nothing here."
    }

    @ToolbarContentBuilder private var toolbar: some ToolbarContent {
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
