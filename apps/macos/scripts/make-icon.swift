// Draws Resources/AppIcon.png, the icon of both apps: Lucide's paper plane on blue.
//
//   swift apps/macos/scripts/make-icon.swift
import AppKit
import CoreText

let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
let font = root.appendingPathComponent("../../packages/apple/Sources/MailUI/Fonts/lucide.ttf").standardized
let side: CGFloat = 1024

guard let descriptors = CTFontManagerCreateFontDescriptorsFromURL(font as CFURL) as? [CTFontDescriptor], let descriptor = descriptors.first else {
    fatalError("lucide.ttf is missing")
}
let context = CGContext(data: nil, width: Int(side), height: Int(side), bitsPerComponent: 8, bytesPerRow: 0,
                        space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!

// The squircle Apple's icons sit in, with the room macOS leaves around it.
let inset: CGFloat = 100
let shape = CGPath(roundedRect: CGRect(x: inset, y: inset, width: side - 2 * inset, height: side - 2 * inset),
                   cornerWidth: 185, cornerHeight: 185, transform: nil)
context.addPath(shape)
context.clip()
let colors = [CGColor(srgbRed: 0.40, green: 0.64, blue: 0.97, alpha: 1), CGColor(srgbRed: 0.20, green: 0.42, blue: 0.88, alpha: 1)]
let gradient = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB), colors: colors as CFArray, locations: [0, 1])!
context.drawLinearGradient(gradient, start: CGPoint(x: 0, y: side), end: CGPoint(x: side, y: 0), options: [])

let glyphSide: CGFloat = 520
let ctFont = CTFontCreateWithFontDescriptor(descriptor, glyphSide, nil)
var character = Array("\u{e152}".utf16)
var glyph = CGGlyph()
CTFontGetGlyphsForCharacters(ctFont, &character, &glyph, 1)
let path = CTFontCreatePathForGlyph(ctFont, glyph, nil)!
let box = path.boundingBoxOfPath
var place = CGAffineTransform(translationX: (side - box.width) / 2 - box.minX - 12, y: (side - box.height) / 2 - box.minY - 12)
context.addPath(path.copy(using: &place)!)
context.setFillColor(CGColor(gray: 1, alpha: 1))
context.fillPath()

let image = context.makeImage()!
let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])!
try! png.write(to: root.appendingPathComponent("Resources/AppIcon.png"))
print("✓ Wrote apps/macos/Resources/AppIcon.png")
