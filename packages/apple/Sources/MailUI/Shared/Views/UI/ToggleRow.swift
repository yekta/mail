import SwiftUI

/// A setting that is on or off, with a line saying what it does; the switch is the system's,
/// at the end of the row.
struct ToggleRow: View {
    let title: String
    var detail: String?
    @Binding var isOn: Bool

    var body: some View {
        HStack(alignment: .center, spacing: Space.l) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).textStyle(.body)
                if let detail {
                    Text(detail).textStyle(.caption).fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: Space.s)
            Toggle(isOn: $isOn) { Text(title) }
                .labelsHidden()
                .toggleStyle(.switch)
                .tint(Tokens.primary.color)
                #if os(macOS)
                .controlSize(.small)
                #endif
        }
        .frame(minHeight: ControlSize.small.height * Platform.scale)
        .contentShape(Rectangle())
        .onTapGesture { isOn.toggle() }
    }
}
