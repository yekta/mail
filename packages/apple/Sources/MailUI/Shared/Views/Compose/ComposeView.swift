import SwiftUI
import UniformTypeIdentifiers

/// Writing a message: who it goes to and from, its subject, its text with the signature and the
/// quote under it, and its files. It is kept as a draft as it is written. Sending waits for the
/// undo delay, or for a time picked to send it at.
struct ComposeView: View {
    @Environment(MailStore.self) private var store
    @State private var compose: Compose
    /// Typed into the address fields and not yet made a chip.
    @State private var to = ""
    @State private var cc = ""
    @State private var bcc = ""
    /// The draft as it was last kept, to know when there is something new to keep.
    @State private var kept: Draft
    @State private var edited = false
    @State private var closing = false
    @State private var showQuote = false
        @State private var importing = false
    @State private var timing: Timing?
    @State private var pickingSnippet = false
    /// The word after a `;` at the caret, while there is one.
    @State private var trigger: String?
    @State private var triggerIndex = 0
    @State private var warning: Warning?
    @State private var discarding = false
    @State private var dropping = false
    /// A line about something that went wrong, under the title.
    @State private var notice: String?
    @State private var bodyHeight: CGFloat = 160
    @State private var editing = BodyEditing()

    fileprivate enum Timing: String, Identifiable {
        case sendLater, remind
        var id: String { rawValue }
    }

    /// Something to confirm before sending.
    fileprivate struct Warning: Identifiable {
        let id = UUID()
        let message: String
        let at: Date?
        let archive: Bool
    }

    /// An address the message can be from: one of an account's identities.
    private struct Sender: Hashable {
        let account: String
        let identity: Identity
        let first: Bool

        var key: String { account + "\n" + identity.email }
        var text: String { identity.name.map { "\($0) <\(identity.email)>" } ?? identity.email }
    }

    init(compose: Compose) {
        _compose = State(initialValue: compose)
        _kept = State(initialValue: compose.draft)
        _to = State(initialValue: compose.typing?.to ?? "")
        _cc = State(initialValue: compose.typing?.cc ?? "")
        _bcc = State(initialValue: compose.typing?.bcc ?? "")
    }

    var body: some View {
        #if os(macOS)
        VStack(spacing: 0) {
            HeaderBar(title: title) {
                IconButton(symbol: .arrowLeft, help: "Back (Esc)", action: close)
                    .keyboardShortcut(.cancelAction)
            } trailing: {
                actions
            }
            Rule(color: Tokens.cardBorder)
            page
        }
        .modifier(behaviour)
        #else
        Sheet(title: title, size: .large, close: "Cancel", background: Tokens.card) {
            page.modifier(behaviour)
        } trailing: {
            actions
        }
        #endif
    }

    private var page: some View {
        VStack(spacing: 0) {
            if let notice {
                Notice(text: notice, tone: .error)
                    .padding(.horizontal, Space.l)
                    .padding(.vertical, Space.s)
            }
            ScrollView {
                VStack(spacing: 0) {
                    fields
                    message
                }
                #if os(macOS)
                .frame(maxWidth: Theme.cardWidth)
                .background(Tokens.card.color)
                .padding(.top, Space.l)
                .padding(.horizontal, Space.xl + 4)
                .frame(maxWidth: .infinity)
                #endif
            }
            .scrollDismissesKeyboard(.interactively)
            #if os(macOS)
            .background(Tokens.background.color)
            #endif
            Rule(color: Tokens.cardBorder)
            toolbar
        }
        .background(Tokens.card.color)
    }

    /// What happens around the page on both platforms: drops, the file picker, the sheets, the
    /// questions, the autosave and the keeping of the draft when the view goes.
    private var behaviour: some ViewModifier {
        Behaviour(
            dropping: $dropping, importing: $importing, timing: $timing, pickingSnippet: $pickingSnippet, warning: $warning,
            discarding: $discarding, view: self
        )
    }

    private struct Behaviour: ViewModifier {
        @Binding var dropping: Bool
        @Binding var importing: Bool
        @Binding var timing: Timing?
        @Binding var pickingSnippet: Bool
        @Binding var warning: Warning?
        @Binding var discarding: Bool
        let view: ComposeView

