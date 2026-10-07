#if os(iOS)
import UIKit

/// Prints a page through the system's print sheet.
@MainActor
enum Printer {
    static func print(html: String) {
        let info = UIPrintInfo.printInfo()
        info.outputType = .general
        info.jobName = "Mail"
        let controller = UIPrintInteractionController.shared
        controller.printInfo = info
        controller.printFormatter = UIMarkupTextPrintFormatter(markupText: html)
        controller.present(animated: true)
    }
}
#endif
