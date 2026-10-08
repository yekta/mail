import SwiftUI

/// A section's name in small capitals, and maybe a line about it.
struct SectionHeading: View {
    let title: String
    var detail: String?

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            Text(title.uppercased()).textStyle(.overline)
            if let detail {
                Text(detail).textStyle(.caption).fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// A part of a page: its heading, then its controls.
struct FormSection<Content: View>: View {
    let title: String
    var detail: String?
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            SectionHeading(title: title, detail: detail)
            content
        }
    }
}

/// A rounded card on the page: a form being filled, a thing of its own.
struct Card<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        content
            .padding(Space.l)
            .background(RoundedRectangle(cornerRadius: Theme.cardRadius).fill(Tokens.card.color))
            .overlay(RoundedRectangle(cornerRadius: Theme.cardRadius).strokeBorder(Tokens.border.color, lineWidth: Theme.hairline))
    }
}

/// Save and Cancel under a form.
struct FormButtons: View {
    var save = "Save"
    var saveDisabled = false
    let onSave: () -> Void
    let onCancel: () -> Void

    var body: some View {
        HStack(spacing: 0) {
            ActionButton(title: save, variant: .primary, action: onSave).disabled(saveDisabled)
            ActionButton(title: "Cancel", variant: .ghost, action: onCancel).keyboardShortcut(.cancelAction)
        }
    }
}
