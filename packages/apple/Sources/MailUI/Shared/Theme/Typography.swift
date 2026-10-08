import SwiftUI

/// Every size of text the apps draw, with the weight and the colour it usually has. A view
/// picks a style, never a size.
enum TextStyle {
    /// The app's name on the first screen.
    case display
    /// A thread's subject.
    case title
    /// A sheet's or a pane's heading.
    case heading
    /// A heading inside a page: a message's sender, a draft's name.
    case subheading
    /// Mail, fields and rows.
    case body
    /// Body text that stands out: an unread row, a selected one.
    case bodyStrong
    /// Controls, bars and the words beside them.
    case label
    /// A control's words when they must stand out: a button.
    case labelStrong
    /// A line under a title: a detail, a date, a hint.
    case caption
    /// The smallest words: a count, a field's label.
    case footnote
    /// A section's name, in capitals.
    case overline
    /// Keys and codes.
    case mono

    var size: CGFloat {
        switch self {
        case .display: 28
        case .title: 24
        case .heading: 17
        case .subheading, .body, .bodyStrong: 14
        case .label, .labelStrong: 13
        case .caption, .mono: 12
        case .footnote, .overline: 11
        }
    }

    var weight: PlatformFont.Weight {
        switch self {
        case .display, .title, .heading, .subheading, .overline: .semibold
        case .bodyStrong, .labelStrong: .medium
        case .body, .label, .caption, .footnote, .mono: .regular
        }
    }

    /// The colour the style has unless a view says otherwise.
    var color: Color {
        switch self {
        case .display, .title, .heading, .subheading, .body, .bodyStrong, .label, .labelStrong: Tokens.foreground.color
        case .caption, .footnote, .overline: Tokens.mutedMoreForeground.color
        case .mono: Tokens.mutedForeground.color
        }
    }

    var font: Font {
        guard self != .mono else { return .system(size: size * Platform.scale, design: .monospaced) }
        return .ui(size, weight)
    }
}

extension Font {
    static func ui(_ style: MailUI.TextStyle) -> Font { style.font }
}

extension View {
    /// Draws the text in a style of the scale, in its colour or in `color`.
    func textStyle(_ style: TextStyle, color: Color? = nil) -> some View {
        font(style.font).foregroundStyle(color ?? style.color)
    }
}
