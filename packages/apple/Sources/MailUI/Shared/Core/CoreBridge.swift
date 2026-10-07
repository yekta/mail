import CMailCore
import Foundation

/// The app's end of the Rust core: commands go in as JSON, events come out as JSON. Events and
/// answers are decoded on a background queue, in the order they were sent; events are handed to
/// the main thread.
final class CoreBridge: @unchecked Sendable {
    struct CoreError: Error, LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    /// Called on the main thread with each event that isn't an answer.
    var onEvent: ((CoreEvent) -> Void)?

    private let decoding = DispatchQueue(label: "com.yekta.mail.events", qos: .userInitiated)
    private let lock = NSLock()
    private var nextID: UInt64 = 0
    /// What to do with each answer, on the decoding queue: whether it went well, and its value.
    private var answers: [UInt64: (Bool, Data) -> Void] = [:]

    static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    static let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        return encoder
    }()

    func start(config: [String: Any]) -> Bool {
        guard let json = Self.json(config) else { return false }
        let context = Unmanaged.passUnretained(self).toOpaque()
        return json.withCString { mail_core_start($0, coreEvent, context) }
    }

    /// Sends a command and waits for its answer, decoded off the main thread.
    func call<Answer: Decodable>(_ type: String, _ fields: [String: Any] = [:], as: Answer.Type = Answer.self) async throws -> Answer {
        try await withCheckedThrowingContinuation { continuation in
            let id = register { ok, data in
                guard ok else {
                    let message = (try? Self.decoder.decode(Failure.self, from: data))?.error ?? "Something went wrong."
                    continuation.resume(throwing: CoreError(message: message))
                    return
                }
                do {
                    continuation.resume(returning: try Self.decoder.decode(Answer.self, from: data))
                } catch {
                    continuation.resume(throwing: CoreError(message: "The core's answer can't be read: \(error)"))
                }
            }
            post(type, fields, id: id)
        }
    }

    /// Sends a command whose answer doesn't matter.
    func send(_ type: String, _ fields: [String: Any] = [:]) {
        post(type, fields, id: register { _, _ in })
    }

    /// A value as the JSON object a command carries.
    static func object<Value: Encodable>(_ value: Value) -> Any {
        guard let data = try? encoder.encode(value) else { return [:] }
        return (try? JSONSerialization.jsonObject(with: data)) ?? [:]
    }

    private func register(_ answer: @escaping (Bool, Data) -> Void) -> UInt64 {
        lock.withLock {
            nextID += 1
            answers[nextID] = answer
            return nextID
        }
    }

    private func post(_ type: String, _ fields: [String: Any], id: UInt64) {
        var command = fields
        command["type"] = type
        command["id"] = id
        guard let json = Self.json(command) else { return }
        json.withCString { mail_core_command($0) }
    }

    fileprivate func received(_ data: Data) {
        decoding.async { [weak self] in
            guard let self, let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
            if event["type"] as? String == "reply" {
                let id = (event["id"] as? NSNumber)?.uint64Value ?? 0
                let value = (try? JSONSerialization.data(withJSONObject: event["value"] ?? [:], options: [.fragmentsAllowed])) ?? Data()
                let answer = self.lock.withLock { self.answers.removeValue(forKey: id) }
                answer?(event["ok"] as? Bool == true, value)
                return
            }
            guard let decoded = try? Self.decoder.decode(CoreEvent.self, from: data) else { return }
            DispatchQueue.main.async { self.onEvent?(decoded) }
        }
    }

    private struct Failure: Decodable {
        let error: String
    }

    private static func json(_ object: [String: Any]) -> String? {
        guard let data = try? JSONSerialization.data(withJSONObject: object) else { return nil }
        return String(data: data, encoding: .utf8)
    }
}

private func coreEvent(_ json: UnsafePointer<CChar>?, _ context: UnsafeMutableRawPointer?) {
    guard let json, let context else { return }
    let data = Data(bytes: json, count: strlen(json))
    Unmanaged<CoreBridge>.fromOpaque(context).takeUnretainedValue().received(data)
}