        func body(content: Content) -> some View {
            content
                .overlay(alignment: .bottomLeading) { view.snippetMenu }
                .overlay {
                    if dropping {
                        RoundedRectangle(cornerRadius: Theme.radius)
                            .strokeBorder(Tokens.primary.color, style: StrokeStyle(lineWidth: 2, dash: [6, 4]))
                            .padding(Space.xs + 2)
                            .allowsHitTesting(false)
                    }
                }
                .background { view.shortcuts }
                .onDrop(of: [.item], isTargeted: $dropping) { providers in
                    Task { view.add(await Outgoing.stage(providers)) }
                    return true
                }
                .fileImporter(isPresented: $importing, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
                    guard case .success(let urls) = result else { return }
                    Task { view.add(await Outgoing.stage(urls.map(Incoming.file))) }
                }
                .sheet(item: $timing) { timing in
                    TimePicker(title: timing == .sendLater ? "Send later" : "Remind me if nobody replies") { choice in view.picked(choice, for: timing) }
                        .environment(view.store)
                }
                .sheet(isPresented: $pickingSnippet) {
                    SnippetPicker(snippets: Snippet.all(in: view.store.preferences)) { snippet in
                        pickingSnippet = false
                        view.put(snippet, replacingTrigger: false)
                    }
                }
                .alert("Send anyway?", isPresented: Binding(get: { warning != nil }, set: { if !$0 { warning = nil } }), presenting: warning) { warning in
                    Button("Send") { view.finish(at: warning.at, archive: warning.archive) }
                    Button("Keep writing", role: .cancel) {}
                } message: { warning in
                    Text(warning.message)
                }
                .alert("Discard this draft?", isPresented: $discarding) {
                    Button("Discard", role: .destructive, action: view.discard)
                    Button("Keep it", role: .cancel) {}
                }
                .task(id: view.live) {
                    view.store.noteCompose(view.live)
                    await view.autosave()
                }
                .task {
                    if !view.store.preferencesLoaded { await view.store.loadPreferences() }
                }
                .onAppear {
                    guard !view.compose.draft.to.isEmpty else { return }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { view.editing.focus() }
                }
                .onDisappear(perform: view.left)
        }
    }

    // MARK: Parts

    /// Send later, send and archive, and Send: at the end of the bar.
    private var actions: some View {
        HStack(spacing: 0) {
            IconButton(symbol: .calendarClock, help: "Send later (⌘⇧L)") { timing = .sendLater }
                .keyboardShortcut("l", modifiers: [.command, .shift])
            if compose.thread != nil {
                IconButton(symbol: .archive, help: "Send and archive (⌘⇧↩)") { send(archive: true) }
                    .keyboardShortcut(.return, modifiers: [.command, .shift])
            }
            ActionButton(title: "Send", symbol: .send, variant: .primary) { send() }
                .keyboardShortcut(.return, modifiers: .command)
                .disabled(assembled.draft.to.isEmpty && assembled.draft.cc.isEmpty && assembled.draft.bcc.isEmpty)
        }
    }

    @ViewBuilder private var fields: some View {
        if senders.count > 1 {
            row("From") {
                Dropdown(options: senders, selection: Binding(get: { sender }, set: { $0.map(choose) }), title: \.text)
                    .frame(maxWidth: 420)
                Spacer()
            }
        }
        AddressField(label: "To", addresses: $compose.draft.to, text: $to, autofocus: compose.draft.to.isEmpty) {
            if !compose.showCc {
                ActionButton(title: "Cc Bcc", variant: .ghost) { compose.showCc = true }
            }
        }
        if compose.showCc {
            AddressField(label: "Cc", addresses: $compose.draft.cc, text: $cc)
            AddressField(label: "Bcc", addresses: $compose.draft.bcc, text: $bcc)
        }
        row("Subject") {
            TextField("", text: $compose.draft.subject).textFieldStyle(.plain)
        }
    }

