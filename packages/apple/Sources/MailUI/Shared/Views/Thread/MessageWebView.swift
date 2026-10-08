import SwiftUI
import WebKit

/// The web views that draw message bodies, kept and reused: making one is the slow part. Their
/// store is on disk, so a message's images come from the cache the next time the app opens.
/// A page wider than the view (designed mail is often a fixed 600px) is scaled down to fit it.
@MainActor
final class WebViewPool {
    static let shared = WebViewPool()

    private var spare: [MessageWKWebView] = []
    private let heights = HeightReporter()
    private lazy var configuration: WKWebViewConfiguration = {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .default()
        configuration.defaultWebpagePreferences.allowsContentJavaScript = false
        let script = """
            (function () {
              function fit(body) {
                body.style.width = '';
                body.style.transform = '';
                var wide = document.documentElement.scrollWidth, shown = document.documentElement.clientWidth;
                if (wide <= shown + 1) { return; }
                body.style.width = wide + 'px';
                body.style.transformOrigin = '0 0';
                body.style.transform = 'scale(' + shown / wide + ')';
              }
              function report() {
                var body = document.body;
                if (body) { fit(body); }
                var height = Math.ceil(body ? body.getBoundingClientRect().height : 0);
                window.webkit.messageHandlers.height.postMessage(height);
              }
              if (document.body) { new ResizeObserver(report).observe(document.body); }
              document.addEventListener('toggle', report, true);
              window.addEventListener('load', report);
              window.addEventListener('resize', report);
              report();
            })();
            """
        configuration.userContentController.addUserScript(WKUserScript(source: script, injectionTime: .atDocumentEnd, forMainFrameOnly: true, in: .defaultClient))
        configuration.userContentController.add(heights, contentWorld: .defaultClient, name: "height")
        return configuration
    }()

    /// Starts WebKit before the first message shows, so the first card isn't the one that waits.
    func warm() {
        guard spare.isEmpty else { return }
        give(take())
    }

    /// Loads these pages into spare views ahead of their cards, so they are parsed by the time
    /// the cards take them.
    func preload(_ pages: [String]) {
        for html in pages.prefix(8) {
            let view = take()
            view.show(html)
            spare.append(view)
        }
    }

    /// A spare view: the one already showing `html` when there is one, else an empty one.
    func take(_ html: String? = nil) -> MessageWKWebView {
        let index = spare.lastIndex { $0.loaded == html } ?? spare.lastIndex { $0.loaded == nil } ?? spare.indices.last
        if let index { return spare.remove(at: index) }
        let view = MessageWKWebView(frame: .zero, configuration: configuration)
        view.navigationDelegate = view
        #if os(macOS)
        view.setValue(false, forKey: "drawsBackground")
        #else
        view.isOpaque = false
        view.backgroundColor = .clear
        view.scrollView.backgroundColor = .clear
        view.scrollView.isScrollEnabled = false
        view.scrollView.bounces = false
        #endif
        return view
    }

    func give(_ view: MessageWKWebView) {
        view.onHeight = nil
        view.loaded = nil
        view.original = false
        view.loadHTMLString("", baseURL: nil)
        if spare.count < 8 { spare.append(view) }
    }
}

/// Passes each page's height to the web view that shows it.
private final class HeightReporter: NSObject, WKScriptMessageHandler {
    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        guard let height = message.body as? NSNumber, let view = message.webView as? MessageWKWebView else { return }
        view.onHeight?(CGFloat(height.doubleValue))
    }
}

final class MessageWKWebView: WKWebView, WKNavigationDelegate {
    var onHeight: ((CGFloat) -> Void)?
    var loaded: String?
    /// The page shown as its sender made it: the `dark` class the core put on its body taken off.
    var original = false { didSet { if original != oldValue { applyOriginal() } } }

    func show(_ html: String) {
        guard loaded != html else { return }
        loaded = html
        loadHTMLString(html, baseURL: nil)
    }

    private func applyOriginal() {
        let script = "document.body && document.body.classList.toggle('dark', \(original ? "false" : "true"));"
        evaluateJavaScript(script, in: nil, in: .defaultClient) { _ in }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        if original { applyOriginal() }
    }

    /// Links open in the browser; the card never navigates.
    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
        guard action.navigationType == .linkActivated, let url = action.request.url else {
            decisionHandler(action.request.url?.scheme == "about" || action.navigationType == .other ? .allow : .cancel)
            return
        }
        Platform.open(url)
        decisionHandler(.cancel)
    }

    #if os(macOS)
    /// The thread scrolls, not the card.
    override func scrollWheel(with event: NSEvent) {
        nextResponder?.scrollWheel(with: event)
    }
    #endif
}

/// A message body, as tall as its page. It opens at the height the store kept for it, when
/// it was last measured at this width, so the thread is laid out before the page is.
/// `original` shows a designed mail as its sender made it.
struct MessageWebView: View {
    @Environment(MailStore.self) private var store
    let message: String
    let html: String
    var original = false
    @State private var height: CGFloat
    @State private var measured = false

    init(message: String, html: String, original: Bool = false, kept: CGFloat? = nil) {
        self.message = message
        self.html = html
        self.original = original
        _height = State(initialValue: kept ?? 40)
    }

    var body: some View {
        WebRepresentable(html: html, original: original, height: $height) { width, reported in
            measured = true
            store.noteBodyHeight(message, width: width, height: reported)
        }
        .frame(height: height)
        .background {
            GeometryReader { geometry in
                Color.clear.onAppear { restore(width: geometry.size.width) }
            }
        }
    }

    /// The kept height was measured at one width; at another it is no guide, and the card
    /// starts small until its page reports.
    private func restore(width: CGFloat) {
        guard !measured else { return }
        height = store.bodyHeight(message, width: width) ?? 40
    }
}

#if os(macOS)
private struct WebRepresentable: NSViewRepresentable {
    let html: String
    let original: Bool
    @Binding var height: CGFloat
    let reported: (CGFloat, CGFloat) -> Void

    func makeNSView(context: Context) -> MessageWKWebView {
        let view = WebViewPool.shared.take(html)
        view.onHeight = { [weak view] measured in
            DispatchQueue.main.async {
                reported(view?.bounds.width ?? 0, measured)
                if abs(height - measured) > 0.5 { height = max(measured, 20) }
            }
        }
        view.original = original
        view.show(html)
        return view
    }

    func updateNSView(_ view: MessageWKWebView, context: Context) {
        view.original = original
        view.show(html)
    }

    static func dismantleNSView(_ view: MessageWKWebView, coordinator: ()) {
        WebViewPool.shared.give(view)
    }
}
#else
private struct WebRepresentable: UIViewRepresentable {
    let html: String
    let original: Bool
    @Binding var height: CGFloat
    let reported: (CGFloat, CGFloat) -> Void

    func makeUIView(context: Context) -> MessageWKWebView {
        let view = WebViewPool.shared.take(html)
        view.onHeight = { [weak view] measured in
            DispatchQueue.main.async {
                reported(view?.bounds.width ?? 0, measured)
                if abs(height - measured) > 0.5 { height = max(measured, 20) }
            }
        }
        view.original = original
        view.show(html)
        return view
    }

    func updateUIView(_ view: MessageWKWebView, context: Context) {
        view.original = original
        view.show(html)
    }

    static func dismantleUIView(_ view: MessageWKWebView, coordinator: ()) {
        WebViewPool.shared.give(view)
    }
}
#endif
