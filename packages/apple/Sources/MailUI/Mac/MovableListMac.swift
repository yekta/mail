#if os(macOS)
import AppKit
import SwiftUI

/// Rows along `axis`, each drawn by `row`, that the list picks up and moves itself: pressed and
/// pulled, a picture of the row shrinks a little and floats under the pointer, over everything
/// in the window, the rows it passes slide out of its way, and let go it slides into its place.
/// Rows of the same `group` change places; the rows that `follow` one go out of sight while it
/// is lifted and come back under it. `moved` says where a row landed: its index among the rows
/// without it and its followers. A click goes to `clicked`, a secondary click opens `menu`, and
/// a row reads `hovered` from the environment. A vertical list takes the room it is given and
/// scrolls; a horizontal one is as wide as its rows, or as wide as it is given when it `fills`,
/// with the rows sharing the width.
struct MovableList<Item: Identifiable, Row: View>: NSViewRepresentable {
    let items: [Item]
    var axis: Axis = .vertical
    /// Room before the first row and after the last.
    var inset: CGFloat = 0
    var fills = false
    var group: (Item) -> String? = { _ in nil }
    var follows: (Item) -> Bool = { _ in false }
    /// The shape a row lifts in: its light, inset from the row's edges.
    var rowInset = EdgeInsets()
    var rowRadius: CGFloat = 0
    var indicators = true
    var clicked: (Item) -> Void = { _ in }
    var moved: (Item, Int) -> Void = { _, _ in }
    var menu: (Item) -> [RowMenuItem] = { _ in [] }
    @ViewBuilder let row: (Item) -> Row
    @Environment(MailStore.self) private var store

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let stack = MovableStack(axis: axis)
        stack.clicked = { [weak coordinator = context.coordinator] in coordinator?.clicked($0) }
        stack.moved = { [weak coordinator = context.coordinator] in coordinator?.moved($0, to: $1) }
        guard axis == .vertical else { return stack }
        let scroll = NSScrollView()
        scroll.documentView = stack
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = indicators
        scroll.autohidesScrollers = true
        scroll.automaticallyAdjustsContentInsets = false
        scroll.contentInsets = NSEdgeInsets(top: inset, left: 0, bottom: inset, right: 0)
        scroll.scrollerInsets = NSEdgeInsets(top: -inset, left: 0, bottom: -inset, right: 0)
        stack.autoresizingMask = [.width]
        return scroll
    }

    func updateNSView(_ view: NSView, context: Context) {
        guard let stack = Self.stack(of: view) else { return }
        let coordinator = context.coordinator
        coordinator.list = self
        coordinator.store = store
        stack.fills = fills
        stack.light = (inset: rowInset, radius: rowRadius, color: Tokens.accent.platform)
        coordinator.show(in: stack)
    }

    static func dismantleNSView(_ view: NSView, coordinator: Coordinator) {
        stack(of: view)?.cancelLift()
    }

    private static func stack(of view: NSView) -> MovableStack? {
        (view as? NSScrollView)?.documentView as? MovableStack ?? view as? MovableStack
    }

    /// A vertical list takes the room it is offered; a horizontal one is as wide as its rows.
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSView, context: Context) -> CGSize? {
        guard axis == .horizontal, let stack = Self.stack(of: nsView) else { return proposal.replacingUnspecifiedDimensions() }
        let content = stack.contentSize
        return CGSize(width: fills ? proposal.width ?? content.width : content.width, height: content.height)
    }

    final class Coordinator {
        var list: MovableList?
        var store: MailStore?

        func show(in stack: MovableStack) {
            guard let list, let store else { return }
            stack.show(list.items.map { item in
                MovableStack.Slot(
                    id: AnyHashable(item.id), group: list.group(item), follows: list.follows(item), menu: list.menu(item),
                    content: AnyView(list.row(item).environment(store))
                )
            })
        }

        func clicked(_ slot: Int) {
            guard let list, list.items.indices.contains(slot) else { return }
            list.clicked(list.items[slot])
        }

        func moved(_ slot: Int, to index: Int) {
            guard let list, list.items.indices.contains(slot) else { return }
            list.moved(list.items[slot], index)
        }
    }
}

