#if os(macOS)
import AppKit
import SwiftUI

/// The menu bar's commands: the keys with ⌘, where they can be found.
struct MacCommands: Commands {
    let store: MailStore

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("New Message") { store.newMessage() }.keyboardShortcut("n").disabled(store.sheetOpen)
        }
        CommandGroup(after: .appInfo) {
            Button("Check for Updates…") { store.updater.check(asked: true) }
        }
        CommandGroup(replacing: .appSettings) {
            Button("Settings…") { store.settingsOpen = true }.keyboardShortcut(",").disabled(store.sheetOpen)
        }
        CommandGroup(replacing: .printItem) {
            Button("Print…") { store.run(.print) }.keyboardShortcut("p").disabled(store.sheetOpen || store.current == nil)
        }
        CommandMenu("Message") {
            Button("Command Palette…") { store.showPalette() }.keyboardShortcut("k").disabled(store.sheetOpen)
            Button("Keyboard Shortcuts") { store.shortcutsOpen = true }.keyboardShortcut("/").disabled(store.sheetOpen)
            Divider()
            Button("Unsubscribe") { store.run(.unsubscribe) }.keyboardShortcut("u").disabled(store.sheetOpen || store.current == nil && store.selection.isEmpty)
            Button("Get Me To Zero…") { store.archiveAll() }.disabled(store.sheetOpen || store.searchRows != nil || store.rows.isEmpty)
        }
        CommandMenu("Go") {
            Button("All Inboxes") { store.select(mailbox: "inbox") }.keyboardShortcut("0").disabled(store.sheetOpen)
            ForEach(Array(store.accounts.prefix(9).enumerated()), id: \.element.id) { index, account in
                Button(account.address) { store.select(mailbox: "\(account.id)/inbox") }
                    .keyboardShortcut(KeyEquivalent(Character("\(index + 1)")))
                    .disabled(store.sheetOpen)
            }
        }
    }
}

/// The single keys of Newton and Superhuman (the shortcuts sheet lists them all), ignored while
/// typing or with a sheet open.
struct KeyHandler: NSViewRepresentable {
    let store: MailStore
    let focusSearch: () -> Void
    @Binding var sidebarOpen: Bool

    func makeCoordinator() -> KeyCoordinator { KeyCoordinator(store: store) }

    func makeNSView(context: Context) -> NSView {
        let coordinator = context.coordinator
        let view = NSView()
        coordinator.view = view
        coordinator.monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            MainActor.assumeIsolated { coordinator.handle(event) } ? nil : event
        }
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {
        context.coordinator.focusSearch = focusSearch
        context.coordinator.sidebarOpen = $sidebarOpen
    }

    static func dismantleNSView(_ view: NSView, coordinator: KeyCoordinator) {
        if let monitor = coordinator.monitor { NSEvent.removeMonitor(monitor) }
    }
}

@MainActor
final class KeyCoordinator {
    let store: MailStore
    var monitor: Any?
    /// In the window the keys are for: another window's keys (Quick Look's) are left alone.
    weak var view: NSView?
    var focusSearch: () -> Void = {}
    var sidebarOpen: Binding<Bool>?
    /// When `g` was pressed, for the key that says where to go.
    private var goPressed: Date?

    private static let places = ["i": "inbox", "s": "starred", "t": "sent", "d": "drafts", "a": "archive", "h": "snoozed", "!": "spam", "#": "trash"]

    init(store: MailStore) {
        self.store = store
    }

    func handle(_ event: NSEvent) -> Bool {
        guard let window = view?.window, event.window === window, window.isKeyWindow, window.attachedSheet == nil else { return false }
        guard !store.sheetOpen, !(window.firstResponder is NSText) else { return false }
        let flags = event.modifierFlags.intersection([.command, .control, .option])
        let key = event.charactersIgnoringModifiers ?? ""
        if flags == .command { return command(key) }
        guard flags.isEmpty else { return false }
        if let pressed = goPressed {
            goPressed = nil
            if Date().timeIntervalSince(pressed) < 1.5 { return go(key) }
        }
        let shift = event.modifierFlags.contains(.shift)
        switch event.keyCode {
        case 125: shift ? store.extendSelection(1) : store.move(1)
        case 126: shift ? store.extendSelection(-1) : store.move(-1)
        case 36, 76: return enter()
        case 53: return escape()
        case 51, 117: store.run(.trash)
        case 48:
            guard !store.splits.isEmpty, store.conversation == nil else { return false }
            store.moveSplit(shift ? -1 : 1)
        default: return character(key)
        }
        return true
    }

    private func command(_ key: String) -> Bool {
        switch key {
        case "z": store.undo()
        case "a":
            guard store.conversation == nil, !store.visibleRows.isEmpty else { return false }
            store.selectAll()
        default: return false
        }
        return true
    }

    private func character(_ key: String) -> Bool {
        let inThread = store.conversation != nil
        switch key {
        case "j": store.move(1)
        case "k": store.move(-1)
        case "J": store.extendSelection(1)
        case "K": store.extendSelection(-1)
        case "o": return open()
        case "O":
            guard inThread else { return false }
            store.expandAllMessages()
        case "n", "p":
            guard inThread else { return false }
            store.moveMessage(key == "n" ? 1 : -1)
        case "e": store.run(.archive)
        case "E": store.run(.inbox)
        case "#": store.run(.trash)
        case "!": store.run(.spam)
        case "s": store.run(.star)
        case "S": store.toggleFilter(.starred)
        case "u": store.run(.read)
        case "U": store.toggleFilter(.unread)
        case "I": store.run(.markRead)
        case "h": store.run(.snooze)
        case "l": store.run(.label)
        case "v": store.run(.move)
        case "M": store.run(.mute)
        case "x": select()
        case "z": store.undo()
        case "r": store.reply(.reply)
        case "a": store.reply(.replyAll)
        case "f": store.reply(.forward)
        case "c": store.newMessage()
        case "/": focusSearch()
        case "?": store.shortcutsOpen = true
        case "g": goPressed = Date()
        default: return false
        }
        return true
    }

    private func open() -> Bool {
        guard store.conversation == nil, let current = store.selected else { return false }
        store.open(current)
        return true
    }

    /// Enter opens the row, and in an open thread replies to everyone.
    private func enter() -> Bool {
        guard store.conversation == nil else {
            store.reply(.replyAll)
            return true
        }
        return open()
    }

    private func escape() -> Bool {
        if let sidebarOpen, sidebarOpen.wrappedValue {
            withAnimation(.easeOut(duration: 0.2)) { sidebarOpen.wrappedValue = false }
        } else if store.conversation != nil {
            store.close()
        } else if !store.selection.isEmpty {
            store.clearSelection()
        } else if store.searchRows != nil {
            store.endSearch()
        } else if store.filter != nil {
            store.setFilter(nil)
        } else {
            return false
        }
        return true
    }

    /// `x`: the row the keyboard is on into the selection, or out of it.
    private func select() {
        guard store.conversation == nil else { return }
        if store.selected == nil { store.move(1) }
        guard let current = store.selected else { return }
        store.toggleSelection(current)
    }

    private func go(_ key: String) -> Bool {
        if key == "l" {
            store.showPalette(.labels)
            return true
        }
        guard let kind = Self.places[key] else { return false }
        store.go(to: kind)
        return true
    }
}
#endif
