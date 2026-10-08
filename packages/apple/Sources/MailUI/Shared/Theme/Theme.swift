import SwiftUI

/// A number of `packages/theme/tokens.json`, light and dark.
struct ThemeNumber: Equatable {
    let light: Double
    let dark: Double
}

/// A colour of `packages/theme/tokens.json`, light and dark.
struct ThemeColor: Equatable {
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

/// The distances between things. Views space with these, never a number of their own.
enum Space {
    /// Inside a control: an icon and its words.
    static let xs: CGFloat = 4
    /// Between controls on a line.
    static let s: CGFloat = 8
    /// Between the lines of a block.
    static let m: CGFloat = 12
    /// Between blocks, and a page's inset on a phone.
    static let l: CGFloat = 16
    /// A page's inset on the Mac.
    static let xl: CGFloat = 20
    /// Between the sections of a page.
    static let xxl: CGFloat = 28
}

/// The sizes the apps are drawn with, measured from Newton Mail. Those of controls are as on
/// the Mac; the components enlarge them on iOS with `Platform.scale`.
enum Theme {
    static let radius = Tokens.radius
    /// A card or a popup, which rounds a little more than a control.
    static let cardRadius = Tokens.radius + 2
    /// A hairline: a rule, a border.
    static let hairline: CGFloat = 1
    /// Between the faces of buttons side by side. Each button pads half of it inside its hit
    /// area, so the gap is only drawn: hit areas still touch.
    static let buttonGap: CGFloat = 2

    /// A text field, as tall as the regular control.
    static let fieldHeight: CGFloat = ControlSize.regular.height
    /// A row of a list or a menu.
    static let rowHeight: CGFloat = ControlSize.regular.height
    /// A chip: the smallest thing that can be clicked.
    static let chipHeight: CGFloat = ControlSize.small.height
    /// A bar across the window: the top bar, a sheet's header, a toolbar.
    static let barHeight: CGFloat = 52
    /// A search field in a sheet.
    static let searchHeight: CGFloat = ControlSize.large.height

    /// A thread row on the Mac: one line.
    static let macRowHeight: CGFloat = 48
    /// A thread row on iOS: sender, subject, snippet.
    static let iosRowHeight: CGFloat = 88
    /// The bar of the account's colour at the start of a thread row.
    static let accountBarWidth: CGFloat = 2
    /// The widest the list and the thread's card grow on the Mac.
    static let cardWidth: CGFloat = 1000
    static let sidebarWidth: CGFloat = 232

    static let accountColors = [
        Tokens.account1, Tokens.account2, Tokens.account3, Tokens.account4, Tokens.account5,
        Tokens.account6, Tokens.account7, Tokens.account8, Tokens.account9, Tokens.account10,
    ]

    /// The names the core knows the colours by, `account-1` to `account-10`.
    static let accountColorNames = accountColors.indices.map { "account-\($0 + 1)" }

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
