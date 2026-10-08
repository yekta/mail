#if os(macOS)
import SwiftUI

/// A new version of the app, at the foot of the sidebar: the offer, the download as it goes, and
/// the restart that finishes it. Its icon lines up with the sidebar rows'.
struct UpdateRow: View {
    let updater: AppUpdater

    var body: some View {
        switch updater.state {
        case .idle:
            EmptyView()
        case .checking:
            line("Checking for updates", symbol: .refreshCw) { spinner }
        case .upToDate:
            line("Up to date", detail: "Wonnet \(updater.current)", symbol: .circleCheck, tint: Tokens.success.color)
        case .available(let version):
            line(
                "New version", detail: "Wonnet \(version)", symbol: .circleArrowDown, tint: Tokens.success.color,
                action: ActionButton(title: "Update", variant: .primary, size: .small) { updater.install() }
            )
        case .downloading(let version, let fraction):
            line("Downloading", detail: "Wonnet \(version)", symbol: .circleArrowDown, progress: fraction) {
                Text("\(Int(fraction * 100))%").textStyle(.caption).monospacedDigit()
            }
        case .installing(let version):
            line("Installing", detail: "Wonnet \(version)", symbol: .circleArrowDown) { spinner }
        case .ready(let version):
            line(
                "Installed", detail: "Restart to use Wonnet \(version)", symbol: .circleCheck, tint: Tokens.success.color,
                action: ActionButton(title: "Restart", variant: .primary, size: .small) { updater.relaunch() }
            )
        case .failed(let message):
            line(
                "Update failed", detail: message, symbol: .circleAlert, tint: Tokens.destructive.color,
                action: ActionButton(title: "Try again", size: .small) { updater.retry() }
            )
        }
    }

    private var spinner: some View {
        ProgressView().controlSize(.mini)
    }

    /// The icon and the title, with `accessory` at the end; under them the detail, the download's
    /// bar and the button.
    private func line<Accessory: View>(
        _ title: String, detail: String? = nil, symbol: Symbol, tint: Color = Tokens.mutedForeground.color, progress: Double? = nil,
        action: ActionButton? = nil, @ViewBuilder accessory: () -> Accessory = { EmptyView() }
    ) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: Space.m) {
                Image(symbol, size: 15).foregroundStyle(tint).frame(width: 18)
                Text(title).textStyle(.label).lineLimit(1)
                Spacer(minLength: Space.xs)
                accessory()
            }
            .frame(minHeight: ControlSize.small.height)
            Group {
                if let detail {
                    Text(detail).textStyle(.caption).fixedSize(horizontal: false, vertical: true)
                }
                if let progress {
                    ProgressView(value: progress).progressViewStyle(.linear).controlSize(.small).tint(Tokens.mutedForeground.color)
                }
                if let action {
                    // The pill's own gap is taken back, so its edge lines up with the words.
                    action.padding(.top, Space.s - 2).padding(.leading, -Theme.buttonGap / 2)
                }
            }
            .padding(.leading, 18 + Space.m)
        }
    }
}
#endif