/// The rows, each in a hosting view, laid along the axis. The pointer is its, not the rows': a
/// row pressed and pulled 3pt is lifted, one pressed and let go is clicked, and a secondary
/// click opens the row's menu.
final class MovableStack: NSView {
    struct Slot {
        let id: AnyHashable
        let group: String?
        let follows: Bool
        let menu: [RowMenuItem]
        let content: AnyView
    }

    let axis: Axis
    var fills = false
    /// What a lifted row lies on: its light, inset from the row's edges.
    var light = (inset: EdgeInsets(), radius: CGFloat(0), color: NSColor.clear)
    var clicked: (Int) -> Void = { _ in }
    /// The row at `from` was let go where `index` is among the rows without it and its followers.
    var moved: (_ from: Int, _ index: Int) -> Void = { _, _ in }
    private var slots: [Slot] = []
    private var hosts: [NSHostingView<AnyView>] = []
    /// The rows out of sight while one is lifted: the ones that follow it.
    private var vanished: Range<Int>?
    private var hovered = -1
    private var pressed: Int?
    private var start = NSPoint.zero
    private var lift: Lift?

    init(axis: Axis) {
        self.axis = axis
        super.init(frame: .zero)
        wantsLayer = true
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    override var isFlipped: Bool { true }

    override var acceptsFirstResponder: Bool { false }

    /// Every click in the stack is the stack's: the rows don't take any.
    override func hitTest(_ point: NSPoint) -> NSView? {
        guard let superview, bounds.contains(convert(point, from: superview)) else { return nil }
        return self
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        guard let index = slot(at: convert(event.locationInWindow, from: nil)), !slots[index].menu.isEmpty else { return nil }
        let menu = NSMenu()
        for (position, item) in slots[index].menu.enumerated() {
            let entry = NSMenuItem(title: item.title, action: #selector(chose(_:)), keyEquivalent: "")
            entry.target = self
            entry.representedObject = [index, position]
            menu.addItem(entry)
        }
        return menu
    }

    @objc private func chose(_ sender: NSMenuItem) {
        guard let place = sender.representedObject as? [Int], place.count == 2, slots.indices.contains(place[0]),
              slots[place[0]].menu.indices.contains(place[1])
        else { return }
        slots[place[0]].menu[place[1]].perform()
    }

    /// The rows as they are now, reusing the view of a row that was there before. Rows that are
    /// others put a lifted one back.
    func show(_ slots: [Slot]) {
        let same = slots.map(\.id) == self.slots.map(\.id)
        if !same { cancelLift() }
        var kept = Dictionary(zip(self.slots.map(\.id), hosts), uniquingKeysWith: { first, _ in first })
        hosts = slots.map { slot in
            guard let host = kept.removeValue(forKey: slot.id) else {
                let host = NSHostingView(rootView: AnyView(EmptyView()))
                host.sizingOptions = .intrinsicContentSize
                host.safeAreaRegions = []
                host.wantsLayer = true
                addSubview(host)
                return host
            }
            return host
        }
        kept.values.forEach { $0.removeFromSuperview() }
        self.slots = slots
        for index in hosts.indices { hosts[index].rootView = content(index) }
        if same { placeRows() } else { needsLayout = true }
    }

    private func content(_ index: Int) -> AnyView {
        AnyView(slots[index].content.environment(\.hovered, index == hovered))
    }

    /// The rows' ideal size together along the axis.
    var contentSize: CGSize {
        var length: CGFloat = 0
        var cross: CGFloat = 0
        for index in hosts.indices where vanished?.contains(index) != true {
            let size = measure(index)
            length += axis == .vertical ? size.height : size.width
            cross = max(cross, axis == .vertical ? size.width : size.height)
        }
        return axis == .vertical ? CGSize(width: cross, height: length) : CGSize(width: length, height: cross)
    }

    /// A row's ideal size: a row is one line, so its height stands whatever the width.
    private func measure(_ index: Int) -> CGSize {
        hosts[index].intrinsicContentSize
    }

    override func setFrameSize(_ size: NSSize) {
        super.setFrameSize(size)
        needsLayout = true
    }

    override func layout() {
        super.layout()
        placeRows()
    }

    /// Lays the rows along the axis, past the ones out of sight. A vertical stack grows to its
    /// rows for the scroll view around it.
    private func placeRows() {
        var offset: CGFloat = 0
        let shown = hosts.indices.filter { vanished?.contains($0) != true }
        for index in shown {
            let size = measure(index)
            switch axis {
            case .vertical:
                hosts[index].frame = CGRect(x: 0, y: offset, width: bounds.width, height: size.height)
                offset += size.height
            case .horizontal:
                let width = fills ? bounds.width / CGFloat(max(shown.count, 1)) : size.width
                hosts[index].frame = CGRect(x: offset, y: 0, width: width, height: bounds.height)
                offset += width
            }
        }
        guard axis == .vertical, frame.height != offset else { return }
        setFrameSize(NSSize(width: frame.width, height: offset))
    }

    // MARK: The pointer

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: bounds, options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self))
    }

    override func mouseMoved(with event: NSEvent) {
        hover(slot(at: convert(event.locationInWindow, from: nil)) ?? -1)
    }

    override func mouseExited(with event: NSEvent) {
        hover(-1)
    }

    private func hover(_ index: Int) {
        guard index != hovered, lift == nil else { return }
        let before = hovered
        hovered = index
        for index in [before, index] where hosts.indices.contains(index) {
            hosts[index].rootView = content(index)
        }
    }

    private func slot(at point: NSPoint) -> Int? {
        hosts.firstIndex { !$0.isHidden && $0.frame.contains(point) }
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        guard lift == nil, let index = slot(at: point) else {
            super.mouseDown(with: event)
            return
        }
        pressed = index
        start = point
    }

    override func mouseDragged(with event: NSEvent) {
        guard let pressed else { return }
        if lift == nil {
            let point = convert(event.locationInWindow, from: nil)
            guard slots[pressed].group != nil, hypot(point.x - start.x, point.y - start.y) >= 3 else { return }
            let follows = slots.map(\.follows)
            let lifted = Landings.lifted(from: pressed, follows: follows)
            vanish(lifted.dropFirst())
            lift = Lift(
                stack: self, hosts: hosts, from: pressed, grip: start, light: light,
                kept: slots.indices.filter { !lifted.contains($0) }, landings: Landings.of(from: pressed, groups: slots.map(\.group), follows: follows)
            )
            guard lift != nil else {
                reappear()
                self.pressed = nil
                return
            }
        }
        lift?.follow(event.locationInWindow)
    }

    override func mouseUp(with event: NSEvent) {
        guard let pressed else { return }
        self.pressed = nil
        guard let lift else {
            let point = convert(event.locationInWindow, from: nil)
            if hosts[pressed].frame.contains(point) { clicked(pressed) }
            return
        }
        lift.drop { [weak self] from, index in
            self?.lift = nil
            self?.landed(from, at: index)
        }
    }

    /// Puts the rows out of sight and closes the gap they leave, at once, so the lift finds the
    /// rows where they will stay.
    private func vanish(_ range: Range<Int>) {
        guard !range.isEmpty else { return }
        vanished = range
        for index in range { hosts[index].isHidden = true }
        placeRows()
    }

    private func reappear() {
        guard let vanished else { return }
        self.vanished = nil
        for index in vanished where hosts.indices.contains(index) { hosts[index].isHidden = false }
        placeRows()
    }

    /// The row at `from` landed where `index` is among the rows without it and its followers.
    /// The rows are put there before anyone hears of it, so nothing jumps while the list is told.
    private func landed(_ from: Int, at index: Int) {
        let lifted = Landings.lifted(from: from, follows: slots.map(\.follows))
        let kept = slots.indices.filter { !lifted.contains($0) }
        let order = Array(kept[..<index]) + Array(lifted) + Array(kept[index...])
        slots = order.map { slots[$0] }
        hosts = order.map { hosts[$0] }
        reappear()
        guard index != kept.filter({ $0 < from }).count else { return }
        moved(from, index)
    }

    /// Puts the rows back where they are laid out, for when the list changes under a lift.
    func cancelLift() {
        pressed = nil
        lift?.cancel()
        lift = nil
        reappear()
    }
}

