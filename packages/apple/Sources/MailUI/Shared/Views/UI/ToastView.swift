import SwiftUI

/// The note at the bottom: a line of text and maybe an action, gone after a while.
struct ToastView: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        if let toast = store.toast {
            HStack(spacing: Space.l) {
                Text(toast.message).textStyle(.label, color: Tokens.popoverForeground.color)
                if let action = toast.action {
                    PlainButton(action: store.toastTapped) {
                        Text(action).textStyle(.labelStrong, color: Tokens.primary.color)
                    }
                }
            }
            .padding(.horizontal, Space.l + 2)
            .frame(height: Theme.searchHeight * Platform.scale)
                        .background(Capsule().fill(Tokens.popover.color).shadow(color: Tokens.shadow.opacity(Tokens.shadowOpacity).color, radius: 12, y: 4))
            .overlay(Capsule().strokeBorder(Tokens.border.color, lineWidth: Theme.hairline))
            .padding(.bottom, Space.xl)
            .task(id: toast.id) {
                try? await Task.sleep(for: .seconds(toast.duration))
                store.dismissToast(toast)
            }
        }
    }
}
