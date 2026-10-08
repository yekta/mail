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
@MainActor
final class ScrollKeeper: NSObject {
    private(set) var pending: CGFloat?
    private let report: (CGFloat) -> Void
    private weak var scroll: PlatformScrollView?
    private var restoring = false
    #if os(iOS)
    private var observations: [NSKeyValueObservation] = []
    #endif

    init(restore: CGFloat, report: @escaping (CGFloat) -> Void) {
        pending = restore > 0 ? restore : nil
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

    @objc private func moved() {
        guard !restoring else { return }
        guard pending == nil else {
            restore()
            return
        }
        guard let scroll else { return }
        report(Self.offset(of: scroll))
    }

    @objc private func grew() {
        restore()
    }

    #if os(macOS)
    @objc private func dragged() {
        pending = nil
    }
    #else
    @objc private func panned(_ gesture: UIPanGestureRecognizer) {
        if gesture.state == .began { pending = nil }
    }
    #endif

    /// Scrolls as far towards the kept offset as the content allows; done once it is reached.
    private func restore() {
        guard let pending, let scroll, scroll.bounds.height > 0 else { return }
        let (top, bottom) = Self.range(of: scroll)
        guard bottom > top else { return }
        restoring = true
        Self.scroll(scroll, to: min(pending, bottom))
        restoring = false
        if bottom >= pending { self.pending = nil }
    }

    #if os(macOS)
    private static func offset(of scroll: NSScrollView) -> CGFloat {
        scroll.contentView.bounds.origin.y
    }

    private static func range(of scroll: NSScrollView) -> (CGFloat, CGFloat) {
        let height = scroll.documentView?.frame.height ?? 0
        let top = -scroll.contentInsets.top
        return (top, max(height - scroll.contentView.bounds.height + scroll.contentInsets.bottom, top))
    }

    private static func scroll(_ scroll: NSScrollView, to y: CGFloat) {
        scroll.contentView.scroll(to: NSPoint(x: scroll.contentView.bounds.origin.x, y: y))
        scroll.reflectScrolledClipView(scroll.contentView)
    }
    #else
    private static func offset(of scroll: UIScrollView) -> CGFloat {
        scroll.contentOffset.y
    }

    private static func range(of scroll: UIScrollView) -> (CGFloat, CGFloat) {
        let top = -scroll.adjustedContentInset.top
        return (top, max(scroll.contentSize.height - scroll.bounds.height + scroll.adjustedContentInset.bottom, top))
    }

    private static func scroll(_ scroll: UIScrollView, to y: CGFloat) {
        scroll.contentOffset = CGPoint(x: scroll.contentOffset.x, y: y)
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
