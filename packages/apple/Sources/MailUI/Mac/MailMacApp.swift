#if os(macOS)
import AppKit
import SwiftUI

/// The Mac app: the list or the open thread, with the sidebar sliding over them from the left.
public struct MailMacApp: App {
    @State private var store = MailStore()
    @NSApplicationDelegateAdaptor(MacDelegate.self) private var delegate

    public init() {
        Notifier.shared.install()
    }

    private var defaultServer: String {
        Bundle.main.object(forInfoDictionaryKey: "MailServerURL") as? String ?? "http://localhost:3000"
    }

    public var body: some Scene {
        WindowGroup("Mail") {
            MacRoot()
                .environment(store)
                .preferredColorScheme(store.appearance.scheme)
                .onAppear {
                    delegate.store = store
                    store.start(defaultServer: defaultServer)
                }
                .onOpenURL { url in Task { await store.finishSignIn(url) } }
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: 1280, height: 820)
        .commands { MacCommands(store: store) }
    }
}

/// Saves where the app is, then lets it quit: SwiftUI on its own refuses to quit under a sheet.
@MainActor
final class MacDelegate: NSObject, NSApplicationDelegate {
    var store: MailStore?

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let store else { return .terminateNow }
        Task {
            await store.saveUiNow()
            sender.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }
}

struct MacRoot: View {
    @Environment(MailStore.self) private var store
    @State private var focusSearch = 0

    var body: some View {
        @Bindable var store = store
        Group {
            if !store.booted {
                // The page's colour alone, until the core says where the app was left.
                Tokens.card.color
            } else if !store.signedIn {
                OnboardingView().overlay(alignment: .bottom) { ToastView() }
            } else {
                ZStack(alignment: .leading) {
                    content
                    if store.sidebarOpen { sidebar }
                }
                .overlay(alignment: .bottom) { ToastView() }
                .background(KeyHandler(store: store, focusSearch: { focusSearch += 1 }, sidebarOpen: $store.sidebarOpen))
            }
        }
        .frame(minWidth: 860, minHeight: 520)
        .ignoresSafeArea()
        .mailSheets(store)
    }

    /// Over the page, without moving it; a click beside it closes it.
    @ViewBuilder private var sidebar: some View {
        Tokens.overlay.color.opacity(0.15)
            .contentShape(Rectangle())
            .onTapGesture { showSidebar(false) }
            .transition(.opacity)
        VStack(spacing: 0) {
            Color.clear.frame(height: 38)
            SidebarView(
                showSettings: {
                    showSidebar(false)
                    store.settingsOpen = true
                },
                picked: { _ in showSidebar(false) }
            )
        }
        .frame(width: Theme.sidebarWidth)
        .background(Tokens.sidebar.color)
        .overlay(alignment: .trailing) { Rectangle().fill(Tokens.sidebarBorder.color).frame(width: 1) }
        .transition(.move(edge: .leading))
    }

    private func showSidebar(_ open: Bool) {
        withAnimation(.easeOut(duration: 0.2)) { store.sidebarOpen = open }
    }

    private var content: some View {
        VStack(spacing: 0) {
            Text(store.conversation == nil && store.compose == nil ? store.mailboxName : "")
                .font(.ui(13, .medium))
                .foregroundStyle(Tokens.secondaryForeground.color)
                .frame(maxWidth: .infinity)
                .frame(height: 30)
                .allowsHitTesting(false)
            if case .ready(let version) = store.updater.state {
                Banner(symbol: .circleCheck, text: "Wonnet \(version) is installed. Restart to use it.", action: "Restart") { store.updater.relaunch() }
            }
            if let compose = store.compose {
                // Writing is a page of its own, as a thread is, with its bar on top.
                ComposeView(compose: compose).id(compose.id)
            } else {
                MacTopBar(focusSearch: focusSearch, showSidebar: { showSidebar(true) })
                Rectangle().fill(Tokens.border.color).frame(height: 1)
                page
            }
        }
        .background(Tokens.card.color)
    }

