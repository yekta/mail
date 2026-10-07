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
