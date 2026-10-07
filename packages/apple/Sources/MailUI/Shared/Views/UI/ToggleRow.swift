import SwiftUI

/// A setting that is on or off, with a line saying what it does.
struct ToggleRow: View {
    let title: String
    var detail: String?
    @Binding var isOn: Bool

    var body: some View {
        Toggle(isOn: $isOn) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.ui(14)).foregroundStyle(Tokens.foreground.color)
                if let detail {
                    Text(detail).font(.ui(12)).foregroundStyle(Tokens.mutedForeground.color)
                }
            }
        }
        .toggleStyle(.switch)
        .tint(Tokens.primary.color)
        #if os(macOS)
        .controlSize(.small)
        #endif
    }
}