    /// The list, and the thread open over it.
    private var page: some View {
        ZStack {
            Tokens.background.color
            // The list stays under an open thread, so going back finds it where it was left.
            Group {
                if store.visibleRows.isEmpty, store.splits.isEmpty, store.filter == nil {
                    EmptyList()
                } else if store.visibleRows.isEmpty {
                    VStack(spacing: 0) {
                        ListHeader()
                        EmptyList().frame(maxHeight: .infinity)
                    }
                    .frame(maxWidth: Theme.cardWidth)
                    .background(Tokens.card.color)
                    .padding(.top, 16)
                    .padding(.horizontal, 24)
                } else {
                    // The list scrolls the whole page; only its header stays put above it.
                    let header = ListHeader.shows(store)
                    VStack(spacing: 0) {
                        if header {
                            ListHeader()
                                .frame(maxWidth: Theme.cardWidth)
                                .padding(.top, 16)
                                .padding(.horizontal, 24)
                        }
                        ThreadListMac(
                            store: store, rows: store.visibleRows, selected: store.selected, checked: store.selection,
                            topInset: header ? 0 : 16, shown: store.conversation == nil
                        )
                    }
                }
            }
            .opacity(store.conversation == nil ? 1 : 0)
            .allowsHitTesting(store.conversation == nil)
            if let conversation = store.conversation {
                ThreadScreen(conversation: conversation)
            }
        }
    }
}

/// Search and compose above the list; back and the thread's actions above a thread.
struct MacTopBar: View {
    @Environment(MailStore.self) private var store
    let focusSearch: Int
    let showSidebar: () -> Void
    @FocusState private var searching: Bool
    @State private var query = ""

    var body: some View {
        HStack(spacing: 12) {
            IconButton(symbol: .menu, help: "Mailboxes", circled: false, action: showSidebar)
            if let open = store.conversation {
                IconButton(symbol: .arrowLeft, help: "Back (Esc)") { store.close() }
                Spacer()
                ThreadActions(thread: open.id)
                Spacer()
            } else if !store.selection.isEmpty {
                SelectionBar()
            } else {
                HStack(spacing: 8) {
                    Image(.search, size: 14).foregroundStyle(Tokens.mutedForeground.color)
                    TextField("Search", text: $query)
                        .textFieldStyle(.plain)
                        .font(.ui(13))
                        .focused($searching)
                        .onSubmit { store.search(query) }
                        .onChange(of: query) { _, text in store.search(text) }
                        .onExitCommand {
                            query = ""
                            store.endSearch()
                            searching = false
                        }
                    if !query.isEmpty {
                        Button {
                            query = ""
                            store.endSearch()
                        } label: {
                            Image(.x, size: 12).foregroundStyle(Tokens.mutedForeground.color)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, 12)
                .frame(maxWidth: 420)
                .frame(height: 32)
                .overlay(Capsule().strokeBorder(Tokens.input.color, lineWidth: 1))
                Spacer()
            }
            ActionButton(title: "Compose", symbol: .squarePen, variant: .outline, action: store.newMessage)
        }
        .padding(.horizontal, 20)
        .frame(height: 52)
        .onAppear { query = store.searchQuery }
        .onChange(of: focusSearch) { searching = true }
        .onChange(of: store.searchQuery) { _, text in if text.isEmpty { query = "" } }
    }
}

/// An empty mailbox: a calm line instead of a list.
struct EmptyList: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        VStack(spacing: 12) {
            Image(store.searchRows != nil ? .search : .inbox, size: 30).foregroundStyle(Tokens.mutedForeground.color.opacity(0.6))
            Text(message).font(.ui(15)).foregroundStyle(Tokens.mutedForeground.color)
        }
    }

    private var message: String {
        if store.searchRows != nil { return "Nothing matches." }
        if store.filter == .unread { return "Nothing unread." }
        if store.filter == .starred { return "Nothing starred." }
        if store.connection == "connecting" && store.accounts.isEmpty { return "Syncing…" }
        return store.baseMailbox.hasSuffix("inbox") ? "All done. Enjoy the quiet." : "Nothing here."
    }
}
#endif
