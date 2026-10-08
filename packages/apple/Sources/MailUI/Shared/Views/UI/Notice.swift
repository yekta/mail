import SwiftUI

/// A line about something: a hint under a field, an error, a note that something was left out.
/// With `action`, a button at its end does the one thing about it.
struct Notice: View {
    enum Tone {
        case muted, error
    }

    let text: String
    var tone: Tone = .muted
    var symbol: Symbol?
    var action: String?
    var perform: () -> Void = {}

    var body: some View {
        HStack(spacing: Space.s) {
            if let symbol {
                Image(symbol, size: 13).foregroundStyle(color)
            }
            Text(text).textStyle(.caption, color: color).fixedSize(horizontal: false, vertical: true)
            if let action {
                ActionButton(title: action, variant: .ghost, action: perform)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var color: Color {
        tone == .error ? Tokens.destructive.color : Tokens.mutedMoreForeground.color
    }
}

/// A list with nothing in it: an icon and a calm line.
struct EmptyState: View {
    let symbol: Symbol
    let message: String

    var body: some View {
        VStack(spacing: Space.m) {
            Image(symbol, size: 30).foregroundStyle(Tokens.mutedMostForeground.color)
            Text(message).textStyle(.body, color: Tokens.mutedMoreForeground.color)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
