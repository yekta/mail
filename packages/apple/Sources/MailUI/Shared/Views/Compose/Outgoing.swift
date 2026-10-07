import Foundation
import UniformTypeIdentifiers

/// A file put into a draft: one picked or dropped, or the data of a pasted image.
enum Incoming {
    case file(URL)
    case data(Data, name: String)
}

/// Files going out with drafts, copied where the core can read them until it uploads them, and
/// the checks made on a message before it goes.
enum Outgoing {
    /// The most the attachments of a message can weigh together.
    static let limit: Int64 = 25 * 1024 * 1024

    /// Copies what came in. Runs off the main thread: files can be large.
    static func stage(_ incoming: [Incoming]) async -> [DraftAttachment] {
        await Task.detached(priority: .userInitiated) {
            incoming.compactMap { item -> DraftAttachment? in
                switch item {
                case .file(let url): try? copy(url)
                case .data(let data, let name): try? write(data, name: name)
                }
            }
        }.value
    }

    /// Loads what was dropped: a file, or the data of an image dragged from another app.
    static func stage(_ providers: [NSItemProvider]) async -> [DraftAttachment] {
        var staged: [DraftAttachment] = []
        for provider in providers {
            guard let attachment = await load(provider) else { continue }
            staged.append(attachment)
        }
        return staged
    }

    private static func load(_ provider: NSItemProvider) async -> DraftAttachment? {
        if provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
            let url = await withCheckedContinuation { continuation in
                _ = provider.loadObject(ofClass: URL.self) { url, _ in continuation.resume(returning: url) }
            }
            guard let url, url.isFileURL else { return nil }
            return await stage([.file(url)]).first
        }
        let types = provider.registeredTypeIdentifiers.compactMap(UTType.init).filter { $0.conforms(to: .data) }
        guard let type = types.first else { return nil }
        let name = provider.suggestedName.map { name in
            name.contains(".") ? name : name + "." + (type.preferredFilenameExtension ?? "")
        }
        return await withCheckedContinuation { continuation in
            _ = provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, _ in
                // The file is gone once this returns, so it is copied now.
                continuation.resume(returning: url.flatMap { try? copy($0, name: name) })
            }
        }
    }

    private static func copy(_ url: URL, name: String? = nil) throws -> DraftAttachment {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let target = try place(name ?? url.lastPathComponent)
        try FileManager.default.copyItem(at: url, to: target)
        return attachment(at: target)
    }

    private static func write(_ data: Data, name: String) throws -> DraftAttachment {
        let target = try place(name)
        try data.write(to: target)
        return attachment(at: target)
    }

    private static var folder: URL { Platform.dataFolder.appendingPathComponent("Outgoing", isDirectory: true) }

    /// Forgets files staged over a week ago: their drafts were sent or set aside long since.
    static func prune() {
        Task.detached(priority: .background) {
            let manager = FileManager.default
            let old = Date().addingTimeInterval(-7 * 24 * 3600)
            let staged = (try? manager.contentsOfDirectory(at: folder, includingPropertiesForKeys: [.creationDateKey])) ?? []
            for item in staged {
                guard let created = try? item.resourceValues(forKeys: [.creationDateKey]).creationDate, created < old else { continue }
                try? manager.removeItem(at: item)
            }
        }
    }

    /// A new folder of its own for each file, so names never collide.
    private static func place(_ name: String) throws -> URL {
        let folder = folder.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        return folder.appendingPathComponent(name.isEmpty ? "Attachment" : name)
    }

    private static func attachment(at url: URL) -> DraftAttachment {
        let size = (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? NSNumber)?.int64Value ?? 0
        let mime = UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
        return DraftAttachment(name: url.lastPathComponent, mime: mime, size: size, path: url.path)
    }

    // Checks before sending.

    /// Whether the text speaks of an attachment.
    static func mentionsAttachment(_ text: String) -> Bool {
        text.range(of: #"\b(attach(ed|es|ing|ment|ments)?|enclosed)\b"#, options: [.regularExpression, .caseInsensitive]) != nil
    }

    /// The `{placeholders}` of a snippet still in the text.
    static func placeholders(in text: String) -> [String] {
        text.matches(of: #/\{[A-Za-z_]+\}/#).map { String(text[$0.range]) }
    }
}
