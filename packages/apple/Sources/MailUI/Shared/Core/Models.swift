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
    /// The row is a saved draft: open it with `open_draft`.
    var draftId: String?
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
    let identities: [Identity]
    let mailboxes: [Mailbox]
}

struct Identity: Codable, Hashable {
    let name: String?
    let email: String
    let signature: String?
}

struct Mailboxes: Codable {
    let unified: [Mailbox]
    let accounts: [AccountView]
}

struct ThreadPage: Codable {
    let rows: [ThreadRow]
    let total: Int
    let splits: [SplitTab]
}

/// A tab over a split inbox.
struct SplitTab: Codable, Hashable, Identifiable {
    let mailbox: String
    let name: String
    let unread: Int
    let total: Int

    var id: String { mailbox }
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
    let failed: Bool
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
    let muted: Bool
    let labels: [LabelChip]
    /// Whether the unsubscribe action can do something.
    let unsubscribe: Bool
    /// The saved reply draft of this thread.
    let draftId: String?
    /// The person the thread is with, for the contact pane.
    let person: String?
    let messages: [MessageItem]
}

struct LabelChip: Codable, Hashable, Identifiable {
    let id: String
    let name: String
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
    /// One of the account's identities; none for its own address.
    var from: Address?
    var html: String?
    /// The quoted text under a reply, kept apart while writing.
    var quote: String?
    var attachments: [DraftAttachment] = []
    /// A message whose attachments go along, for a forward.
    var forwardAttachmentsOf: String?

    enum CodingKeys: String, CodingKey {
        case accountId, to, cc, bcc, subject, text, inReplyTo, references, threadId, from, html, quote, attachments, forwardAttachmentsOf
    }

    init(accountId: String, to: [Address], subject: String, text: String) {
        self.accountId = accountId
        self.to = to
        self.subject = subject
        self.text = text
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        accountId = try values.decode(String.self, forKey: .accountId)
        to = try values.decodeIfPresent([Address].self, forKey: .to) ?? []
        cc = try values.decodeIfPresent([Address].self, forKey: .cc) ?? []
        bcc = try values.decodeIfPresent([Address].self, forKey: .bcc) ?? []
        subject = try values.decodeIfPresent(String.self, forKey: .subject) ?? ""
        text = try values.decodeIfPresent(String.self, forKey: .text) ?? ""
        inReplyTo = try values.decodeIfPresent(String.self, forKey: .inReplyTo)
        references = try values.decodeIfPresent([String].self, forKey: .references) ?? []
        threadId = try values.decodeIfPresent(String.self, forKey: .threadId)
        from = try values.decodeIfPresent(Address.self, forKey: .from)
        html = try values.decodeIfPresent(String.self, forKey: .html)
        quote = try values.decodeIfPresent(String.self, forKey: .quote)
        attachments = try values.decodeIfPresent([DraftAttachment].self, forKey: .attachments) ?? []
        forwardAttachmentsOf = try values.decodeIfPresent(String.self, forKey: .forwardAttachmentsOf)
    }
}

/// A file going out with a draft: `path` on this device until the core uploads it.
struct DraftAttachment: Codable, Hashable {
    var name: String
    var mime: String
    var size: Int64
    var upload: String?
    var path: String?
}

/// The answer to `new_draft`, `reply_draft` and `open_draft`.
struct DraftReply: Codable {
    let id: String?
    let draft: Draft
    let from: String
}

/// What an action did: a toast's message, whether `undo` takes it back, a page to open.
struct ActReply: Codable {
    let message: String?
    let undo: Bool
    let url: String?
}

struct MessageReply: Codable {
    let message: String
}

struct IDReply: Codable {
    let id: String
}

struct PathReply: Codable {
    let path: String
}

struct HTMLReply: Codable {
    let html: String
}

/// A time a snooze, a reminder or a send-later can be for.
struct TimeChoice: Codable, Hashable, Identifiable {
    let id: String
    let name: String
    let label: String
    let until: Int64

    var date: Date { Date(timeIntervalSince1970: TimeInterval(until) / 1000) }
}

struct TimeChoices: Codable {
    let choices: [TimeChoice]
}

struct ContactList: Codable {
    let contacts: [Address]
}