/// A row on its way: where it was picked up, where it floats, and where it would land. A
/// picture of it floats in a window of its own over the stack's, so that it goes wherever the
/// pointer does, over everything the window shows. The row itself stays in the stack, hidden,
/// and the other rows move with transforms only, so the layout stays as it was until it lands.
private final class Lift {
    private static let scale: CGFloat = 0.96
    private let stack: MovableStack
    private let axis: Axis
    private let window: NSWindow
    private let hosts: [NSView]
    private let from: Int
    /// How far along the axis from the row's start it was picked up.
    private let grip: CGFloat
    /// Every row as it is laid out.
    private let rects: [CGRect]
    /// The rows still in sight, without the lifted one: each landing is a position among them.
    private let kept: [Int]
    private let landings: [Int]
    /// Where the lifted row was, among the kept.
    private let origin: Int
    private var index: Int
    private let host: NSView
    /// Lies over the stack's window, lets the pointer through, and carries the picture.
    private let float: NSWindow
    private let overlay: NSView
    /// The picture on its light.
    private let holder = NSView()
    private let light = NSView()
    /// Where the pointer is from the holder's origin.
    private var offset = CGPoint.zero

    init?(
        stack: MovableStack, hosts: [NSView], from: Int, grip: NSPoint, light shape: (inset: EdgeInsets, radius: CGFloat, color: NSColor),
        kept: [Int], landings: [Int]
    ) {
        guard let window = stack.window, let host = hosts[safe: from], let picture = Self.picture(of: host) else { return nil }
        float = NSWindow(contentRect: window.frame, styleMask: .borderless, backing: .buffered, defer: false)
        guard let overlay = float.contentView else { return nil }
        self.stack = stack
        axis = stack.axis
        self.window = window
        self.overlay = overlay
        self.hosts = hosts
        self.host = host
        self.from = from
        rects = hosts.map(\.frame)
        self.kept = kept
        self.landings = landings
        origin = kept.filter { $0 < from }.count
        index = origin
        self.grip = (axis == .vertical ? grip.y : grip.x) - Self.start(rects[from], axis)

        float.isOpaque = false
        float.backgroundColor = .clear
        float.hasShadow = false
        float.ignoresMouseEvents = true
        float.isReleasedWhenClosed = false
        float.animationBehavior = .none
        float.appearance = window.effectiveAppearance
        overlay.wantsLayer = true
        window.addChildWindow(float, ordered: .above)
        holder.frame = inOverlay(host.convert(host.bounds, to: nil))
        holder.wantsLayer = true
        overlay.addSubview(holder)
        let pointer = inOverlay(stack.convert(grip, to: nil))
        offset = CGPoint(x: pointer.x - holder.frame.minX, y: pointer.y - holder.frame.minY)
        let size = holder.bounds.size
        let inset = shape.inset
        light.frame = CGRect(x: inset.leading, y: inset.bottom, width: size.width - inset.leading - inset.trailing, height: size.height - inset.top - inset.bottom)
        light.wantsLayer = true
        light.layer?.cornerRadius = shape.radius
        holder.effectiveAppearance.performAsCurrentDrawingAppearance { light.layer?.backgroundColor = shape.color.cgColor }
        holder.addSubview(light)
        let image = NSImageView(image: picture)
        image.frame = holder.bounds
        image.imageScaling = .scaleNone
        holder.addSubview(image)
        host.isHidden = true
        guard let layer = holder.layer else { return }
        animate(layer, to: Self.scaled(Self.scale, in: size), duration: 0.15)
    }

