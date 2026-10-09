#if os(iOS)
import SwiftUI
import UIKit

/// Rows along `axis`, each drawn by `row`, that the list picks up and moves itself: held, a row
/// lifts on its light and follows the finger, the rows it passes slide out of its way, and let
/// go it slides into its place. Rows of the same `group` change places; the rows that `follow`
/// one go out of sight while it is lifted and come back under it. `moved` says where a row
/// landed: its index among the rows without it and its followers. A tap goes to `clicked`, a
/// hold opens `menu`. A vertical list takes the room it is given and scrolls; a horizontal one
/// is as wide as its rows, or as wide as it is given when it `fills`, with the rows sharing the
/// width.
struct MovableList<Item: Identifiable, Row: View>: UIViewRepresentable {
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

    func makeUIView(context: Context) -> UICollectionView {
        let coordinator = context.coordinator
        let view = UICollectionView(frame: .zero, collectionViewLayout: layout(coordinator))
        view.backgroundColor = .clear
        view.delegate = coordinator
        view.dragDelegate = coordinator
        view.dropDelegate = coordinator
        view.dragInteractionEnabled = true
        view.showsVerticalScrollIndicator = indicators
        view.showsHorizontalScrollIndicator = false
        view.isScrollEnabled = axis == .vertical
        if axis == .vertical { view.contentInset = UIEdgeInsets(top: inset, left: 0, bottom: inset, right: 0) }
        coordinator.attach(view)
        return view
    }

    private func layout(_ coordinator: Coordinator) -> UICollectionViewLayout {
        switch axis {
        case .vertical:
            var configuration = UICollectionLayoutListConfiguration(appearance: .plain)
            configuration.showsSeparators = false
            configuration.backgroundColor = .clear
            return UICollectionViewCompositionalLayout.list(using: configuration)
        case .horizontal:
            let (fills, inset) = (fills, inset)
            return UICollectionViewCompositionalLayout { [weak coordinator] _, _ in
                let count = max(coordinator?.shownCount ?? 1, 1)
                let width: NSCollectionLayoutDimension = fills ? .fractionalWidth(1 / CGFloat(count)) : .estimated(80)
                let item = NSCollectionLayoutItem(layoutSize: NSCollectionLayoutSize(widthDimension: width, heightDimension: .fractionalHeight(1)))
                let group = NSCollectionLayoutGroup.horizontal(
                    layoutSize: NSCollectionLayoutSize(widthDimension: .fractionalWidth(1), heightDimension: .fractionalHeight(1)), subitems: [item]
                )
                let section = NSCollectionLayoutSection(group: group)
                section.contentInsets = NSDirectionalEdgeInsets(top: 0, leading: inset, bottom: 0, trailing: inset)
                return section
            }
        }
    }

    func updateUIView(_ view: UICollectionView, context: Context) {
        let coordinator = context.coordinator
        coordinator.list = self
        coordinator.store = store
        coordinator.show(in: view)
    }

    /// A vertical list takes the room it is offered; a horizontal one is as tall as its tallest
    /// row, and as wide as its rows unless it fills.
    func sizeThatFits(_ proposal: ProposedViewSize, uiView: UICollectionView, context: Context) -> CGSize? {
        guard axis == .horizontal, let store = context.coordinator.store else { return proposal.replacingUnspecifiedDimensions() }
        let width = proposal.width ?? uiView.bounds.width
        let ids = items.map(\.id)
        if let measured = context.coordinator.measured, measured.ids == ids, measured.width == width { return measured.size }
        let each = fills ? (width - 2 * inset) / CGFloat(max(items.count, 1)) : CGFloat.greatestFiniteMagnitude
        let sizes = items.map { item in
            UIHostingController(rootView: row(item).environment(store)).sizeThatFits(in: CGSize(width: each, height: .greatestFiniteMagnitude))
        }
        let size = CGSize(width: fills ? width : sizes.map(\.width).reduce(0, +) + 2 * inset, height: sizes.map(\.height).max() ?? 0)
        context.coordinator.measured = (ids, width, size)
        return size
    }

    final class Coordinator: NSObject, UICollectionViewDelegate, UICollectionViewDragDelegate, UICollectionViewDropDelegate {
        var list: MovableList?
        var store: MailStore?
        private var source: UICollectionViewDiffableDataSource<Int, Item.ID>?
        /// The rows as the list last gave them.
        private var ids: [Item.ID] = []
        /// Counts the list's updates, so that a cell made ahead of time is redrawn if one came since.
        private var generation = 0
        /// The row being dragged, the rows out of sight while it is, and where it can land.
        private var dragged: Item.ID?
        private var vanished: [Item.ID] = []
        private var landings: [Int] = []
        /// The size a horizontal list was last measured at, for the rows and width it had.
        var measured: (ids: [Item.ID], width: CGFloat, size: CGSize)?

