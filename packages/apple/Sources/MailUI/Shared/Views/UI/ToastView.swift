import SwiftUI

/// The note at the bottom: a line of text and maybe an action, gone after a while.
struct ToastView: View {
    @Environment(MailStore.self) private var store

    var body: some View {
        if let toast = store.toast {
            HStack(spacing: 16) {
                Text(toast.message).font(.ui(13))
                if let action = toast.action {
                    Button(action: store.toastTapped) {
                        Text(action).font(.ui(13, .semibold)).foregroundStyle(Tokens.primary.color)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 18)
            .frame(height: 40 * Platform.scale)
            .foregroundStyle(Tokens.popoverForeground.color)
            .background(Capsule().fill(Tokens.popover.color).shadow(color: .black.opacity(0.18), radius: 12, y: 4))
            .overlay(Capsule().strokeBorder(Tokens.border.color, lineWidth: 1))
            .padding(.bottom, 20)
            .task(id: toast.id) {
                try? await Task.sleep(for: .seconds(toast.duration))
                store.dismissToast(toast)
            }
        }
    }
}