    private static func start(_ rect: CGRect, _ axis: Axis) -> CGFloat {
        axis == .vertical ? rect.minY : rect.minX
    }

    private static func end(_ rect: CGRect, _ axis: Axis) -> CGFloat {
        axis == .vertical ? rect.maxY : rect.maxX
    }

    private var size: CGFloat {
        axis == .vertical ? rects[from].height : rects[from].width
    }

    /// A rect of the stack's window in the overlay.
    private func inOverlay(_ rect: CGRect) -> CGRect {
        overlay.convert(float.convertFromScreen(window.convertToScreen(rect)), from: nil)
    }

    /// A point of the stack's window in the overlay.
    private func inOverlay(_ point: CGPoint) -> CGPoint {
        overlay.convert(float.convertPoint(fromScreen: window.convertPoint(toScreen: point)), from: nil)
    }

    /// The row as it is drawn right now.
    private static func picture(of view: NSView) -> NSImage? {
        guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return nil }
        view.cacheDisplay(in: view.bounds, to: rep)
        let image = NSImage(size: view.bounds.size)
        image.addRepresentation(rep)
        return image
    }

    /// Floats the row under the pointer, at `location` in the window, and parts the rows where
    /// it would land.
    func follow(_ location: NSPoint) {
        let pointer = inOverlay(location)
        holder.setFrameOrigin(CGPoint(x: pointer.x - offset.x, y: pointer.y - offset.y))
        let point = stack.convert(location, from: nil)
        let center = (axis == .vertical ? point.y : point.x) - grip + size / 2
        let landing = index
        while let next = landings.first(where: { $0 > index }), center >= middle(of: index..<next) { index = next }
        while let previous = landings.last(where: { $0 < index }), center <= middle(of: previous..<index) { index = previous }
        guard landing != index else { return }
        for slot in kept {
            guard let layer = hosts[safe: slot]?.layer else { continue }
            let shift = shift(slot)
            let translation = axis == .vertical ? CATransform3DMakeTranslation(0, shift, 0) : CATransform3DMakeTranslation(shift, 0, 0)
            animate(layer, to: translation, duration: 0.2)
        }
    }

    /// How far the row is slid out of the way, with the gap where it is.
    private func shift(_ slot: Int) -> CGFloat {
        guard let position = kept.firstIndex(of: slot) else { return 0 }
        return position >= origin && position < index ? -size : position < origin && position >= index ? size : 0
    }

    /// The middle of the rows between two landings, as they are drawn.
    private func middle(of positions: Range<Int>) -> CGFloat {
        let first = kept[positions.lowerBound]
        let last = kept[positions.upperBound - 1]
        return (Self.start(rects[first], axis) + Self.end(rects[last], axis)) / 2 + shift(first)
    }

    /// Slides the row into its place, grows it back and puts out its light, then says where it
    /// landed.
    func drop(_ landed: @escaping (_ from: Int, _ index: Int) -> Void) {
        let start = index < origin ? Self.start(rects[kept[index]], axis) : index == origin ? Self.start(rects[from], axis) : Self.end(rects[kept[index - 1]], axis) - size
        let frame = axis == .vertical ? CGRect(x: rects[from].minX, y: start, width: rects[from].width, height: size) : CGRect(x: start, y: rects[from].minY, width: size, height: rects[from].height)
        let place = inOverlay(stack.convert(frame, to: nil))
        NSAnimationContext.runAnimationGroup { [self] context in
            context.duration = 0.25
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            holder.animator().setFrameOrigin(place.origin)
            light.animator().alphaValue = 0
            guard let layer = holder.layer else { return }
            animate(layer, to: CATransform3DIdentity, duration: 0.25)
        } completionHandler: { [self] in
            cancel()
            landed(from, index)
        }
    }

    /// Shows the row in its place again and puts every row where it is laid out.
    func cancel() {
        host.isHidden = false
        window.removeChildWindow(float)
        float.orderOut(nil)
        for view in hosts {
            guard let layer = view.layer else { continue }
            layer.removeAllAnimations()
            layer.transform = CATransform3DIdentity
        }
    }

    private func animate(_ layer: CALayer, to transform: CATransform3D, duration: CFTimeInterval) {
        let animation = CABasicAnimation(keyPath: "transform")
        animation.fromValue = layer.presentation()?.transform ?? layer.transform
        animation.toValue = transform
        animation.duration = duration
        animation.timingFunction = CAMediaTimingFunction(name: .easeOut)
        layer.transform = transform
        layer.add(animation, forKey: "transform")
    }

    private static func scaled(_ scale: CGFloat, in size: CGSize) -> CATransform3D {
        var transform = CATransform3DMakeTranslation(size.width / 2, size.height / 2, 0)
        transform = CATransform3DScale(transform, scale, scale, 1)
        return CATransform3DTranslate(transform, -size.width / 2, -size.height / 2, 0)
    }
}

private extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
#endif
