#if os(iOS)
import SwiftUI
import UIKit

/// The sidebar under the list: swiping right slides the list aside as a card and shows the
/// sidebar, swiping left or a tap on what is left of the card closes it. The card always stays
/// partly in view. `enabled` is false while a swipe to the right is someone else's, like going
/// back from a thread.
struct DrawerView<Sidebar: View, Content: View>: UIViewControllerRepresentable {
    @Binding var isOpen: Bool
    let enabled: Bool
    @ViewBuilder let sidebar: Sidebar
    @ViewBuilder let content: Content

    func makeUIViewController(context: Context) -> DrawerController {
        let controller = DrawerController(sidebar: UIHostingController(rootView: AnyView(sidebar)), content: UIHostingController(rootView: AnyView(content)))
        controller.setOpen(isOpen, animated: false)
        return controller
    }

    func updateUIViewController(_ controller: DrawerController, context: Context) {
        (controller.sidebar as? UIHostingController<AnyView>)?.rootView = AnyView(sidebar)
        (controller.content as? UIHostingController<AnyView>)?.rootView = AnyView(content)
        controller.onChange = { open in
            guard isOpen != open else { return }
            isOpen = open
        }
        controller.enabled = enabled
        controller.follow(isOpen)
    }
}

/// A view the drawer leaves a swipe to the right to while `ownsRightSwipe` is true.
protocol OwnsRightSwipe: AnyObject {
    var ownsRightSwipe: Bool { get }
}

final class DrawerController: UIViewController, UIGestureRecognizerDelegate {
    private static let widest: CGFloat = 340
    /// What the sidebar leaves of the card on a phone.
    private static let peek: CGFloat = 56

    let sidebar: UIViewController
    let content: UIViewController
    var onChange: ((Bool) -> Void)?
    var enabled = true

    private let card = UIView()
    private let cardEdge = UIView()
    /// Behind the card, since the card clips: it casts the card's shadow over the sidebar.
    private let cardShadow = UIView()
    /// Over the card while the sidebar shows: it dims the list and takes the tap that closes.
    private let shade = UIControl()
    private let pan = UIPanGestureRecognizer()
    private(set) var open = false
    /// How far the card is aside, from 0 to 1.
    private var progress: CGFloat = 0
    private var progressAtStart: CGFloat = 0

