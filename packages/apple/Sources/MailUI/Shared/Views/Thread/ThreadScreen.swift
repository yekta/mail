import QuickLook
import SwiftUI

/// A thread: its subject, its labels, who is in it, and its messages stacked as cards; the
/// earlier ones folded to a line, as Newton showed them. It opens on the first unread message.
/// On a wide Mac window a sender clicked is shown beside it.
struct ThreadScreen: View {
    @Environment(MailStore.self) private var store
    let conversation: Conversation
    /// A sender clicked, shown in the pane beside the thread.
    @State private var paneEmail: String?
    @State private var sheetEmail: String?
    @State private var preview: URL?
    /// A copy of an attachment, while the user picks where to save it.
    @State private var saving: URL?

    var body: some View {
        GeometryReader { window in
            HStack(spacing: 0) {
                thread(height: window.size.height)
                if showsPane(width: window.size.width), let email = paneEmail {
                    Rule(vertical: true)
                    PersonView(email: email, openThread: store.open)
                        .overlay(alignment: .topTrailing) {
                            IconButton(symbol: .x, help: "Close", circled: false) { paneEmail = nil }.padding(Space.m)
                        }
                        .frame(width: 280)
                }
            }
            .onChange(of: sheetEmail) { _, email in
                // Where the pane is shown, a sender opens in it rather than in a sheet.
                guard let email, showsPane(width: window.size.width) else { return }
                paneEmail = email
                sheetEmail = nil
            }
        }
        .background(Tokens.background.color)
        .quickLookPreview($preview)
        .fileMover(isPresented: Binding(get: { saving != nil }, set: { if !$0 { saving = nil } }), file: saving) { result in
            guard case .failure(let error) = result else { return }
            store.show(error.localizedDescription)
        }
        .sheet(isPresented: Binding(get: { sheetEmail != nil }, set: { if !$0 { sheetEmail = nil } })) {
            Sheet(title: "Contact", background: Tokens.card) {
                PersonView(email: sheetEmail ?? "", closeSheet: { sheetEmail = nil }) { thread in
                    sheetEmail = nil
                    #if os(macOS)
                    store.open(thread)
                    #else
                    store.show(thread: thread)
                    #endif
                }
            }
            .environment(store)
        }
        .id(conversation.id)
    }

    private func showsPane(width: CGFloat) -> Bool {
        #if os(macOS)
        width >= 1100
        #else
        false
        #endif
    }

    private func thread(height: CGFloat) -> some View {
        ScrollViewReader { proxy in
            ScrollView {
                page
                    #if os(macOS)
                    .padding(.top, Space.l)
                    .padding(.horizontal, Space.xl + 4)
                    #endif
                    .frame(maxWidth: .infinity, minHeight: height, alignment: .top)
                    #if os(macOS)
                    // The page around the card: a click there closes the thread, as Newton's did.
                    .background { Tokens.background.color.onTapGesture(perform: store.close) }
                    #endif
                    .background(ScrollKept(restore: store.threadOffset, report: store.noteThreadScroll))
            }
            .onAppear {
                // The message it was left on, else the first unread one; where it was scrolled to
                // wins over both.
                let kept = store.focusedMessage.flatMap { id in conversation.messages.contains { $0.id == id } ? id : nil }
                let target = kept ?? conversation.messages.first(where: \.unread)?.id
                store.focusedMessage = target
                guard let target, store.threadOffset <= 0 else { return }
                DispatchQueue.main.async { proxy.scrollTo(target, anchor: .top) }
            }
            .onChange(of: store.focusedMessage) { _, id in
                guard let id else { return }
                withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo(id, anchor: .top) }
            }
        }
    }

    private var page: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            ForEach(conversation.messages) { message in
                let folded = message.folded && !store.unfoldedMessages.contains(message.id)
                MessageCard(
                    message: message, folded: folded,
                    showPerson: { sheetEmail = message.fromEmail },
                    open: { index in open(message, index) },
                    save: { index in save(message, index) }
                )
                .contentShape(Rectangle())
                .onTapGesture { if folded { store.unfoldedMessages.insert(message.id) } }
                .id(message.id)
                if message.id != conversation.messages.last?.id {
                    Rule(color: Tokens.cardBorder)
                }
            }
            if let draft = conversation.draftId {
                draftCard(draft)
            }
            replies
        }
        .padding(.horizontal, pagePadding)
        .padding(.vertical, Space.xxl)
        .frame(maxWidth: Theme.cardWidth)
        .background(Tokens.card.color)
    }

    private var pagePadding: CGFloat {
        #if os(macOS)
        80
        #else
        Space.l
        #endif
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: Space.s + 2) {
            HStack(alignment: .top, spacing: Space.m) {
                VStack(alignment: .leading, spacing: Space.xs + 2) {
                    Text(conversation.subject).textStyle(.title).textSelection(.enabled)
                    Text(conversation.participants).textStyle(.label, color: Tokens.mutedMoreForeground.color)
                }
                Spacer(minLength: 0)
                StarButton(starred: conversation.starred) { store.toggleStar(conversation.id) }
            }
            if !conversation.labels.isEmpty || conversation.muted || conversation.unsubscribe {
                HStack(alignment: .center, spacing: Space.m) {
                    FlowLayout {
                        ForEach(conversation.labels) { label in Chip(title: label.name, symbol: .tag) }
                        if conversation.muted {
                            Chip(title: "Muted", symbol: .bellOff, help: "New mail in this thread stays out of the inbox")
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    if conversation.unsubscribe {
                        ActionButton(title: "Unsubscribe", symbol: .mailX, variant: .ghost) { store.act(.unsubscribe, on: [conversation.id]) }
                    }
                }
            }
            if conversation.messages.contains(where: \.blockedImages) {
                Notice(text: "Images from the sender were left out.", action: "Load images", perform: store.showImages)
            }
        }
        .padding(.bottom, Space.xl)
    }

    /// The reply the user started and didn't send.
    private func draftCard(_ id: String) -> some View {
        VStack(spacing: 0) {
            Rule(color: Tokens.cardBorder)
            HStack(spacing: Space.m) {
                Image(.pencil, size: 14).foregroundStyle(Tokens.destructive.color)
                Text("Draft").textStyle(.subheading, color: Tokens.destructive.color)
                Text("A reply you started").textStyle(.label, color: Tokens.mutedMoreForeground.color)
                Spacer(minLength: Space.s)
                ActionButton(title: "Open", variant: .outline) { store.openDraft(id) }
            }
            .padding(.vertical, Space.l)
            .contentShape(Rectangle())
            .onTapGesture { store.openDraft(id) }
        }
    }

    private var replies: some View {
        HStack(spacing: 0) {
            IconButton(symbol: .replyAll, help: "Reply all") { store.reply(.replyAll, to: conversation.id) }
            IconButton(symbol: .reply, help: "Reply") { store.reply(.reply, to: conversation.id) }
            IconButton(symbol: .forward, help: "Forward") { store.reply(.forward, to: conversation.id) }
        }
        .padding(.top, Space.xl + 2)
    }

    private func open(_ message: MessageItem, _ index: Int) {
        Task {
            guard let file = await store.openAttachment(message: message.id, index: index) else { return }
            preview = file
        }
    }

    private func save(_ message: MessageItem, _ index: Int) {
        let name = message.attachments[index].name
        Task {
            guard let file = await store.openAttachment(message: message.id, index: index) else { return }
            do {
                saving = try await Self.copy(file, named: name)
            } catch {
                store.show(error.localizedDescription)
            }
        }
    }

    /// A copy to hand to the save panel, which moves it: the downloaded file stays where it is.
    private static func copy(_ file: URL, named name: String) async throws -> URL {
        try await Task.detached(priority: .userInitiated) {
            let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            let copy = folder.appendingPathComponent(name.isEmpty ? file.lastPathComponent : name)
            try FileManager.default.copyItem(at: file, to: copy)
            return copy
        }.value
    }
}

