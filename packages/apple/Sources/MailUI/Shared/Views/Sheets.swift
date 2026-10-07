import SwiftUI

/// A sheet over the window. One at a time: when one turns into another (the palette into the
/// snooze times), the first goes and the second comes.
enum Modal: Identifiable {
    case palette(PaletteScope)
    case label(Labeling)
    case snooze([String])
    case shortcuts
    case settings

    var id: String {
        switch self {
        case .palette(let scope): "palette/\(scope)"
        case .label(let labeling): "label/\(labeling.id)"
        case .snooze(let threads): "snooze/\(threads.joined(separator: ","))"
        case .shortcuts: "shortcuts"
        case .settings: "settings"
        }
    }
}

extension MailStore {
    var modal: Modal? {
        get {
            if let palette { return .palette(palette) }
            if let labeling { return .label(labeling) }
            if let snoozing { return .snooze(snoozing) }
            if shortcutsOpen { return .shortcuts }
            return settingsOpen ? .settings : nil
        }
        set {
            guard newValue == nil else { return }
            palette = nil
            labeling = nil
            snoozing = nil
            shortcutsOpen = false
            settingsOpen = false
        }
    }
}

extension View {
    /// The sheets and the questions both apps show over everything.
    func mailSheets(_ store: MailStore) -> some View {
        modifier(MailSheets(store: store))
    }
}

private struct MailSheets: ViewModifier {
    @Bindable var store: MailStore

    func body(content: Content) -> some View {
        content
            .sheet(item: $store.modal) { modal in
                Group {
                    switch modal {
                    case .palette(let scope): CommandPalette(scope: scope)
                    case .label(let labeling): LabelPicker(labeling: labeling)
                    case .snooze(let threads): SnoozePicker(threads: threads)
                    case .shortcuts: ShortcutsView()
                    case .settings: SettingsView()
                    }
                }
                .environment(store)
                #if os(iOS)
                .presentationDetents(detents(modal))
                #endif
            }
            .alert(
                store.confirmation?.title ?? "", isPresented: Binding(get: { store.confirmation != nil }, set: { if !$0 { store.confirmation = nil } }),
                presenting: store.confirmation
            ) { confirmation in
                Button(confirmation.action, role: .destructive, action: confirmation.perform)
                Button("Cancel", role: .cancel) {}
            } message: { confirmation in
                Text(confirmation.message)
            }
    }

    #if os(iOS)
    private func detents(_ modal: Modal) -> Set<PresentationDetent> {
        switch modal {
        case .snooze, .label: [.medium, .large]
        default: [.large]
        }
    }
    #endif
}