        var shownCount: Int { ids.count - vanished.count }

        func attach(_ view: UICollectionView) {
            let registration = UICollectionView.CellRegistration<MovableCell, Item.ID> { [weak self] cell, _, id in
                self?.configure(cell, id)
            }
            let source = UICollectionViewDiffableDataSource<Int, Item.ID>(collectionView: view) { view, indexPath, id in
                view.dequeueConfiguredReusableCell(using: registration, for: indexPath, item: id)
            }
            source.reorderingHandlers.canReorderItem = { [weak self] id in self?.movable(id) == true }
            source.reorderingHandlers.didReorder = { [weak self] transaction in self?.reordered(to: transaction.finalSnapshot.itemIdentifiers) }
            self.source = source
        }

        /// Applies the rows when they are others, and redraws the rows on screen.
        func show(in view: UICollectionView) {
            generation += 1
            let wanted = list?.items.map(\.id) ?? []
            if wanted != ids {
                ids = wanted
                apply(animated: false)
            }
            for indexPath in view.indexPathsForVisibleItems {
                guard let cell = view.cellForItem(at: indexPath) as? MovableCell, let id = source?.itemIdentifier(for: indexPath) else { continue }
                configure(cell, id)
            }
        }

        private func apply(animated: Bool) {
            var snapshot = NSDiffableDataSourceSnapshot<Int, Item.ID>()
            snapshot.appendSections([0])
            snapshot.appendItems(ids.filter { !vanished.contains($0) })
            source?.apply(snapshot, animatingDifferences: animated)
        }

        func collectionView(_ view: UICollectionView, willDisplay cell: UICollectionViewCell, forItemAt indexPath: IndexPath) {
            guard let cell = cell as? MovableCell, cell.generation != generation, let id = source?.itemIdentifier(for: indexPath) else { return }
            configure(cell, id)
        }

        /// Draws the row in the cell, dimmed while the cell is pressed as a button would be.
        private func configure(_ cell: MovableCell, _ id: Item.ID) {
            guard let list, let store, let item = item(id) else { return }
            cell.generation = generation
            let content = list.row(item).environment(store)
            cell.configurationUpdateHandler = { cell, state in
                cell.contentConfiguration = UIHostingConfiguration { content.opacity(state.isHighlighted ? 0.6 : 1) }
                    .margins(.all, 0)
                    .minSize(width: 0, height: 0)
            }
        }

        private func item(_ id: Item.ID) -> Item? {
            list?.items.first { $0.id == id }
        }

        private func movable(_ id: Item.ID) -> Bool {
            guard let list, let item = item(id) else { return false }
            return list.group(item) != nil
        }

        func collectionView(_ view: UICollectionView, didSelectItemAt indexPath: IndexPath) {
            view.deselectItem(at: indexPath, animated: false)
            guard let list, let id = source?.itemIdentifier(for: indexPath), let item = item(id) else { return }
            list.clicked(item)
        }

        // MARK: Moving a row

        func collectionView(_ view: UICollectionView, itemsForBeginning session: UIDragSession, at indexPath: IndexPath) -> [UIDragItem] {
            guard let list, let id = source?.itemIdentifier(for: indexPath), movable(id), let from = ids.firstIndex(of: id) else { return [] }
            let follows = list.items.map(list.follows)
            dragged = id
            vanished = Landings.lifted(from: from, follows: follows).dropFirst().map { ids[$0] }
            landings = Landings.of(from: from, groups: list.items.map(list.group), follows: follows)
            return [UIDragItem(itemProvider: NSItemProvider())]
        }

        func collectionView(_ view: UICollectionView, dragSessionIsRestrictedToDraggingApplication session: UIDragSession) -> Bool { true }

        func collectionView(_ view: UICollectionView, dragSessionWillBegin session: UIDragSession) {
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            guard !vanished.isEmpty else { return }
            apply(animated: true)
        }

        func collectionView(_ view: UICollectionView, dragSessionDidEnd session: UIDragSession) {
            dragged = nil
            landings = []
            guard !vanished.isEmpty else { return }
            vanished = []
            apply(animated: true)
        }

        func collectionView(_ view: UICollectionView, dragPreviewParametersForItemAt indexPath: IndexPath) -> UIDragPreviewParameters? {
            lifted(view, at: indexPath, UIDragPreviewParameters())
        }

        func collectionView(_ view: UICollectionView, dropPreviewParametersForItemAt indexPath: IndexPath) -> UIDragPreviewParameters? {
            lifted(view, at: indexPath, UIDragPreviewParameters())
        }