/// One message of a thread. Its sender opens the person; its files open in Quick Look.
struct MessageCard: View {
    @Environment(MailStore.self) private var store
    @Environment(\.colorScheme) private var colorScheme
    let message: MessageItem
    let folded: Bool
    let showPerson: () -> Void
    let open: (Int) -> Void
    let save: (Int) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            HStack(alignment: .center, spacing: Space.m) {
                Avatar(initials: message.initials, email: message.fromEmail, size: 34)
                    .onTapGesture(perform: tapSender)
                VStack(alignment: .leading, spacing: 2) {
                    Text(message.fromName)
                        .textStyle(folded ? .body : .subheading, color: folded ? Tokens.mutedMoreForeground.color : Tokens.foreground.color)
                        .onTapGesture(perform: tapSender)
                        .help(folded ? "" : message.fromEmail)
                    Text(folded ? message.snippet : message.to).textStyle(.caption).lineLimit(1)
                }
                Spacer(minLength: Space.s)
                if !message.attachments.isEmpty {
                    Image(.paperclip, size: 13).foregroundStyle(Tokens.mutedMoreForeground.color)
                }
                Text(message.date).textStyle(.caption)
                if canShowOriginal {
                    IconButton(symbol: original ? .moon : .sun, help: original ? "Show in dark colours" : "Show the original", circled: false, quiet: true) {
                        store.toggleOriginal(message.id)
                    }
                }
            }
            if !folded {
                Group {
                    if let html = message.html {
                        MessageWebView(html: html, original: original)
                    } else if message.failed {
                        Text("This message couldn't be loaded. Open the conversation again to retry.")
                            .textStyle(.body, color: Tokens.mutedMoreForeground.color)
                    } else {
                        Text(message.snippet).textStyle(.body, color: Tokens.mutedMoreForeground.color).redacted(reason: .placeholder)
                    }
                }
                .padding(.leading, bodyIndent)
                if !message.attachments.isEmpty {
                    attachments.padding(.leading, bodyIndent)
                }
            }
        }
        .padding(.vertical, Space.l)
    }

    /// On the Mac the body lines up with the name beside the avatar; a phone has no width to spare.
    private var bodyIndent: CGFloat {
        #if os(macOS)
        46
        #else
        0
        #endif
    }

    /// A sender's own tap gesture takes the click from the card's, so a folded message unfolds here too.
    private func tapSender() {
        guard folded else { return showPerson() }
        store.unfoldedMessages.insert(message.id)
    }

    private var original: Bool { store.originalMessages.contains(message.id) }

    /// Designed mail, turned dark, can be shown as its sender made it.
    private var canShowOriginal: Bool {
        !folded && message.designed && message.html != nil && colorScheme == .dark && store.darkMail
    }

    private var attachments: some View {
        FlowLayout(spacing: Space.s) {
            ForEach(Array(message.attachments.enumerated()), id: \.offset) { index, attachment in
                Chip(
                    title: attachment.name, symbol: .paperclip,
                    detail: ByteCountFormatter.string(fromByteCount: attachment.size, countStyle: .file),
                    pending: store.downloading.contains("\(message.id)/\(index)"), help: "Open \(attachment.name)",
                    action: { open(index) }
                )
                .contextMenu {
                    Button("Open") { open(index) }
                    Button("Save…") { save(index) }
                }
            }
        }
    }
}
