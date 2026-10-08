import SwiftUI

/// How big a sheet is: on the Mac a window of the size, on iOS how far up the screen it comes.
enum SheetSize {
    /// A short list: labels, times, a person.
    case small
    /// A list to scroll: the palette.
    case medium
    /// A page: settings, the shortcuts.
    case large

    #if os(macOS)
    var frame: CGSize {
        switch self {
        case .small: CGSize(width: 400, height: 420)
        case .medium: CGSize(width: 560, height: 460)
        case .large: CGSize(width: 560, height: 640)
        }
    }
    #else
    var detents: Set<PresentationDetent> {
        self == .small ? [.medium, .large] : [.large]
    }
    #endif
}

/// What every sheet is made of: its title, a way to close it, and its content. On iOS the title
/// and the close are the navigation bar's, as the system draws them; on the Mac a header bar
/// of ours, closed with Esc. `trailing` goes in the bar at its end.
struct Sheet<Content: View, Trailing: View>: View {
    @Environment(\.dismiss) private var dismiss
    let title: String
    var size: SheetSize = .small
    /// What the close says when "Close" is not it: "Cancel" over a message being written.
    var close = "Close"
    /// Where the content sits: the popover colour for lists, the page colour for forms.
    var background: ThemeColor = Tokens.popover
    @ViewBuilder let content: Content
    @ViewBuilder var trailing: Trailing

    var body: some View {
        #if os(macOS)
        VStack(spacing: 0) {
            HeaderBar(title: title) {
                IconButton(symbol: .x, help: "Close (Esc)", circled: false) { dismiss() }
                    .keyboardShortcut(.cancelAction)
            } trailing: {
                trailing
            }
            Rule()
            content
        }
        .frame(width: size.frame.width, height: size.frame.height)
        .background(background.color)
        #else
        NavigationStack {
            content
                .background(background.color)
                .navigationTitle(title)
                .navigationBarTitleDisplayMode(.inline)
                .toolbarBackground(background.color, for: .navigationBar)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button(close) { dismiss() }
                    }
                    ToolbarItem(placement: .primaryAction) { trailing }
                }
        }
        .tint(Tokens.primary.color)
        .presentationDetents(size.detents)
        .presentationDragIndicator(.hidden)
        #endif
    }
}

extension Sheet where Trailing == EmptyView {
    init(title: String, size: SheetSize = .small, close: String = "Close", background: ThemeColor = Tokens.popover, @ViewBuilder content: () -> Content) {
        self.init(title: title, size: size, close: close, background: background, content: content, trailing: { EmptyView() })
    }
}

/// A bar across the top of a page or a sheet on the Mac: what leads, the title in the middle,
/// and what trails.
struct HeaderBar<Leading: View, Trailing: View>: View {
    var title: String?
    @ViewBuilder let leading: Leading
    @ViewBuilder let trailing: Trailing

    var body: some View {
        HStack(spacing: Space.s + 2) {
            leading
            Spacer()
            trailing
        }
        .overlay {
            if let title {
                Text(title).textStyle(.subheading).lineLimit(1).allowsHitTesting(false)
            }
        }
        .padding(.horizontal, Space.l)
        .frame(height: Theme.barHeight)
    }
}
