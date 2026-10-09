import SwiftUI

#if os(macOS)
import AppKit
typealias PlatformScrollView = NSScrollView
#else
import UIKit
typealias PlatformScrollView = UIScrollView
#endif

/// Brings a scroll view back to where it was left, and reports where it goes after. The content
/// may not be tall enough at first (rows not yet laid out, message bodies still measuring), so
/// the offset is applied again each time the content grows, until it fits or the user scrolls.
/// Content that shrinks under the kept place for a moment (a body measured again before its
/// images are in) clamps the view to its end; that is not a scroll, and the place is taken up
/// again once the content is tall enough.
@MainActor
final class ScrollKeeper: NSObject {
    private(set) var pending: CGFloat?
    /// Where the view was left, as last reported, and how tall the content was then.
    private var kept: CGFloat
    private var height: CGFloat = 0
    /// Whether the kept place is past the end of content that shrank under it, and where the
    /// view was clamped to meanwhile.
    private var lost = false
    private var at: CGFloat = 0
    private let report: (CGFloat) -> Void
    private weak var scroll: PlatformScrollView?
    private var restoring = false
    #if os(iOS)
    private var observations: [NSKeyValueObservation] = []
    #endif

    init(restore: CGFloat, report: @escaping (CGFloat) -> Void) {
        pending = restore > 0 ? restore : nil
        kept = restore
        self.report = report
    }

