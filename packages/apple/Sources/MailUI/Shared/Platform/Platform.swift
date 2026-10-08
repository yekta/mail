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

    /// Avenir Next, which both systems ship, at an exact size.
    static func face(_ size: CGFloat, _ weight: PlatformFont.Weight = .regular) -> PlatformFont {
        let name = switch weight {
        case .ultraLight, .thin, .light: "AvenirNext-UltraLight"
        case .medium: "AvenirNext-Medium"
        case .semibold: "AvenirNext-DemiBold"
        case .bold: "AvenirNext-Bold"
        case .heavy, .black: "AvenirNext-Heavy"
        default: "AvenirNext-Regular"
        }
        guard let font = PlatformFont(name: name, size: size) else {
            return .systemFont(ofSize: size, weight: weight)
        }
        return font
    }

    /// Avenir Next at a Mac size, enlarged on iOS.
    static func font(_ size: CGFloat, _ weight: PlatformFont.Weight = .regular) -> PlatformFont {
        face(size * scale, weight)
    }
}

extension Font {
    /// Avenir Next at a Mac size, enlarged on iOS.
    static func ui(_ size: CGFloat, _ weight: PlatformFont.Weight = .regular) -> Font {
        Font(Platform.font(size, weight) as CTFont)
    }
}
