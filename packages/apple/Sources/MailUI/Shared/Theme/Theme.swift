import SwiftUI

/// A colour of `packages/theme/tokens.json`, light and dark.
struct ThemeColor {
    let light: UInt32
    let dark: UInt32

    private static func make(_ hex: UInt32) -> PlatformColor {
        PlatformColor(
            red: CGFloat((hex >> 16) & 0xff) / 255, green: CGFloat((hex >> 8) & 0xff) / 255,
            blue: CGFloat(hex & 0xff) / 255, alpha: 1
        )
    }

    /// The colour for the appearance it is drawn in.
    var platform: PlatformColor {
        let (light, dark) = (Self.make(self.light), Self.make(self.dark))
        #if os(macOS)
        return NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark : light
        }
        #else
        return UIColor { traits in traits.userInterfaceStyle == .dark ? dark : light }
        #endif
    }

    var color: Color { Color(platform) }
}

/// The sizes the apps are drawn with, measured from Newton Mail.
enum Theme {
    /// A thread row on the Mac: one line.
    static let macRowHeight: CGFloat = 48
    /// A thread row on iOS: sender, subject, snippet.
    static let iosRowHeight: CGFloat = 88
    /// The bar of the account's colour at the start of a thread row.
    static let accountBarWidth: CGFloat = 3
    /// The widest the list and the thread's card grow on the Mac.
    static let cardWidth: CGFloat = 1000
    static let sidebarWidth: CGFloat = 232
    static let radius = Tokens.radius

    /// An account's colour by the token name the core gives it.
    static func accountColor(_ name: String) -> ThemeColor {
        switch name {
        case "chart-2": Tokens.chart2
        case "chart-3": Tokens.chart3
        case "chart-4": Tokens.chart4
        case "chart-5": Tokens.chart5
        default: Tokens.chart1
        }
    }
}

enum Appearance: String, CaseIterable, Identifiable {
    case system, light, dark

    var id: String { rawValue }

    var name: String {
        switch self {
        case .system: "System"
        case .light: "Light"
        case .dark: "Dark"
        }
    }

    var scheme: ColorScheme? {
        switch self {
        case .system: nil
        case .light: .light
        case .dark: .dark
        }
    }
}