    func attach(_ scroll: PlatformScrollView) {
        guard self.scroll !== scroll else { return }
        self.scroll = scroll
        #if os(macOS)
        scroll.contentView.postsBoundsChangedNotifications = true
        scroll.documentView?.postsFrameChangedNotifications = true
        let center = NotificationCenter.default
        center.addObserver(self, selector: #selector(moved), name: NSView.boundsDidChangeNotification, object: scroll.contentView)
        if let document = scroll.documentView {
            center.addObserver(self, selector: #selector(grew), name: NSView.frameDidChangeNotification, object: document)
        }
        center.addObserver(self, selector: #selector(dragged), name: NSScrollView.willStartLiveScrollNotification, object: scroll)
        #else
        observations = [
            scroll.observe(\.contentOffset) { [weak self] _, _ in MainActor.assumeIsolated { self?.moved() } },
            scroll.observe(\.contentSize) { [weak self] _, _ in MainActor.assumeIsolated { self?.grew() } },
        ]
        scroll.panGestureRecognizer.addTarget(self, action: #selector(panned))
        #endif
        restore()
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    /// Goes to the top, for a list that now shows another mailbox.
    func top() {
        pending = nil
        lost = false
        guard let scroll else { return }
        Self.scroll(scroll, to: Self.range(of: scroll).0)
    }

    @objc private func moved() {
        guard !restoring, let scroll else { return }
        guard pending == nil else {
            restore()
            return
        }
        let (top, bottom) = Self.range(of: scroll)
        // A bounce past an end is that end.
        let offset = min(max(Self.offset(of: scroll), top), bottom)
        let atEnd = abs(offset - bottom) <= 1
        // Clamped to the end of content that shrank short of the kept place: not a scroll. While
        // the place is lost, the content growing back may set the same offset again.
        let clamped = lost ? atEnd || offset == at : atEnd && kept - offset > 1 && Self.height(of: scroll) < height
        guard !clamped else {
            lost = true
            restore()
            at = Self.offset(of: scroll)
            return
        }
        lost = false
        keep(offset)
        report(offset)
    }

    @objc private func grew() {
        restore()
    }

    #if os(macOS)
    @objc private func dragged() {
        scrolled()
    }
    #else
    @objc private func panned(_ gesture: UIPanGestureRecognizer) {
        if gesture.state == .began { scrolled() }
    }
    #endif

    /// The user scrolls: wherever the view goes now is where it is left.
    private func scrolled() {
        pending = nil
        lost = false
    }

    private func keep(_ offset: CGFloat) {
        kept = offset
        height = scroll.map(Self.height(of:)) ?? 0
    }

    /// The offset to get back to: the one restored, else the kept one the content shrank under.
    private var target: CGFloat? {
        pending ?? (lost ? kept : nil)
    }

    /// Scrolls as far towards the target as the content allows; done once it is reached.
    private func restore() {
        guard let target, let scroll, scroll.bounds.height > 0 else { return }
        let (top, bottom) = Self.range(of: scroll)
        guard bottom > top else { return }
        restoring = true
        Self.scroll(scroll, to: min(target, bottom))
        restoring = false
        guard bottom >= target else { return }
        keep(target)
        pending = nil
        lost = false
    }

    #if os(macOS)
    private static func offset(of scroll: NSScrollView) -> CGFloat {
        scroll.contentView.bounds.origin.y
    }

    private static func height(of scroll: NSScrollView) -> CGFloat {
        scroll.documentView?.frame.height ?? 0
    }

    private static func range(of scroll: NSScrollView) -> (CGFloat, CGFloat) {
        let top = -scroll.contentInsets.top
        return (top, max(height(of: scroll) - scroll.contentView.bounds.height + scroll.contentInsets.bottom, top))
    }

    private static func scroll(_ scroll: NSScrollView, to y: CGFloat) {
        scroll.contentView.scroll(to: NSPoint(x: scroll.contentView.bounds.origin.x, y: y))
        scroll.reflectScrolledClipView(scroll.contentView)
    }
    #else
    // Offsets are from the content's top, under the bars, so they mean the same whatever the bars' height.
    private static func offset(of scroll: UIScrollView) -> CGFloat {
        scroll.contentOffset.y + scroll.adjustedContentInset.top
    }

    private static func height(of scroll: UIScrollView) -> CGFloat {
        scroll.contentSize.height
    }

    private static func range(of scroll: UIScrollView) -> (CGFloat, CGFloat) {
        let insets = scroll.adjustedContentInset
        return (0, max(height(of: scroll) - scroll.bounds.height + insets.top + insets.bottom, 0))
    }

    private static func scroll(_ scroll: UIScrollView, to y: CGFloat) {
        scroll.contentOffset = CGPoint(x: scroll.contentOffset.x, y: y - scroll.adjustedContentInset.top)
    }
    #endif
}

/// Put in the background of a SwiftUI scroll view's content, keeps that scroll view's place.
struct ScrollKept {
    let restore: CGFloat
    let report: (CGFloat) -> Void
}

#if os(macOS)
extension ScrollKept: NSViewRepresentable {
    func makeCoordinator() -> ScrollKeeper { ScrollKeeper(restore: restore, report: report) }

    func makeNSView(context: Context) -> ScrollFinder {
        let view = ScrollFinder()
        view.found = { scroll in context.coordinator.attach(scroll) }
        return view
    }

    func updateNSView(_ view: ScrollFinder, context: Context) {}
}

/// Finds the scroll view it was put in once it is in the window.
final class ScrollFinder: NSView {
    var found: ((NSScrollView) -> Void)?

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard window != nil, let scroll = enclosingScrollView else { return }
        found?(scroll)
    }
}
#else
extension ScrollKept: UIViewRepresentable {
    func makeCoordinator() -> ScrollKeeper { ScrollKeeper(restore: restore, report: report) }

    func makeUIView(context: Context) -> ScrollFinder {
        let view = ScrollFinder()
        view.found = { scroll in context.coordinator.attach(scroll) }
        return view
    }

    func updateUIView(_ view: ScrollFinder, context: Context) {}
}

/// Finds the scroll view it was put in once it is in the window.
final class ScrollFinder: UIView {
    var found: ((UIScrollView) -> Void)?

    override func didMoveToWindow() {
        super.didMoveToWindow()
        guard window != nil else { return }
        var view = superview
        while let current = view, !(current is UIScrollView) { view = current.superview }
        guard let scroll = view as? UIScrollView else { return }
        found?(scroll)
    }
}
#endif
