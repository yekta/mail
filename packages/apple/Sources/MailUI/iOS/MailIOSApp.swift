#if os(iOS)
import SwiftUI

/// The iOS app: mailboxes, then a mailbox's threads, then a thread, in one navigation stack.
public struct MailIOSApp: App {
    @State private var store = MailStore()

    public init() {}

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
    @State private var path: [Route] = [.mailbox("inbox")]
    @State private var settings = false

    var body: some View {
        @Bindable var store = store
        Group {
            if !store.signedIn {
                OnboardingView()
            } else {
                NavigationStack(path: $path) {
                    SidebarView(showSettings: { settings = true }, picked: { path.append(.mailbox($0)) })
                        .navigationTitle("Mailboxes")
                        .navigationBarTitleDisplayMode(.inline)
                        .toolbarBackground(Tokens.sidebar.color, for: .navigationBar)
                        .toolbarColorScheme(.dark, for: .navigationBar)
                        .navigationDestination(for: Route.self) { route in
                            switch route {
                            case .mailbox(let id): MailboxScreen(mailbox: id, path: $path)
                            case .thread(let id): ThreadScreenIOS(thread: id)
                            }
                        }
                }
                .tint(Tokens.primary.color)
            }
        }
        .overlay(alignment: .bottom) { ToastView() }
        .sheet(item: $store.compose) { compose in ComposeView(compose: compose).environment(store) }
        .sheet(isPresented: Binding(get: { store.snoozing != nil }, set: { if !$0 { store.snoozing = nil } })) {
            SnoozePicker(threads: store.snoozing ?? []).environment(store).presentationDetents([.medium])
        }
        .sheet(isPresented: $settings) { SettingsView().environment(store) }
    }
}

/// A mailbox's threads, with search and compose.
struct MailboxScreen: View {
    @Environment(MailStore.self) private var store
    let mailbox: String
    @Binding var path: [Route]
    @State private var query = ""

    var body: some View {
        Group {
            if store.visibleRows.isEmpty {
                VStack(spacing: 12) {
                    Image(store.searchRows != nil ? .search : .inbox, size: 30).foregroundStyle(Tokens.mutedForeground.color.opacity(0.6))
                    Text(store.searchRows != nil ? "Nothing matches." : "All done. Enjoy the quiet.")
                        .font(.ui(15)).foregroundStyle(Tokens.mutedForeground.color)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Tokens.card.color)
            } else {
                ThreadListIOS(store: store, rows: store.visibleRows) { thread in
                    store.open(thread)
                    path.append(.thread(thread))
                }
                .ignoresSafeArea(edges: .bottom)
            }
        }
        .navigationTitle(store.mailboxName)
        .navigationBarTitleDisplayMode(.inline)
        .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .automatic))
        .onChange(of: query) { _, text in store.search(text) }
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(action: store.newMessage) { Image(.squarePen, size: 18) }.accessibilityLabel("Compose")
            }
        }
        .onAppear {
            if store.mailbox != mailbox { store.select(mailbox: mailbox) }
        }
        .refreshable { await store.refresh() }
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
                Button { store.act(.archive, on: [thread]) } label: { Image(.archive, size: 18) }.accessibilityLabel("Archive")
                Spacer()
                Button { store.act(.trash, on: [thread]) } label: { Image(.trash, size: 18) }.accessibilityLabel("Trash")
                Spacer()
                Button { store.snoozing = [thread] } label: { Image(.clock, size: 18) }.accessibilityLabel("Snooze")
                Spacer()
                Button { store.toggleRead(thread) } label: { Image(.mail, size: 18) }.accessibilityLabel("Mark unread")
                Spacer()
                Button { store.reply(.reply, to: thread) } label: { Image(.reply, size: 18) }.accessibilityLabel("Reply")
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
