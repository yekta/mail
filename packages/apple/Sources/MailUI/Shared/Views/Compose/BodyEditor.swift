import SwiftUI
import UniformTypeIdentifiers

/// Keys the body editor hands over while a menu over it is open.
enum EditorKey {
    case up, down, accept, cancel
}

/// Lets the compose view reach into its body editor: focus it, or put text at the caret or in
/// place of the `;word` being typed.
@MainActor
final class BodyEditing {
    fileprivate weak var view: BodyTextView?

    func focus() {
        guard let view else { return }
        #if os(macOS)
        view.window?.makeFirstResponder(view)
        #else
        view.becomeFirstResponder()
        #endif
    }

    func insert(_ text: String) {
        guard let view else { return }
        view.replace(view.selection, with: text)
    }

    func replaceTrigger(with text: String) {
        guard let view, let trigger = BodyEditor.trigger(in: view.content, caret: view.selection.location) else { return }
        view.replace(trigger.range, with: text)
    }
}

/// The text of a message, as tall as what is written, so the sheet scrolls it with the fields.
/// Files and images pasted into it are attached. `onTrigger` hears the word typed after a `;`
/// at the caret (nil when there is none); `onKey` the arrow keys, Tab, Return and Esc while
/// `menuOpen`.
struct BodyEditor {
    @Binding var text: String
    @Binding var height: CGFloat
    let editing: BodyEditing
    var menuOpen = false
    let onTrigger: (String?) -> Void
    let onKey: (EditorKey) -> Bool
    let onPaste: ([Incoming]) -> Void

    /// The `;word` just before the caret: a `;` at the start of a word, then letters.
    static func trigger(in text: String, caret: Int) -> (word: String, range: NSRange)? {
        let text = text as NSString
        guard caret <= text.length else { return nil }
        var start = caret
        while start > 0, caret - start < 30 {
            let character = text.character(at: start - 1)
            if character == 59 { break }
            guard let scalar = UnicodeScalar(character), CharacterSet.alphanumerics.contains(scalar) || character == 95 || character == 45 else { return nil }
            start -= 1
        }
        guard start > 0, text.character(at: start - 1) == 59 else { return nil }
        let semicolon = start - 1
        if semicolon > 0, let before = UnicodeScalar(text.character(at: semicolon - 1)), !CharacterSet.whitespacesAndNewlines.contains(before) {
            return nil
        }
        return (text.substring(with: NSRange(location: start, length: caret - start)), NSRange(location: semicolon, length: caret - semicolon))
    }

    final class Coordinator: NSObject {
        var parent: BodyEditor

        init(_ parent: BodyEditor) {
            self.parent = parent
        }

        @MainActor func changed(_ view: BodyTextView) {
            if parent.text != view.content { parent.text = view.content }
            selected(view)
            view.reportHeight()
        }

        @MainActor func selected(_ view: BodyTextView) {
            let trigger = BodyEditor.trigger(in: view.content, caret: view.selection.location)
            parent.onTrigger(view.selection.length == 0 ? trigger?.word : nil)
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    @MainActor private func configure(_ view: BodyTextView, context: Context) {
        editing.view = view
        view.onPaste = onPaste
        view.onKey = onKey
        view.menuOpen = menuOpen
        view.onHeight = { reported in
            DispatchQueue.main.async { if abs(height - reported) > 0.5 { height = reported } }
        }
    }

    @MainActor private func update(_ view: BodyTextView, context: Context) {
        context.coordinator.parent = self
        configure(view, context: context)
        guard view.content != text else { return }
        view.content = text
        view.reportHeight()
    }
}

#if os(macOS)
extension BodyEditor: NSViewRepresentable {
    func makeNSView(context: Context) -> BodyTextView {
        let view = BodyTextView(usingTextLayoutManager: false)
        view.delegate = context.coordinator
        view.isRichText = false
        view.importsGraphics = false
        view.allowsUndo = true
        view.drawsBackground = false
        view.isVerticallyResizable = false
        view.isHorizontallyResizable = false
        view.textContainerInset = .zero
        view.textContainer?.lineFragmentPadding = 0
        view.textContainer?.widthTracksTextView = true
        view.font = Platform.font(14)
        view.textColor = Tokens.foreground.platform
        view.insertionPointColor = Tokens.foreground.platform
        view.typingAttributes = [.font: Platform.font(14), .foregroundColor: Tokens.foreground.platform]
        view.string = text
        configure(view, context: context)
        return view
    }

    func updateNSView(_ view: BodyTextView, context: Context) {
        update(view, context: context)
    }
}

extension BodyEditor.Coordinator: NSTextViewDelegate {
    func textDidChange(_ notification: Notification) {
        guard let view = notification.object as? BodyTextView else { return }
        MainActor.assumeIsolated { changed(view) }
    }

    func textViewDidChangeSelection(_ notification: Notification) {
        guard let view = notification.object as? BodyTextView else { return }
        MainActor.assumeIsolated { selected(view) }
    }