        func collectionView(_ view: UICollectionView, canHandle session: UIDropSession) -> Bool {
            session.localDragSession != nil && dragged != nil
        }

        /// The other rows part where the dragged one can land. Elsewhere it lands at the nearest
        /// place it can.
        func collectionView(
            _ view: UICollectionView, dropSessionDidUpdate session: UIDropSession, withDestinationIndexPath destination: IndexPath?
        ) -> UICollectionViewDropProposal {
            guard dragged != nil else { return UICollectionViewDropProposal(operation: .cancel) }
            guard let destination, landings.contains(destination.item) else {
                return UICollectionViewDropProposal(operation: .move, intent: .unspecified)
            }
            return UICollectionViewDropProposal(operation: .move, intent: .insertAtDestinationIndexPath)
        }

        func collectionView(_ view: UICollectionView, performDropWith coordinator: UICollectionViewDropCoordinator) {
            guard let drop = coordinator.items.first,
                  let index = lands(view, at: coordinator.destinationIndexPath, point: coordinator.session.location(in: view))
            else { return }
            coordinator.drop(drop.dragItem, toItemAt: IndexPath(item: index, section: 0))
        }

        /// Where a drop lands: the nearest place it can to where the finger is, or past the rows
        /// the end it is nearer.
        private func lands(_ view: UICollectionView, at destination: IndexPath?, point: CGPoint) -> Int? {
            guard let list, let first = landings.first, let last = landings.last else { return nil }
            if let destination {
                return landings.min { abs($0 - destination.item) < abs($1 - destination.item) }
            }
            let shown = shownCount
            guard shown > 0, let frame = view.layoutAttributesForItem(at: IndexPath(item: shown - 1, section: 0))?.frame else { return nil }
            let past = list.axis == .vertical ? point.y > frame.maxY : point.x > frame.maxX
            return past ? last : first
        }

        /// The rows are in their new order, without the ones out of sight: those go back under
        /// the dragged one, where it landed.
        private func reordered(to shown: [Item.ID]) {
            guard let list, let dragged, let index = shown.firstIndex(of: dragged), let item = item(dragged) else { return }
            let followers = vanished
            ids = shown.flatMap { $0 == dragged ? [$0] + followers : [$0] }
            list.moved(item, index)
        }

        /// The row lifts in the shape of its light.
        private func lifted<Parameters: UIPreviewParameters>(_ view: UICollectionView, at indexPath: IndexPath, _ parameters: Parameters) -> Parameters? {
            guard let list, let cell = view.cellForItem(at: indexPath) else { return nil }
            let inset = list.rowInset
            let rect = cell.bounds.inset(by: UIEdgeInsets(top: inset.top, left: inset.leading, bottom: inset.bottom, right: inset.trailing))
            parameters.visiblePath = UIBezierPath(roundedRect: rect, cornerRadius: list.rowRadius)
            parameters.backgroundColor = Tokens.accent.platform
            return parameters
        }

        // MARK: The row's menu

        func collectionView(
            _ view: UICollectionView, contextMenuConfigurationForItemsAt indexPaths: [IndexPath], point: CGPoint
        ) -> UIContextMenuConfiguration? {
            guard let list, let indexPath = indexPaths.first, let id = source?.itemIdentifier(for: indexPath), let item = item(id) else { return nil }
            let menu = list.menu(item)
            guard !menu.isEmpty else { return nil }
            return UIContextMenuConfiguration { _ in
                UIMenu(children: menu.map { item in
                    UIAction(title: item.title, attributes: item.destructive ? [.destructive] : []) { _ in item.perform() }
                })
            }
        }

        func collectionView(
            _ view: UICollectionView, contextMenuConfiguration configuration: UIContextMenuConfiguration, highlightPreviewForItemAt indexPath: IndexPath
        ) -> UITargetedPreview? {
            held(view, at: indexPath)
        }

        func collectionView(
            _ view: UICollectionView, contextMenuConfiguration configuration: UIContextMenuConfiguration, dismissalPreviewForItemAt indexPath: IndexPath
        ) -> UITargetedPreview? {
            held(view, at: indexPath)
        }

        private func held(_ view: UICollectionView, at indexPath: IndexPath) -> UITargetedPreview? {
            guard let cell = view.cellForItem(at: indexPath), let parameters = lifted(view, at: indexPath, UIPreviewParameters()) else { return nil }
            return UITargetedPreview(view: cell, parameters: parameters)
        }
    }
}

final class MovableCell: UICollectionViewCell {
    var generation = 0
}
#endif
