import SwiftUI

/// A number of `packages/theme/tokens.json`, light and dark.
struct ThemeNumber {
    let light: Double
    let dark: Double
}

/// A colour of `packages/theme/tokens.json`, light and dark.
struct ThemeColor {
    let light: UInt32
    let dark: UInt32
    var alpha = ThemeNumber(light: 1, dark: 1)

    private static func make(_ hex: UInt32, alpha: Double) -> PlatformColor {
        PlatformColor(
            red: CGFloat((hex >> 16) & 0xff) / 255, green: CGFloat((hex >> 8) & 0xff) / 255,
            blue: CGFloat(hex & 0xff) / 255, alpha: alpha
        )
    }

    /// The colour at an opacity of the theme, such as `Tokens.shadow.opacity(Tokens.shadowOpacity)`.
    func opacity(_ alpha: ThemeNumber) -> ThemeColor {
        ThemeColor(light: light, dark: dark, alpha: alpha)
    }

    /// The colour for the appearance it is drawn in.
    var platform: PlatformColor {
        let (light, dark) = (Self.make(self.light, alpha: alpha.light), Self.make(self.dark, alpha: alpha.dark))
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

    static let accountColors = [
        Tokens.account1, Tokens.account2, Tokens.account3, Tokens.account4, Tokens.account5,
        Tokens.account6, Tokens.account7, Tokens.account8, Tokens.account9, Tokens.account10,
    ]

    /// An account's colour by the token name the core gives it, `account-1` to `account-10`.
    static func accountColor(_ name: String) -> ThemeColor {
        guard let number = Int(name.dropFirst("account-".count)), (1...accountColors.count).contains(number) else {
            return accountColors[0]
        }
        return accountColors[number - 1]
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