/// Someone, and the latest threads with them.
struct Person: Codable, Hashable {
    let name: String
    let email: String
    let initials: String
    let threads: [ThreadRow]
}

struct PreferenceList: Codable {
    let values: [PreferenceValue]
}

struct PreferenceValue: Codable, Hashable {
    let key: String
    let value: JSONValue
}

/// Any JSON, for preferences.
enum JSONValue: Codable, Hashable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() {
            self = .null
        } else if let bool = try? value.decode(Bool.self) {
            self = .bool(bool)
        } else if let number = try? value.decode(Double.self) {
            self = .number(number)
        } else if let string = try? value.decode(String.self) {
            self = .string(string)
        } else if let array = try? value.decode([JSONValue].self) {
            self = .array(array)
        } else {
            self = .object(try value.decode([String: JSONValue].self))
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case .bool(let bool): try container.encode(bool)
        case .number(let number): try container.encode(number)
        case .string(let string): try container.encode(string)
        case .array(let array): try container.encode(array)
        case .object(let object): try container.encode(object)
        }
    }

    /// The value as `JSONSerialization` takes it, to send in a command.
    var any: Any {
        switch self {
        case .null: NSNull()
        case .bool(let bool): bool
        case .number(let number): number
        case .string(let string): string
        case .array(let array): array.map(\.any)
        case .object(let object): object.mapValues(\.any)
        }
    }

    var string: String? {
        guard case .string(let string) = self else { return nil }
        return string
    }

    var bool: Bool? {
        guard case .bool(let bool) = self else { return nil }
        return bool
    }

    subscript(key: String) -> JSONValue? {
        guard case .object(let object) = self else { return nil }
        return object[key]
    }
}

/// Mail that just arrived, for a notification.
struct NewMailItem: Codable, Hashable {
    let thread: String
    let accountId: String
    let from: String
    let subject: String
    let snippet: String
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
    /// The draft it was saved as again.
    let id: String?
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
    case changed(mailboxes: Bool, threads: [String], preferences: Bool)
    case connection(state: String, error: String?)
    case sent(opId: String)
    case sendFailed(opId: String, error: String, draft: Draft, draftId: String?)
    case searchResults(request: UInt64, rows: [ThreadRow])
    case newMail(messages: [NewMailItem])
    case error(message: String)
    case other

    private enum Keys: String, CodingKey {
        case type, mailboxes, threads, preferences, state, error, opId, draft, request, rows, message, messages, draftId
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        switch try values.decode(String.self, forKey: .type) {
        case "changed":
            self = .changed(
                mailboxes: try values.decode(Bool.self, forKey: .mailboxes), threads: try values.decode([String].self, forKey: .threads),
                preferences: try values.decodeIfPresent(Bool.self, forKey: .preferences) ?? false
            )
        case "connection":
            self = .connection(state: try values.decode(String.self, forKey: .state), error: try values.decodeIfPresent(String.self, forKey: .error))
        case "sent":
            self = .sent(opId: try values.decode(String.self, forKey: .opId))
        case "send_failed":
            self = .sendFailed(
                opId: try values.decode(String.self, forKey: .opId), error: try values.decode(String.self, forKey: .error),
                draft: try values.decode(Draft.self, forKey: .draft), draftId: try values.decodeIfPresent(String.self, forKey: .draftId)
            )
        case "search_results":
            self = .searchResults(request: try values.decode(UInt64.self, forKey: .request), rows: try values.decode([ThreadRow].self, forKey: .rows))
        case "new_mail":
            self = .newMail(messages: try values.decode([NewMailItem].self, forKey: .messages))
        case "error":
            self = .error(message: try values.decode(String.self, forKey: .message))
        default:
            self = .other
        }
    }
}

/// What can be done to threads, as the core names it.
enum Action: String {
    case archive, trash, read, unread, star, unstar, inbox, spam, snooze, move, mute, unmute, unsubscribe, block
    case addLabel = "add_label"
    case removeLabel = "remove_label"
}

/// What a list can be narrowed to.
enum Filter: String {
    case unread, starred
}

enum ReplyKind: String {
    case reply
    case replyAll = "reply_all"
    case forward
}
