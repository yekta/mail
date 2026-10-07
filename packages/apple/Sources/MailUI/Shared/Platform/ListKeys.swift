import SwiftUI

#if os(macOS)
import AppKit
#endif

extension View {
    /// ↑ and ↓ move through a list under a field (`-1`, `1`), Esc leaves it. The Mac's text
    /// fields keep the arrows to themselves, so there they are read before the field gets them.
    func onListKeys(move: @escaping (Int) -> Void, escape: @escaping () -> Void) -> some View {
        #if os(macOS)
        background(ListKeyMonitor(move: move, escape: escape))
        #else
        onKeyPress(.upArrow) {
            move(-1)
            return .handled
        }
        .onKeyPress(.downArrow) {
            move(1)
            return .handled
        }
        .onKeyPress(.escape) {
            escape()
            return .handled
        }
        #endif
    }
}

#if os(macOS)
private struct ListKeyMonitor: NSViewRepresentable {
    let move: (Int) -> Void
    let escape: () -> Void

    final class Coordinator {
        var monitor: Any?
        var move: (Int) -> Void = { _ in }
        var escape: () -> Void = {}
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        let coordinator = context.coordinator
        coordinator.monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak view] event in
            guard let window = view?.window, event.window === window else { return event }
            let flags = event.modifierFlags.intersection([.command, .control, .option, .shift])
            guard flags.isEmpty else { return event }
            switch event.keyCode {
            case 125: coordinator.move(1)
            case 126: coordinator.move(-1)
            case 53: coordinator.escape()
            default: return event
            }
            return nil
        }
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {
        context.coordinator.move = move
        context.coordinator.escape = escape
    }

    static func dismantleNSView(_ view: NSView, coordinator: Coordinator) {
        if let monitor = coordinator.monitor { NSEvent.removeMonitor(monitor) }
    }
}
#endif
