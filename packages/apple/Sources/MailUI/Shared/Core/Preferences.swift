import Foundation

/// Text kept to put in messages, `snippet:<id>`. `{first_name}` is filled from the first recipient.
struct Snippet: Identifiable, Hashable {
    let id: String
    var name: String
    var text: String

    var key: String { "snippet:\(id)" }
    var value: JSONValue { .object(["name": .string(name), "text": .string(text)]) }

    static func all(in preferences: [String: JSONValue]) -> [Snippet] {
        preferences.compactMap { key, value in
            guard key.hasPrefix("snippet:"), let name = value["name"]?.string else { return nil }
            return Snippet(id: String(key.dropFirst("snippet:".count)), name: name, text: value["text"]?.string ?? "")
        }
        .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    /// The text with what is known filled in.
    func filled(firstName: String?) -> String {
        guard let firstName else { return text }
        return text.replacingOccurrences(of: "{first_name}", with: firstName)
    }
}

/// A tab of Split Inbox the user made, `split:<id>`: mail from some senders, or with a label.
struct CustomSplit: Identifiable, Hashable {
    let id: String
    var name: String
    /// Addresses, and domains written `@x.com`.
    var from: [String]
    var label: String?
    var order: Int

    var key: String { "split:\(id)" }

    var value: JSONValue {
        .object([
            "name": .string(name), "from": .array(from.map { .string($0) }),
            "label": label.map { .string($0) } ?? .null, "order": .number(Double(order)),
        ])
    }

    static func all(in preferences: [String: JSONValue]) -> [CustomSplit] {
        preferences.compactMap { key, value -> CustomSplit? in
            guard key.hasPrefix("split:"), let name = value["name"]?.string else { return nil }
            var from: [String] = []
            if case .array(let values) = value["from"] { from = values.compactMap(\.string) }
            var order = 0
            if case .number(let number) = value["order"] { order = Int(number) }
            return CustomSplit(id: String(key.dropFirst("split:".count)), name: name, from: from, label: value["label"]?.string, order: order)
        }
        .sorted { ($0.order, $0.name) < ($1.order, $1.name) }
    }
}

extension Address {
    /// Whether mail can be sent to it.
    var isValid: Bool {
        email.wholeMatch(of: #/[^@\s<>,;"]+@[^@\s<>,;"]+\.[^@\s<>,;"]+/#) != nil
    }

    /// The first word of the name, for "Hi {first_name}".
    var firstName: String? {
        guard let name, let first = name.split(separator: " ").first, !first.contains("@") else { return nil }
        return String(first)
    }
}

/// The mailboxes shown as tabs over the list, `tabs`: their ids in order. Unset, the usual four.
/// A mailbox of every account together (`inbox`) is the account's own while one account's
/// mailbox is on screen.
enum TabPreference {
    static let key = "tabs"
    static let standard = ["inbox", "unread", "sent", "snoozed"]

    static func ids(in preferences: [String: JSONValue]) -> [String] {
        guard case .array(let values) = preferences[key] else { return standard }
        return values.compactMap(\.string)
    }

    static func value(_ ids: [String]) -> JSONValue {
        .array(ids.map { .string($0) })
    }
}

/// The sidebar's order: `sidebar`, the ids of the mailboxes of every account together, and
/// `accounts`, the accounts' ids. One not named comes after the named, as the core lists them.
enum SidebarOrder {
    static let mailboxesKey = "sidebar"
    static let accountsKey = "accounts"

    static func ids(_ key: String, in preferences: [String: JSONValue]) -> [String] {
        guard case .array(let values) = preferences[key] else { return [] }
        return values.compactMap(\.string)
    }

    static func value(_ ids: [String]) -> JSONValue {
        .array(ids.map { .string($0) })
    }

    /// The items in the order of `ids`, then the rest as they came.
    static func sorted<Item: Identifiable>(_ items: [Item], by ids: [String]) -> [Item] where Item.ID == String {
        let ranks = Dictionary(ids.enumerated().map { ($1, $0) }, uniquingKeysWith: { first, _ in first })
        return items.enumerated()
            .sorted { (ranks[$0.element.id] ?? ids.count, $0.offset) < (ranks[$1.element.id] ?? ids.count, $1.offset) }
            .map(\.element)
    }

    /// `id` put after the ids `before` it, with the rest of `all` after.
    static func placed(_ id: String, after before: [String], among all: [String]) -> [String] {
        before + [id] + all.filter { $0 != id && !before.contains($0) }
    }
}
