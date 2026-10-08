import CoreText
import SwiftUI

/// The icons the apps draw, from Lucide's font (lucide-static 1.52.0): each is the character it
/// has in that version's `font/codepoints.json`. The filled star, which Lucide doesn't have, is
/// drawn by hand.
enum Symbol: String {
    case alarmClock = "\u{e03a}"
    case archive = "\u{e041}"
    case archiveRestore = "\u{e2cd}"
    case arrowLeft = "\u{e048}"
    case atSign = "\u{e04e}"
    case ban = "\u{e051}"
    case bell = "\u{e059}"
    case bellOff = "\u{e05a}"
    case calendarClock = "\u{e304}"
    case check = "\u{e06c}"
    case checkCheck = "\u{e38e}"
    case chevronDown = "\u{e06d}"
    case chevronLeft = "\u{e06e}"
    case chevronRight = "\u{e06f}"
    case chevronUp = "\u{e070}"
    case circleAlert = "\u{e077}"
    case circleArrowDown = "\u{e078}"
    case circleCheck = "\u{e226}"
    case clock = "\u{e087}"
    case command = "\u{e09a}"
    case download = "\u{e0b2}"
    case ellipsis = "\u{e0b6}"
    case file = "\u{e0c0}"
    case folderInput = "\u{e334}"
    case forward = "\u{e229}"
    case globe = "\u{e0e8}"
    case handshake = "\u{e5c0}"
    case image = "\u{e0f6}"
    case inbox = "\u{e0f7}"
    case keyboard = "\u{e284}"
    case listFilter = "\u{e460}"
    case logOut = "\u{e10e}"
    case mail = "\u{e10f}"
    case mailMinus = "\u{e362}"
    case mailOpen = "\u{e363}"
    case mailX = "\u{e368}"
    case menu = "\u{e115}"
    case monitor = "\u{e11d}"
    case moon = "\u{e11e}"
    case paperclip = "\u{e12d}"
    case pencil = "\u{e1f9}"
    case plus = "\u{e13d}"
    case printer = "\u{e141}"
    case refreshCw = "\u{e145}"
    case reply = "\u{e22a}"
    case replyAll = "\u{e22b}"
    case search = "\u{e151}"
    case send = "\u{e152}"
    case server = "\u{e153}"
    case settings = "\u{e154}"
    case shieldAlert = "\u{e1fe}"
    case squareCheck = "\u{e559}"
    case squarePen = "\u{e172}"
    case star = "\u{e176}"
    /// A star filled in, drawn by hand.
    case starFilled = "star-filled"
    case sun = "\u{e178}"
    case tag = "\u{e17f}"
    case trash = "\u{e18e}"
    case undo = "\u{e2a1}"
    case user = "\u{e19f}"
    case wifiOff = "\u{e1af}"
    case x = "\u{e1b2}"
    case zap = "\u{e1b4}"

    /// The icon a mailbox is drawn with, by the name the core gives it.
    static func named(_ name: String) -> Symbol {
        switch name {
        case "inbox": .inbox
        case "mail": .mail
        case "star": .star
        case "clock": .clock
        case "send": .send
        case "file": .file
        case "archive": .archive
        case "shield-alert": .shieldAlert
        case "trash": .trash
        default: .tag
        }
    }
}

extension PlatformImage {
    private static var symbols: [String: PlatformImage] = [:]
    private static let symbolLock = NSLock()

    private static let symbolFont: CTFontDescriptor? = {
        let url = Bundle.main.url(forResource: "lucide", withExtension: "ttf", subdirectory: "Fonts")
            ?? Bundle.module.url(forResource: "lucide", withExtension: "ttf", subdirectory: "Fonts")
        guard let url, let fonts = CTFontManagerCreateFontDescriptorsFromURL(url as CFURL) as? [CTFontDescriptor] else { return nil }
        return fonts.first
    }()

    /// The symbol in one colour that a view tints, in a square of `side` points. Each is made once.
    static func symbol(_ symbol: Symbol, side: CGFloat) -> PlatformImage {
        let key = "\(symbol.rawValue)/\(side)"
        symbolLock.lock()
        defer { symbolLock.unlock() }
        if let made = symbols[key] { return made }
        let made = drawn(symbol, side: side)
        symbols[key] = made
        return made
    }

    private static func glyph(_ symbol: Symbol, side: CGFloat) -> CGPath? {
        guard let symbolFont else { return nil }
        let font = CTFontCreateWithFontDescriptor(symbolFont, side, nil)
        var character = Array(symbol.rawValue.utf16)
        var glyph = CGGlyph()
        guard CTFontGetGlyphsForCharacters(font, &character, &glyph, 1) else { return nil }
        return CTFontCreatePathForGlyph(font, glyph, nil)
    }

    /// A five-pointed star in Lucide's square of 24, with the room it leaves around its icons.
    private static func filledStar(side: CGFloat) -> CGPath {
        let path = CGMutablePath()
        let center = CGPoint(x: side / 2, y: side / 2 - side * 0.02)
        let outer = side * 0.42
        let inner = outer * 0.45
        for point in 0..<10 {
            let radius = point % 2 == 0 ? outer : inner
            let angle = CGFloat.pi / 2 + CGFloat(point) * .pi / 5
            let spot = CGPoint(x: center.x + radius * cos(angle), y: center.y + radius * sin(angle))
            point == 0 ? path.move(to: spot) : path.addLine(to: spot)
        }
        path.closeSubpath()
        return path
    }

    private static func drawn(_ symbol: Symbol, side: CGFloat) -> PlatformImage {
        guard let path = symbol == .starFilled ? filledStar(side: side) : glyph(symbol, side: side) else { return PlatformImage() }
        let size = CGSize(width: side, height: side)
        #if os(macOS)
        let image = NSImage(size: size, flipped: false) { _ in
            guard let context = NSGraphicsContext.current?.cgContext else { return false }
            context.setFillColor(.black)
            context.addPath(path)
            context.fillPath()
            return true
        }
        image.isTemplate = true
        return image
        #else
        let image = UIGraphicsImageRenderer(size: size).image { renderer in
            let context = renderer.cgContext
            context.translateBy(x: 0, y: side)
            context.scaleBy(x: 1, y: -1)
            context.setFillColor(UIColor.black.cgColor)
            context.addPath(path)
            context.fillPath()
        }
        return image.withRenderingMode(.alwaysTemplate)
        #endif
    }
}

extension Image {
    /// The symbol in the colour of the text around it, `size` points wide on the Mac.
    init(_ symbol: Symbol, size: CGFloat = 15) {
        let side = (size * Platform.scale).rounded()
        #if os(macOS)
        self = Image(nsImage: .symbol(symbol, side: side)).renderingMode(.template)
        #else
        self = Image(uiImage: .symbol(symbol, side: side)).renderingMode(.template)
        #endif
    }
}
