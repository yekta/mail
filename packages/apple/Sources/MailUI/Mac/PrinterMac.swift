#if os(macOS)
import AppKit
import WebKit

/// Prints a page: a web view lays it out out of sight, then the print panel comes down over the
/// window.
@MainActor
enum Printer {
    private static var jobs: [PrintJob] = []

    static func print(html: String) {
        let job = PrintJob()
        jobs.append(job)
        job.start(html) { jobs.removeAll { $0 === job } }
    }
}

@MainActor
private final class PrintJob: NSObject, WKNavigationDelegate {
    private let webView = WKWebView(frame: NSRect(x: 0, y: 0, width: 680, height: 900))
    private var finished: () -> Void = {}

    func start(_ html: String, finished: @escaping () -> Void) {
        self.finished = finished
        webView.navigationDelegate = self
        webView.loadHTMLString(html, baseURL: nil)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        let info = NSPrintInfo.shared
        info.horizontalPagination = .fit
        info.verticalPagination = .automatic
        info.isVerticallyCentered = false
        let operation = webView.printOperation(with: info)
        operation.view?.frame = webView.bounds
        guard let window = NSApp.keyWindow ?? NSApp.mainWindow else {
            operation.run()
            finished()
            return
        }
        operation.runModal(for: window, delegate: self, didRun: #selector(printed), contextInfo: nil)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        finished()
    }

    @objc private func printed() {
        finished()
    }
}
#endif
