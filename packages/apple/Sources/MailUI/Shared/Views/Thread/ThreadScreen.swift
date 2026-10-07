import SwiftUI

/// A thread: its subject, who is in it, and its messages stacked as cards; the earlier ones
/// folded to a line, as Newton showed them.
struct ThreadScreen: View {
    @Environment(MailStore.self) private var store
    let conversation: Conversation
    @State private var unfolded: Set<String> = []

    var body: some View {
        GeometryReader { window in
            ScrollView {
                page
                    #if os(macOS)
                    .padding(.top, 16)
                    .padding(.horizontal, 24)
                    #endif
                    .frame(maxWidth: .infinity, minHeight: window.size.height, alignment: .top)
                    #if os(macOS)
                    // The page around the card: a click there closes the thread, as Newton's did.
                    .background { Tokens.background.color.onTapGesture(perform: store.close) }
                    #endif
            }
        }
        .background(Tokens.background.color)
        .id(conversation.id)
    }

    private var page: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            ForEach(conversation.messages) { message in
                let folded = message.folded && !unfolded.contains(message.id)
                MessageCard(message: message, folded: folded)
                    .contentShape(Rectangle())
                    .onTapGesture { if folded { unfolded.insert(message.id) } }
                if message.id != conversation.messages.last?.id {
                    Rectangle().fill(Tokens.border.color).frame(height: 1)
                }
            }
            if conversation.messages.contains(where: \.blockedImages) {
                ActionButton(title: "Load images", symbol: .image, variant: .ghost, action: store.showImages)
                    .padding(.top, 8)
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
        .padding(.bottom, 20)
    }

    private var replies: some View {
        HStack(spacing: 10) {
            IconButton(symbol: .replyAll, help: "Reply all") { store.reply(.replyAll, to: conversation.id) }
            IconButton(symbol: .reply, help: "Reply") { store.reply(.reply, to: conversation.id) }
            IconButton(symbol: .forward, help: "Forward") { store.reply(.forward, to: conversation.id) }
        }
        .padding(.top, 22)
    }
}

/// One message of a thread.
struct MessageCard: View {
    let message: MessageItem
    let folded: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .center, spacing: 12) {
                Avatar(initials: message.initials, email: message.fromEmail, size: 34)
                VStack(alignment: .leading, spacing: 2) {
                    Text(message.fromName)
                        .font(.ui(14, folded ? .regular : .semibold))
                        .foregroundStyle(folded ? Tokens.mutedForeground.color : Tokens.foreground.color)
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
        HStack(spacing: 8) {
            ForEach(message.attachments, id: \.self) { attachment in
                HStack(spacing: 6) {
                    Image(.paperclip, size: 12)
                    Text(attachment.name).font(.ui(12)).lineLimit(1)
                    Text(ByteCountFormatter.string(fromByteCount: attachment.size, countStyle: .file))
                        .font(.ui(11)).foregroundStyle(Tokens.mutedForeground.color)
                }
                .padding(.horizontal, 10)
                .frame(height: 26)
                .background(Capsule().fill(Tokens.muted.color))
            }
        }
    }
}

/// What can be done to the open thread: archive, trash, snooze, mark unread, spam.
struct ThreadActions: View {
    @Environment(MailStore.self) private var store
    let thread: String

    var body: some View {
        HStack(spacing: 8) {
            IconButton(symbol: .archive, help: "Archive (e)") { store.act(.archive, on: [thread]) }
            IconButton(symbol: .trash, help: "Trash (#)") { store.act(.trash, on: [thread]) }
            IconButton(symbol: .clock, help: "Snooze (h)") { store.snoozing = [thread] }
            IconButton(symbol: .mail, help: "Mark unread (u)") { store.toggleRead(thread) }
            IconButton(symbol: .shieldAlert, help: "Spam") { store.act(.spam, on: [thread]) }
        }
    }
}

/// When a snooze ends.
struct SnoozePicker: View {
    @Environment(MailStore.self) private var store
    let threads: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Snooze until").font(.ui(13, .semibold)).foregroundStyle(Tokens.foreground.color).padding(.bottom, 6)
            ForEach(SnoozeChoice.choices()) { choice in
                Button {
                    store.snoozing = nil
                    store.act(.snooze, on: threads, until: choice.until)
                } label: {
                    HStack {
                        Text(choice.name).font(.ui(14)).foregroundStyle(Tokens.foreground.color)
                        Spacer()
                        Text(choice.until.formatted(.dateTime.weekday(.abbreviated).hour().minute()))
                            .font(.ui(12)).foregroundStyle(Tokens.mutedForeground.color)
                    }
                    .padding(.vertical, 8)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            ActionButton(title: "Cancel", variant: .ghost) { store.snoozing = nil }.padding(.top, 6)
        }
        .padding(20)
        .frame(minWidth: 280)
    }
}
