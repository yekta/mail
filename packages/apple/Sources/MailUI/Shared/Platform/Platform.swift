import SwiftUI

#if os(macOS)
import AppKit

typealias PlatformColor = NSColor
typealias PlatformFont = NSFont
typealias PlatformImage = NSImage
#else
import UIKit

typealias PlatformColor = UIColor
typealias PlatformFont = UIFont
typealias PlatformImage = UIImage
#endif

/// What the Mac and iOS do differently, behind one name each.
enum Platform {
    #if os(macOS)
    /// How much larger than on the Mac text and the controls around it are.
    static let scale: CGFloat = 1
    #else
    static let scale: CGFloat = 1.15
    #endif

    static func open(_ url: URL) {
        #if os(macOS)
        NSWorkspace.shared.open(url)
        #else
        UIApplication.shared.open(url)
        #endif
    }

    /// The app's version, as its bundle says.
    static var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
    }

    /// Where the core keeps its database.
    static var dataFolder: URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return base.appendingPathComponent(Bundle.main.bundleIdentifier ?? "com.yekta.mail", isDirectory: true)
    }

    static func font(_ size: CGFloat, _ weight: PlatformFont.Weight = .regular) -> PlatformFont {
        PlatformFont.systemFont(ofSize: size * scale, weight: weight)
    }
}

extension Font {
    /// The system font at a Mac size, enlarged on iOS.
    static func ui(_ size: CGFloat, _ weight: Font.Weight = .regular) -> Font {
        .system(size: size * Platform.scale, weight: weight)
    }
}
