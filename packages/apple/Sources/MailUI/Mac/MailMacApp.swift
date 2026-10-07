#if os(macOS)
import AppKit
import SwiftUI

/// The Mac app: the sidebar on the left and, beside it, the list or the open thread.
public struct MailMacApp: App {
    @State private var store = MailStore()

    public init() {}

    private var defaultServer: String {
        Bundle.main.object(forInfoDictionaryKey: "MailServerURL") as? String ?? "http://localhost:3000"
    }

    public var body: some Scene {
        WindowGroup("Mail") {
            MacRoot()
                .environment(store)
                .preferredColorScheme(store.appearance.scheme)
                .onAppear { store.start(defaultServer: defaultServer) }
                .onOpenURL { url in Task { await store.finishSignIn(url) } }
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: 1280, height: 820)
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("New Message") { store.newMessage() }.keyboardShortcut("n")
            }
        }
    }
}

struct MacRoot: View {
    @Environment(MailStore.self) private var store
    @State private var settings = false
    @State private var focusSearch = 0

    var body: some View {
        @Bindable var store = store
        Group {
            if !store.signedIn {
                OnboardingView().overlay(alignment: .bottom) { ToastView() }
            } else {
                HStack(spacing: 0) {
                    VStack(spacing: 0) {
                        Color.clear.frame(height: 38)
                        SidebarView(showSettings: { settings = true })
                    }
                    .background(Tokens.sidebar.color)
                    .frame(width: Theme.sidebarWidth)
                    content
                }
                .overlay(alignment: .bottom) { ToastView() }
                .background(KeyHandler(focusSearch: { focusSearch += 1 }))
            }
        }
        .frame(minWidth: 860, minHeight: 520)
        .ignoresSafeArea()
        .sheet(item: $store.compose) { compose in ComposeView(compose: compose).environment(store) }
        .sheet(isPresented: Binding(get: { store.snoozing != nil }, set: { if !$0 { store.snoozing = nil } })) {
            SnoozePicker(threads: store.snoozing ?? []).environment(store)
        }
        .sheet(isPresented: $settings) { SettingsView().environment(store) }
    }

    private var content: some View {
        VStack(spacing: 0) {
            Text(store.conversation == nil ? store.mailboxName : "")
                .font(.ui(13, .medium))
                .foregroundStyle(Tokens.secondaryForeground.color)
                .frame(maxWidth: .infinity)
                .frame(height: 30)
                .allowsHitTesting(false)
            MacTopBar(focusSearch: focusSearch)
            Rectangle().fill(Tokens.border.color).frame(height: 1)
            ZStack {
                Tokens.background.color
                if let conversation = store.conversation {
                    ThreadScreen(conversation: conversation)
                } else if store.visibleRows.isEmpty {
                    EmptyList()
                } else {
                    ThreadListMac(store: store, rows: store.visibleRows, selected: store.selected)
                        .frame(maxWidth: Theme.cardWidth)
                        .background(Tokens.card.color)
                        .padding(.top, 16)
                        .padding(.horizontal, 24)
                }
            }
        }
        .background(Tokens.card.color)
    }
}

/// Search and compose above the list; back and the thread's actions above a thread.
struct MacTopBar: View {
    @Environment(MailStore.self) private var store
    let focusSearch: Int
    @FocusState private var searching: Bool
    @State private var query = ""

    var body: some View {
        HStack(spacing: 12) {
            if let open = store.conversation {
                IconButton(symbol: .arrowLeft, help: "Back (Esc)") { store.close() }
                Spacer()
                ThreadActions(thread: open.id)
                Spacer()
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
        .onChange(of: focusSearch) { searching = true }
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
        if store.connection == "connecting" && store.accounts.isEmpty { return "Syncing…" }
        return store.mailbox.hasSuffix("inbox") ? "All done. Enjoy the quiet." : "Nothing here."
    }
}

/// Newton's single keys: j/k to move, e archive, s star, # trash, u unread, h snooze, r reply,
/// a reply all, f forward, c compose, / search. Ignored while typing or with a sheet open.
private struct KeyHandler: NSViewRepresentable {
    @Environment(MailStore.self) private var store
    let focusSearch: () -> Void

    final class Coordinator {
        var monitor: Any?
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let store = store
        let focusSearch = focusSearch
        context.coordinator.monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            let typing = NSApp.keyWindow?.firstResponder is NSText
            let sheetOpen = store.compose != nil || store.snoozing != nil || NSApp.keyWindow?.attachedSheet != nil
            let modified = !event.modifierFlags.intersection([.command, .control, .option]).isEmpty
            guard !typing, !sheetOpen, !modified else { return event }
            return MainActor.assumeIsolated { handle(event, store: store, focusSearch: focusSearch) } ? nil : event
        }
        return NSView()
    }

    static func dismantleNSView(_ view: NSView, coordinator: Coordinator) {
        if let monitor = coordinator.monitor { NSEvent.removeMonitor(monitor) }
    }

    func updateNSView(_ view: NSView, context: Context) {}

    @MainActor
    private func handle(_ event: NSEvent, store: MailStore, focusSearch: () -> Void) -> Bool {
        let thread = store.current
        switch (event.keyCode, event.charactersIgnoringModifiers ?? "") {
        case (125, _), (_, "j"): store.move(1)
        case (126, _), (_, "k"): store.move(-1)
        case (36, _): if let thread, store.conversation == nil { store.open(thread) } else { return false }
        case (53, _):
            if store.conversation != nil { store.close() } else if store.searchRows != nil { store.endSearch() } else { return false }
        case (_, "e"): if let thread { store.act(.archive, on: [thread]) }
        case (_, "s"): if let thread { store.toggleStar(thread) }
        case (_, "#"): if let thread { store.act(.trash, on: [thread]) }
        case (_, "u"): if let thread { store.toggleRead(thread) }
        case (_, "h"): if let thread { store.snoozing = [thread] }
        case (_, "r"): store.reply(.reply)
        case (_, "a"): store.reply(.replyAll)
        case (_, "f"): store.reply(.forward)
        case (_, "c"): store.newMessage()
        case (_, "/"): focusSearch()
        default: return false
        }
        return true
    }
}
#endif
