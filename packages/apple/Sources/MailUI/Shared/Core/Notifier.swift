import Foundation
import UserNotifications

#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// New mail as notifications while the app is in the background, a click on one opening its
/// thread; and the unread count of the inboxes on the app's icon.
@MainActor
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
    static let shared = Notifier()

    /// Opens the thread of a clicked notification. One clicked before it is set waits for it.
    var open: ((String) -> Void)? {
        didSet {
            guard let open, let pending else { return }
            self.pending = nil
            open(pending)
        }
    }

    private var pending: String?
    private var asked = false
    private var badge = 0

    /// Notifications need a bundle; a bare `swift run` has none.
    private var center: UNUserNotificationCenter? {
        Bundle.main.bundleIdentifier == nil ? nil : UNUserNotificationCenter.current()
    }

    /// Becomes the notifications' delegate, as the app starts, to hear of a click that launched it.
    func install() {
        center?.delegate = self
    }

    /// Asks once, when there is mail to tell of: after an account is added, not at first launch.
    func askPermission() {
        guard !asked, let center else { return }
        asked = true
        center.requestAuthorization(options: [.alert, .sound, .badge]) { _, _ in }
    }

    func notify(_ messages: [NewMailItem]) {
        guard let center, !Self.frontmost else { return }
        for message in messages.suffix(5) {
            let content = UNMutableNotificationContent()
            content.title = message.from
            content.subtitle = message.subject
            content.body = message.snippet
            content.sound = .default
            content.threadIdentifier = message.thread
            content.userInfo = ["thread": message.thread]
            center.add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
        }
    }

    func setBadge(_ count: Int) {
        guard count != badge else { return }
        badge = count
        #if os(macOS)
        NSApp.dockTile.badgeLabel = count > 0 ? "\(count)" : nil
        #else
        center?.setBadgeCount(count)
        #endif
    }

    private static var frontmost: Bool {
        #if os(macOS)
        NSApp.isActive
        #else
        UIApplication.shared.applicationState == .active
        #endif
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let thread = response.notification.request.content.userInfo["thread"] as? String else { return }
        await MainActor.run {
            guard let open else {
                pending = thread
                return
            }
            open(thread)
        }
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        []
    }
}
