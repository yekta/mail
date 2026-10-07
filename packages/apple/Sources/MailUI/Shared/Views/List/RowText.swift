import SwiftUI

/// A thread row's text, styled once and drawn by the Mac's table and iOS's: unread rows are bold
/// and dark, read rows grey, as Newton drew them.
struct RowText {
    let senders: NSAttributedString
    let subject: NSAttributedString
    let snippet: NSAttributedString
    /// The subject and the snippet on one line, for the Mac.
    let line: NSAttributedString
    let date: NSAttributedString

    #if os(macOS)
    static let sendersSize: CGFloat = 13
    static let subjectSize: CGFloat = 13
    static let snippetSize: CGFloat = 13
    static let dateSize: CGFloat = 12
    #else
    static let sendersSize: CGFloat = 16
    static let subjectSize: CGFloat = 15
    static let snippetSize: CGFloat = 14.5
    static let dateSize: CGFloat = 13
    #endif

    private static func style(_ size: CGFloat, _ weight: PlatformFont.Weight, _ color: ThemeColor) -> [NSAttributedString.Key: Any] {
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineBreakMode = .byTruncatingTail
        return [.font: PlatformFont.systemFont(ofSize: size, weight: weight), .foregroundColor: color.platform, .paragraphStyle: paragraph]
    }

    init(_ row: ThreadRow) {
        let strong = row.unread ? Tokens.foreground : Tokens.secondaryForeground
        let weight: PlatformFont.Weight = row.unread ? .semibold : .regular
        senders = NSAttributedString(string: row.senders, attributes: Self.style(Self.sendersSize, weight, strong))
        let subjectText = row.subject.isEmpty ? "(no subject)" : row.subject
        subject = NSAttributedString(string: subjectText, attributes: Self.style(Self.subjectSize, weight, strong))
        let snippetText = row.snippet.replacingOccurrences(of: "\n", with: " ")
        snippet = NSAttributedString(string: snippetText, attributes: Self.style(Self.snippetSize, .regular, Tokens.mutedForeground))
        let line = NSMutableAttributedString(attributedString: subject)
        if !snippetText.isEmpty {
            line.append(NSAttributedString(string: "  –  " + snippetText, attributes: Self.style(Self.snippetSize, .regular, Tokens.mutedForeground)))
        }
        self.line = line
        date = NSAttributedString(string: row.date, attributes: Self.style(Self.dateSize, row.unread ? .semibold : .regular, strong))
    }
}