    private var message: some View {
        VStack(alignment: .leading, spacing: Space.l) {
            BodyEditor(
                text: $compose.draft.text, height: $bodyHeight, editing: editing, menuOpen: !triggerMatches.isEmpty,
                onTrigger: { word in
                    trigger = word
                    triggerIndex = 0
                },
                onKey: triggerKey,
                onPaste: { incoming in Task { add(await Outgoing.stage(incoming)) } }
            )
            .frame(height: max(bodyHeight, 160))
            if let signature {
                Text(signature).textStyle(.body, color: Tokens.mutedMoreForeground.color).textSelection(.enabled)
            }
            if let quote = compose.draft.quote, !quote.isEmpty {
                Chip(title: "•••", help: showQuote ? "Hide the quoted text" : "Show the quoted text", action: { showQuote.toggle() })
                if showQuote {
                    Text(quote)
                        .textStyle(.label, color: Tokens.mutedMoreForeground.color)
                        .textSelection(.enabled)
                        .padding(.leading, Space.m)
                        .overlay(alignment: .leading) { Rectangle().fill(Tokens.cardBorder.color).frame(width: 2) }
                }
            }
            if !compose.draft.attachments.isEmpty || compose.remindAt != nil {
                FlowLayout {
                    ForEach(Array(compose.draft.attachments.enumerated()), id: \.offset) { index, file in
                        Chip(
                            title: file.name, symbol: .paperclip, detail: ByteCountFormatter.string(fromByteCount: file.size, countStyle: .file),
                            remove: { compose.draft.attachments.remove(at: index) }
                        )
                    }
                    if let remind = compose.remindAt {
                        Chip(title: "Remind me \(remind.label) if nobody replies", symbol: .alarmClock, remove: { compose.remindAt = nil })
                    }
                }
            }
        }
        .padding(.horizontal, Space.l)
        .padding(.vertical, Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var toolbar: some View {
        HStack(spacing: 0) {
            IconButton(symbol: .paperclip, help: "Attach files (⌘⇧U)") { importing = true }
                .keyboardShortcut("u", modifiers: [.command, .shift])
            IconButton(symbol: .zap, help: "Snippets (⌘;), or type ; and a name") { pickingSnippet = true }
                .keyboardShortcut(";", modifiers: .command)
            IconButton(symbol: .alarmClock, help: "Remind me if nobody replies (⌘⇧H)", active: compose.remindAt != nil) { timing = .remind }
            .keyboardShortcut("h", modifiers: [.command, .shift])
            if compose.draft.inReplyTo != nil {
                IconButton(symbol: .handshake, help: "Instant Intro: move who introduced you to Bcc (⌘⇧I)", action: instantIntro)
                    .keyboardShortcut("i", modifiers: [.command, .shift])
            }
            Spacer()
            IconButton(symbol: .trash, help: "Discard the draft (⌘⇧,)") { discarding = true }
                .keyboardShortcut(",", modifiers: [.command, .shift])
        }
        .padding(.horizontal, Space.l)
        .frame(height: Theme.barHeight)
    }

    /// The shortcuts that have no button of their own.
    fileprivate var shortcuts: some View {
        Group {
            Button("Close") { close() }.keyboardShortcut("w")
        }
        .opacity(0)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    /// The snippets matching what follows a `;`, over the toolbar.
    @ViewBuilder fileprivate var snippetMenu: some View {
        let matches = triggerMatches
        if !matches.isEmpty {
            PopupCard {
                ForEach(Array(matches.enumerated()), id: \.element.id) { index, snippet in
                    ChoiceRow(title: snippet.name, detail: snippet.text.replacingOccurrences(of: "\n", with: " "), symbol: .zap, highlighted: index == triggerIndex) {
                        put(snippet, replacingTrigger: true)
                    }
                }
            }
            .frame(maxWidth: 380)
            .padding(.leading, Space.l)
            .padding(.bottom, Theme.barHeight + Space.xs)
        }
    }

    private func row(_ label: String, @ViewBuilder content: () -> some View) -> some View {
        VStack(spacing: 0) {
            HStack(spacing: Space.s + 2) {
                Text(label).textStyle(.label, color: Tokens.mutedMoreForeground.color).frame(width: 56, alignment: .leading)
                content()
            }
            .font(.ui(.body))
            .foregroundStyle(Tokens.foreground.color)
            .padding(.horizontal, Space.l)
            .frame(height: Theme.searchHeight * Platform.scale)
            Rule(color: Tokens.cardBorder).padding(.leading, Space.l)
        }
    }

    // MARK: State

    private var title: String {
        guard compose.draft.inReplyTo == nil else { return "Reply" }
        return compose.draft.forwardAttachmentsOf != nil || compose.draft.subject.hasPrefix("Fwd:") ? "Forward" : "New message"
    }

    /// The compose as it is, with what is typed in the address fields, for the store to keep.
    fileprivate var live: Compose {
        var current = compose
        current.typing = Compose.Typing(to: to, cc: cc, bcc: bcc)
        return current
    }

    /// The compose with what is typed in the address fields made addresses.
    fileprivate var assembled: Compose {
        var outgoing = compose
        outgoing.draft.to += Address.typed(to)
        outgoing.draft.cc += Address.typed(cc)
        outgoing.draft.bcc += Address.typed(bcc)
        return outgoing
    }

    private var senders: [Sender] {
        store.accounts.flatMap { account in
            let identities = account.identities.isEmpty ? [Identity(name: nil, email: account.address, signature: nil)] : account.identities
            return identities.enumerated().map { index, identity in Sender(account: account.id, identity: identity, first: index == 0) }
        }
    }

    private var sender: Sender? {
        senders.first { option in
            guard option.account == compose.draft.accountId else { return false }
            guard let from = compose.draft.from else { return option.first }
            return option.identity.email.caseInsensitiveCompare(from.email) == .orderedSame
        }
    }

    private var signature: String? {
        store.signature(account: compose.draft.accountId, email: sender?.identity.email)
    }

    private var triggerMatches: [Snippet] {
        guard let trigger else { return [] }
        let all = Snippet.all(in: store.preferences)
        guard !trigger.isEmpty else { return all }
        return all.filter { $0.name.localizedCaseInsensitiveContains(trigger) }
    }

    // MARK: Doing

    private func choose(_ sender: Sender) {
        compose.draft.accountId = sender.account
        compose.draft.from = sender.first ? nil : Address(name: sender.identity.name, email: sender.identity.email)
        compose.from = sender.identity.email
    }

    private func triggerKey(_ key: EditorKey) -> Bool {
        let matches = triggerMatches
        guard !matches.isEmpty else { return false }
        switch key {
        case .up: triggerIndex = max(triggerIndex - 1, 0)
        case .down: triggerIndex = min(triggerIndex + 1, matches.count - 1)
        case .accept: put(matches[min(triggerIndex, matches.count - 1)], replacingTrigger: true)
        case .cancel: trigger = nil
        }
        return true
    }

    /// Puts a snippet in at the caret, with the first recipient's first name.
    fileprivate func put(_ snippet: Snippet, replacingTrigger: Bool) {
        let text = snippet.filled(firstName: assembled.draft.to.first?.firstName)
        trigger = nil
        guard replacingTrigger else {
            DispatchQueue.main.async {
                editing.focus()
                editing.insert(text)
            }
            return
        }
        editing.replaceTrigger(with: text)
    }

    /// Moves whoever made the introduction to Bcc and thanks them first thing.
    private func instantIntro() {
        var draft = assembled.draft
        guard !draft.to.isEmpty else { return }
        let introducer = draft.to.removeFirst()
        draft.to += draft.cc
        draft.cc = []
        draft.bcc.append(introducer)
        let thanks = introducer.firstName.map { "Thanks for the intro, \($0)! Moving you to Bcc." } ?? "Thanks for the intro! Moving you to Bcc."
        draft.text = thanks + "\n\n" + draft.text
        compose.draft = draft
        compose.showCc = true
        (to, cc, bcc) = ("", "", "")
    }

    fileprivate func add(_ files: [DraftAttachment]) {
        var total = compose.draft.attachments.reduce(Int64(0)) { $0 + $1.size }
        for file in files {
            guard total + file.size <= Outgoing.limit else {
                notice = "\(file.name) wasn't added: attachments can be 25 MB together at most."
                continue
            }
            total += file.size
            compose.draft.attachments.append(file)
        }
    }

    fileprivate func picked(_ choice: TimeChoice, for timing: Timing) {
        self.timing = nil
        guard timing == .sendLater else {
            compose.remindAt = choice
            return
        }
        // After the picker's sheet has gone, so a question before sending can be shown.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) { send(at: choice.date) }
    }

    fileprivate func autosave() async {
        let snapshot = assembled
        guard snapshot.draft != kept else { return }
        store.noteCompose(snapshot)
        edited = true
        try? await Task.sleep(for: .seconds(2))
        guard !Task.isCancelled, !closing else { return }
        kept = snapshot.draft
        compose.draftId = await store.saveDraft(snapshot)
    }

    private func close() {
        closing = true
        store.closeCompose(assembled, keep: edited || compose.keep || compose.draftId != nil)
    }

    fileprivate func discard() {
        closing = true
        store.discardDraft(compose)
    }

    /// Gone without Send or Back: swiped away on iOS. Kept as when closed.
    fileprivate func left() {
        guard !closing, timing == nil, !pickingSnippet, !importing else { return }
        store.closeCompose(assembled, keep: edited || compose.keep || compose.draftId != nil)
    }

    /// Checks the message, asks about what looks forgotten, then sends.
    private func send(at date: Date? = nil, archive: Bool = false) {
        let draft = assembled.draft
        let everyone = draft.to + draft.cc + draft.bcc
        guard !everyone.isEmpty else {
            notice = "Add who it goes to."
            return
        }
        if let wrong = everyone.first(where: { !$0.isValid }) {
            notice = "“\(wrong.email)” isn't an address."
            return
        }
        notice = nil
        var worries: [String] = []
        let placeholders = Outgoing.placeholders(in: draft.text)
        if !placeholders.isEmpty {
            worries.append("\(placeholders.joined(separator: ", ")) still \(placeholders.count == 1 ? "needs" : "need") filling in.")
        }
        if draft.attachments.isEmpty, draft.forwardAttachmentsOf == nil, Outgoing.mentionsAttachment(draft.text) {
            worries.append("It mentions an attachment, but nothing is attached.")
        }
        guard worries.isEmpty else {
            warning = Warning(message: worries.joined(separator: " "), at: date, archive: archive)
            return
        }
        finish(at: date, archive: archive)
    }

    fileprivate func finish(at date: Date?, archive: Bool) {
        closing = true
        store.send(assembled, at: date, archive: archive)
    }
}
