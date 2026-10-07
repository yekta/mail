import Foundation

/// What the core sends, as `crates/core/src/api.rs` defines it.

struct ThreadRow: Codable, Identifiable, Hashable {
    let id: String
    let accountId: String
    let color: String
    let senders: String
    let subject: String
    let snippet: String
    let date: String
    let timestamp: Int64
    let unread: Bool
    let starred: Bool
    let attachment: Bool
    let snoozed: Bool
}

struct Mailbox: Codable, Identifiable, Hashable {
    let id: String
    let name: String
    let symbol: String
    let unread: Int
}

struct AccountView: Codable, Identifiable, Hashable {
    let id: String
    let address: String
    let provider: String
    let color: String
    let status: String
    let mailboxes: [Mailbox]
}

struct Mailboxes: Codable {
    let unified: [Mailbox]
    let accounts: [AccountView]
}

struct ThreadPage: Codable {
    let rows: [ThreadRow]
    let total: Int
}

struct Attachment: Codable, Hashable {
    let name: String
    let mime: String
    let size: Int64
}

struct MessageItem: Codable, Identifiable, Hashable {
    let id: String
    let fromName: String
    let fromEmail: String
    let initials: String
    let to: String
    let date: String
    let snippet: String
    let unread: Bool
    let folded: Bool
    let html: String?
    let blockedImages: Bool
    let attachments: [Attachment]
}

struct Conversation: Codable, Identifiable, Hashable {
    let id: String
    let accountId: String
    let color: String
    let subject: String
    let participants: String
    let starred: Bool
    let unread: Bool
    let messages: [MessageItem]
}

struct Address: Codable, Hashable {
    var name: String?
    var email: String

    /// Reads "Ann <ann@example.com>" or a bare address.
    static func parse(_ text: String) -> Address? {
        let text = text.trimmingCharacters(in: .whitespaces)
        if let open = text.lastIndex(of: "<"), let close = text.lastIndex(of: ">"), open < close {
            let email = text[text.index(after: open)..<close].trimmingCharacters(in: .whitespaces)
            let name = text[..<open].trimmingCharacters(in: CharacterSet(charactersIn: " \""))
            return email.contains("@") ? Address(name: name.isEmpty ? nil : name, email: email) : nil
        }
        return text.contains("@") && !text.contains(" ") ? Address(name: nil, email: text) : nil
    }

    static func list(_ text: String) -> [Address] {
        text.split(whereSeparator: { $0 == "," || $0 == ";" }).compactMap { parse(String($0)) }
    }

    var text: String {
        guard let name else { return email }
        return "\(name) <\(email)>"
    }
}

struct Draft: Codable, Hashable {
    var accountId: String
    var to: [Address]
    var cc: [Address] = []
    var bcc: [Address] = []
    var subject: String
    var text: String
    var inReplyTo: String?
    var references: [String] = []
    var threadId: String?
}

struct DraftReply: Codable {
    let draft: Draft
    let from: String
}

struct StatusAccount: Codable, Hashable {
    let id: String
    let address: String
    let provider: String
    let status: String
    let color: String
}

struct Status: Codable {
    let signedIn: Bool
    let server: String?
    let connection: String
    let accounts: [StatusAccount]
}

struct SendReply: Codable {
    let opId: String
    let sendAt: Int64
}

struct CancelReply: Codable {
    let draft: Draft
}

struct SearchReply: Codable {
    let request: UInt64
    let rows: [ThreadRow]
}

struct URLReply: Codable {
    let url: String
}

struct Empty: Codable {}

enum CoreEvent: Decodable {
    case changed(mailboxes: Bool, threads: [String])
    case connection(state: String, error: String?)
    case sent(opId: String)
    case sendFailed(opId: String, error: String, draft: Draft)
    case searchResults(request: UInt64, rows: [ThreadRow])
    case error(message: String)
    case other

    private enum Keys: String, CodingKey {
        case type, mailboxes, threads, state, error, opId, draft, request, rows, message
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        switch try values.decode(String.self, forKey: .type) {
        case "changed":
            self = .changed(mailboxes: try values.decode(Bool.self, forKey: .mailboxes), threads: try values.decode([String].self, forKey: .threads))
        case "connection":
            self = .connection(state: try values.decode(String.self, forKey: .state), error: try values.decodeIfPresent(String.self, forKey: .error))
        case "sent":
            self = .sent(opId: try values.decode(String.self, forKey: .opId))
        case "send_failed":
            self = .sendFailed(
                opId: try values.decode(String.self, forKey: .opId), error: try values.decode(String.self, forKey: .error),
                draft: try values.decode(Draft.self, forKey: .draft)
            )
        case "search_results":
            self = .searchResults(request: try values.decode(UInt64.self, forKey: .request), rows: try values.decode([ThreadRow].self, forKey: .rows))
        case "error":
            self = .error(message: try values.decode(String.self, forKey: .message))
        default:
            self = .other
        }
    }
}

/// What can be done to threads, as the core names it.
enum Action: String {
    case archive, trash, read, unread, star, unstar, inbox, spam, snooze
}

enum ReplyKind: String {
    case reply
    case replyAll = "reply_all"
    case forward
}