    init(sidebar: UIViewController, content: UIViewController) {
        self.sidebar = sidebar
        self.content = content
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    private var sidebarWidth: CGFloat {
        min(Self.widest, view.bounds.width - Self.peek)
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Tokens.background.platform

        addChild(sidebar)
        sidebar.view.backgroundColor = .clear
        view.addSubview(sidebar.view)
        sidebar.didMove(toParent: self)

        card.clipsToBounds = true
        card.backgroundColor = Tokens.background.platform
        cardShadow.isUserInteractionEnabled = false
        cardShadow.layer.shadowColor = UIColor.black.cgColor
        cardShadow.layer.shadowOffset = .zero
        cardShadow.layer.shadowRadius = 12
        paintCardShadow()
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) { (controller: Self, _: UITraitCollection) in
            controller.paintCardShadow()
        }
        view.addSubview(cardShadow)
        view.addSubview(card)
        addChild(content)
        content.view.backgroundColor = Tokens.background.platform
        card.addSubview(content.view)
        content.didMove(toParent: self)

        shade.backgroundColor = Tokens.background.platform
        shade.alpha = 0
        shade.isHidden = true
        shade.addAction(UIAction { [weak self] _ in self?.setOpen(false, animated: true) }, for: .touchUpInside)
        card.addSubview(shade)
        cardEdge.backgroundColor = Tokens.border.platform
        cardEdge.isUserInteractionEnabled = false
        card.addSubview(cardEdge)

        pan.addTarget(self, action: #selector(panned(_:)))
        pan.delegate = self
        view.addGestureRecognizer(pan)
    }

    private func paintCardShadow() {
        cardShadow.layer.shadowOpacity = traitCollection.userInterfaceStyle == .dark ? 0.5 : 0.16
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        place()
    }

    private func place() {
        let bounds = view.bounds
        sidebar.view.frame = CGRect(x: 0, y: 0, width: sidebarWidth, height: bounds.height)
        card.frame = CGRect(x: progress * sidebarWidth, y: 0, width: bounds.width, height: bounds.height)
        content.view.frame = card.bounds
        cardEdge.frame = CGRect(x: 0, y: 0, width: Theme.hairline, height: bounds.height)
        cardEdge.alpha = min(1, progress * 6)
        if cardShadow.bounds.size != card.bounds.size {
            cardShadow.layer.shadowPath = UIBezierPath(rect: card.bounds).cgPath
        }
        cardShadow.frame = card.frame
        cardShadow.alpha = min(1, progress * 4)
        shade.frame = card.bounds
        shade.alpha = 0.55 * progress
        shade.isHidden = progress == 0
        sidebar.view.isHidden = progress <= 0
    }

    /// Opens or closes as the store says, unless a finger is moving the card.
    func follow(_ wanted: Bool) {
        guard pan.state != .began, pan.state != .changed, wanted != open else { return }
        setOpen(wanted, animated: true)
    }

    func setOpen(_ wanted: Bool, animated: Bool, velocity: CGFloat = 0) {
        let target: CGFloat = wanted ? 1 : 0
        guard open != wanted || progress != target else { return }
        open = wanted
        if wanted { view.endEditing(true) }
        onChange?(wanted)
        guard animated, view.window != nil else {
            stopSliding()
            progress = target
            return place()
        }
        let distance = abs(target - progress)
        let speed = min(12 * distance, abs(velocity) / sidebarWidth)
        slide = Slide(target: target, offset: progress - target, speed: velocity < 0 ? -speed : speed, start: CACurrentMediaTime())
        guard slideLink == nil else { return }
        let link = CADisplayLink(target: self, selector: #selector(slid(_:)))
        link.add(to: .main, forMode: .common)
        slideLink = link
    }

    /// A critically damped spring, followed frame by frame so the shade and the shadow follow it.
    private struct Slide {
        static let stiffness: CGFloat = 22
        let target: CGFloat
        let offset: CGFloat
        let speed: CGFloat
        let start: CFTimeInterval

        func at(_ time: CFTimeInterval) -> (progress: CGFloat, speed: CGFloat) {
            let t = CGFloat(time - start)
            let k = Self.stiffness
            let decay = exp(-k * t)
            let drift = speed + k * offset
            return (target + (offset + drift * t) * decay, (speed - k * drift * t) * decay)
        }
    }

    private var slide: Slide?
    private var slideLink: CADisplayLink?

    @objc private func slid(_ link: CADisplayLink) {
        guard let slide else { return stopSliding() }
        let now = slide.at(link.targetTimestamp)
        let settled = abs(now.progress - slide.target) < 0.0005 && abs(now.speed) < 0.01
        progress = settled ? slide.target : now.progress
        place()
        guard settled else { return }
        stopSliding()
    }

    private func stopSliding() {
        slideLink?.invalidate()
        slideLink = nil
        slide = nil
    }

    @objc private func panned(_ recognizer: UIPanGestureRecognizer) {
        let moved = recognizer.translation(in: view).x
        switch recognizer.state {
        case .began:
            stopSliding()
            progressAtStart = progress
            view.endEditing(true)
        case .changed:
            let wanted = progressAtStart + moved / sidebarWidth
            // Past its ends the card gives a little and no more.
            let over = wanted > 1 ? wanted - 1 : wanted < 0 ? wanted : 0
            progress = min(1, max(0, wanted)) + over * 0.12
            place()
        case .ended, .cancelled:
            let speed = recognizer.velocity(in: view).x
            let opens = abs(speed) > 280 ? speed > 0 : progress > 0.5
            setOpen(opens, animated: true, velocity: speed)
        default:
            break
        }
    }

    // MARK: Whose swipe it is

    func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
        let speed = pan.velocity(in: view)
        guard abs(speed.x) > abs(speed.y) * 1.3 else { return false }
        let rightwards = speed.x > 0
        guard !open else { return !rightwards }
        return enabled && rightwards && !takesRightSwipe(under: pan.location(in: view))
    }

    /// Whether what the finger is on has a swipe to the right of its own: a view that still
    /// scrolls to the left of where it is, like the tabs of a split inbox, or one whose swipe
    /// is out, like a row showing its actions.
    private func takesRightSwipe(under point: CGPoint) -> Bool {
        var view = self.view.hitTest(point, with: nil)
        while let current = view, current !== self.view {
            if let owner = current as? OwnsRightSwipe, owner.ownsRightSwipe { return true }
            if let scroll = current as? UIScrollView, scroll.isScrollEnabled, scroll.contentSize.width > scroll.bounds.width + 1,
                scroll.contentOffset.x > -scroll.adjustedContentInset.left + 1
            {
                return true
            }
            view = current.superview
        }
        return false
    }

    /// The scroll views wait to hear that this swipe isn't the drawer's, which they do the
    /// moment it sets out up or down.
    func gestureRecognizer(_ recognizer: UIGestureRecognizer, shouldBeRequiredToFailBy other: UIGestureRecognizer) -> Bool {
        other is UIPanGestureRecognizer && other.view is UIScrollView
    }
}
#endif
