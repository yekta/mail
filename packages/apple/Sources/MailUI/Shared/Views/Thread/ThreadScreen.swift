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
                    Rectangle().fill(Tokens.border.color).frame(width: 1)
                    PersonView(email: email, openThread: store.open)
                        .overlay(alignment: .topTrailing) {
                            IconButton(symbol: .x, help: "Close", circled: false) { paneEmail = nil }.padding(12)
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
            #if os(macOS)
            PersonView(email: sheetEmail ?? "", closeSheet: { sheetEmail = nil }) { thread in
                sheetEmail = nil
                store.open(thread)
            }
            .environment(store)
            .frame(width: 340, height: 460)
            #else
            PersonView(email: sheetEmail ?? "", closeSheet: { sheetEmail = nil }) { thread in
                sheetEmail = nil
                store.show(thread: thread)
            }
            .environment(store)
            .presentationDetents([.medium, .large])
            #endif
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
                    .padding(.top, 16)
                    .padding(.horizontal, 24)
                    #endif
                    .frame(maxWidth: .infinity, minHeight: height, alignment: .top)
                    #if os(macOS)
                    // The page around the card: a click there closes the thread, as Newton's did.
                    .background { Tokens.background.color.onTapGesture(perform: store.close) }
                    #endif
            }
            .onAppear {
                // The message it was left on, else the first unread one.
                let kept = store.focusedMessage.flatMap { id in conversation.messages.contains { $0.id == id } ? id : nil }
                let target = kept ?? conversation.messages.first(where: \.unread)?.id
                store.focusedMessage = target
                guard let target else { return }
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
                    Rectangle().fill(Tokens.border.color).frame(height: 1)
                }
            }
            if let draft = conversation.draftId {
                draftCard(draft)
            }
            replies
        }
        .padding(.horizontal, pagePadding)
        .padding(.vertical, 28)
        .frame(maxWidth: Theme.cardWidth)
        .background(Tokens.card.color)
    }

    private var pagePadding: CGFloat {
        #if os(macOS)
        80
        #else
        16
        #endif
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(conversation.subject)
                        .font(.ui(24, .semibold))
                        .foregroundStyle(Tokens.foreground.color)
                        .textSelection(.enabled)
                    Text(conversation.participants)
                        .font(.ui(13))
                        .foregroundStyle(Tokens.mutedForeground.color)
                }
                Spacer(minLength: 0)
                Button {
                    store.toggleStar(conversation.id)
                } label: {
                    Image(conversation.starred ? .starFilled : .star, size: 20)
                        .foregroundStyle(conversation.starred ? Tokens.star.color : Tokens.mutedForeground.color)
                }
                .buttonStyle(.plain)
                .help(conversation.starred ? "Unstar" : "Star")
            }
            if !conversation.labels.isEmpty || conversation.muted || conversation.unsubscribe {
                HStack(alignment: .center, spacing: 12) {
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
                HStack(spacing: 8) {
                    Text("Images from the sender were left out.").font(.ui(13)).foregroundStyle(Tokens.mutedForeground.color)
                    ActionButton(title: "Load images", symbol: .image, variant: .ghost, action: store.showImages)
                }
            }
        }
        .padding(.bottom, 20)
    }

    /// The reply the user started and didn't send.
    private func draftCard(_ id: String) -> some View {
        VStack(spacing: 0) {
            Rectangle().fill(Tokens.border.color).frame(height: 1)
            HStack(spacing: 12) {
                Image(.pencil, size: 14)
                Text("Draft").font(.ui(14, .semibold))
                Text("A reply you started").font(.ui(13)).foregroundStyle(Tokens.mutedForeground.color)
                Spacer(minLength: 8)
                ActionButton(title: "Open", variant: .outline) { store.openDraft(id) }
            }
            .foregroundStyle(Tokens.destructive.color)
            .padding(.vertical, 14)
            .contentShape(Rectangle())
            .onTapGesture { store.openDraft(id) }
        }
    }

    private var replies: some View {
        HStack(spacing: 10) {
            IconButton(symbol: .replyAll, help: "Reply all") { store.reply(.replyAll, to: conversation.id) }
            IconButton(symbol: .reply, help: "Reply") { store.reply(.reply, to: conversation.id) }
            IconButton(symbol: .forward, help: "Forward") { store.reply(.forward, to: conversation.id) }
        }
        .padding(.top, 22)
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
    let message: MessageItem
    let folded: Bool
    let showPerson: () -> Void
    let open: (Int) -> Void
    let save: (Int) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .center, spacing: 12) {
                Avatar(initials: message.initials, email: message.fromEmail, size: 34)
                    .onTapGesture { if !folded { showPerson() } }
                VStack(alignment: .leading, spacing: 2) {
                    Text(message.fromName)
                        .font(.ui(14, folded ? .regular : .semibold))
                        .foregroundStyle(folded ? Tokens.mutedForeground.color : Tokens.foreground.color)
                        .onTapGesture { if !folded { showPerson() } }
                        .help(folded ? "" : message.fromEmail)
                    Text(folded ? message.snippet : message.to)
                        .font(.ui(12.5))
                        .foregroundStyle(Tokens.mutedForeground.color)
                        .lineLimit(1)
                }
                Spacer(minLength: 8)
                if !message.attachments.isEmpty {
                    Image(.paperclip, size: 13).foregroundStyle(Tokens.mutedForeground.color)
                }
                Text(message.date).font(.ui(12)).foregroundStyle(Tokens.mutedForeground.color)
            }
            if !folded {
                Group {
                    if let html = message.html {
                        MessageWebView(html: html)
                    } else if message.failed {
                        Text("This message couldn't be loaded. Open the conversation again to retry.")
                            .font(.ui(14))
                            .foregroundStyle(Tokens.mutedForeground.color)
                    } else {
                        Text(message.snippet)
                            .font(.ui(14))
                            .foregroundStyle(Tokens.mutedForeground.color)
                            .redacted(reason: .placeholder)
                    }
                }
                .padding(.leading, 46 * Platform.scale)
                if !message.attachments.isEmpty {
                    attachments.padding(.leading, 46 * Platform.scale)
                }
            }
        }
        .padding(.vertical, 16)
    }

    private var attachments: some View {
        FlowLayout(spacing: 8) {
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