    func textView(_ textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        guard let view = textView as? BodyTextView, view.menuOpen else { return false }
        let keys: [Selector: EditorKey] = [
            #selector(NSResponder.moveUp(_:)): .up, #selector(NSResponder.moveDown(_:)): .down,
            #selector(NSResponder.insertTab(_:)): .accept, #selector(NSResponder.insertNewline(_:)): .accept,
            #selector(NSResponder.cancelOperation(_:)): .cancel,
        ]
        guard let key = keys[selector] else { return false }
        return MainActor.assumeIsolated { view.onKey?(key) ?? false }
    }
}

final class BodyTextView: NSTextView {
    var onPaste: (([Incoming]) -> Void)?
    var onKey: ((EditorKey) -> Bool)?
    var onHeight: ((CGFloat) -> Void)?
    var menuOpen = false

    var content: String {
        get { string }
        set { string = newValue }
    }

    var selection: NSRange { selectedRange() }

    func replace(_ range: NSRange, with text: String) {
        guard shouldChangeText(in: range, replacementString: text) else { return }
        textStorage?.replaceCharacters(in: range, with: NSAttributedString(string: text, attributes: typingAttributes))
        didChangeText()
        setSelectedRange(NSRange(location: range.location + (text as NSString).length, length: 0))
    }

    /// Files dropped on the text are attached by the sheet, not written into it as paths.
    override var acceptableDragTypes: [NSPasteboard.PasteboardType] {
        super.acceptableDragTypes.filter { ![.fileURL, .tiff, .png, NSPasteboard.PasteboardType("NSFilenamesPboardType")].contains($0) }
    }

    override func paste(_ sender: Any?) {
        let board = NSPasteboard.general
        if let urls = board.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL], !urls.isEmpty {
            onPaste?(urls.map(Incoming.file))
            return
        }
        if board.string(forType: .string) == nil, let image = NSImage(pasteboard: board), let tiff = image.tiffRepresentation,
            let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:])
        {
            onPaste?([.data(png, name: "Pasted image.png")])
            return
        }
        super.paste(sender)
    }

    override func setFrameSize(_ size: NSSize) {
        super.setFrameSize(size)
        reportHeight()
    }

    func reportHeight() {
        guard let container = textContainer, let layout = layoutManager else { return }
        layout.ensureLayout(for: container)
        onHeight?(ceil(layout.usedRect(for: container).height) + 6)
    }
}
#else
extension BodyEditor: UIViewRepresentable {
    func makeUIView(context: Context) -> BodyTextView {
        let view = BodyTextView()
        view.delegate = context.coordinator
        view.isScrollEnabled = false
        view.backgroundColor = .clear
        view.textContainerInset = .zero
        view.textContainer.lineFragmentPadding = 0
        view.font = Platform.font(14)
        view.textColor = Tokens.foreground.platform
        view.text = text
        configure(view, context: context)
        return view
    }

    func updateUIView(_ view: BodyTextView, context: Context) {
        update(view, context: context)
    }
}

extension BodyEditor.Coordinator: UITextViewDelegate {
    func textViewDidChange(_ textView: UITextView) {
        guard let view = textView as? BodyTextView else { return }
        changed(view)
    }

    func textViewDidChangeSelection(_ textView: UITextView) {
        guard let view = textView as? BodyTextView else { return }
        selected(view)
    }
}

final class BodyTextView: UITextView {
    var onPaste: (([Incoming]) -> Void)?
    var onKey: ((EditorKey) -> Bool)?
    var onHeight: ((CGFloat) -> Void)?
    var menuOpen = false

    var content: String {
        get { text }
        set { text = newValue }
    }

    var selection: NSRange { selectedRange }

    func replace(_ range: NSRange, with text: String) {
        guard let start = position(from: beginningOfDocument, offset: range.location),
            let end = position(from: start, offset: range.length), let span = textRange(from: start, to: end)
        else { return }
        replace(span, withText: text)
        delegate?.textViewDidChange?(self)
    }

    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        if action == #selector(paste(_:)), UIPasteboard.general.hasImages { return true }
        return super.canPerformAction(action, withSender: sender)
    }

    override func paste(_ sender: Any?) {
        let board = UIPasteboard.general
        if !board.hasStrings, board.hasImages {
            let images = (board.images ?? []).compactMap { $0.pngData() }
            onPaste?(images.map { .data($0, name: "Pasted image.png") })
            return
        }
        super.paste(sender)
    }

    override var keyCommands: [UIKeyCommand]? {
        guard menuOpen else { return super.keyCommands }
        let keys: [(String, Selector)] = [
            (UIKeyCommand.inputUpArrow, #selector(up)), (UIKeyCommand.inputDownArrow, #selector(down)),
            ("\t", #selector(accept)), ("\r", #selector(accept)), (UIKeyCommand.inputEscape, #selector(cancel)),
        ]
        return keys.map { input, action in
            let command = UIKeyCommand(input: input, modifierFlags: [], action: action)
            command.wantsPriorityOverSystemBehavior = true
            return command
        }
    }

    @objc private func up() { _ = onKey?(.up) }
    @objc private func down() { _ = onKey?(.down) }
    @objc private func accept() { _ = onKey?(.accept) }
    @objc private func cancel() { _ = onKey?(.cancel) }

    override func layoutSubviews() {
        super.layoutSubviews()
        reportHeight()
    }

    func reportHeight() {
        guard bounds.width > 0 else { return }
        onHeight?(ceil(sizeThatFits(CGSize(width: bounds.width, height: .greatestFiniteMagnitude)).height) + 6)
    }
}
#endif
