import SwiftUI

/// `MovableList` is a list that picks its rows up and moves them itself, on both systems: the
/// sidebar's mailboxes and accounts, the tabs over the list. Its rows are faces, not buttons: it
/// takes the clicks, tells each row when it is hovered, and offers `RowMenuItem`s on a secondary
/// click or a hold. Rows of the same group change places; a row that `follows` another, like an
/// account's mailbox under it, goes out of sight while that one is lifted and comes back under it.
/// `Mac/MovableListMac.swift` and `iOS/MovableListIOS.swift` are the two.
struct RowMenuItem: Identifiable {
    let title: String
    var destructive = false
    let perform: () -> Void

    var id: String { title }
}

/// Where a lifted row can be put down, worked out the same way by both lists.
enum Landings {
    /// The rows that go out of sight while the row at `from` is lifted: itself and the rows that
    /// follow it.
    static func lifted(from: Int, follows: [Bool]) -> Range<Int> {
        var end = from + 1
        while end < follows.count, follows[end] { end += 1 }
        return from..<end
    }

    /// Where the row at `from` can land, as positions among the rows left when it and its
    /// followers are lifted: before each other row of its group, and after the last of them and
    /// its followers. A row without a group, or alone in it, can only land where it was.
    static func of(from: Int, groups: [String?], follows: [Bool]) -> [Int] {
        let lifted = lifted(from: from, follows: follows)
        let kept = groups.indices.filter { !lifted.contains($0) }
        let origin = kept.filter { $0 < from }.count
        guard let group = groups[from] else { return [origin] }
        var landings: [Int] = []
        var end: Int?
        for (position, slot) in kept.enumerated() {
            if groups[slot] == group {
                landings.append(position)
                end = position + 1
            } else if end == position, follows[slot] {
                end = position + 1
            }
        }
        if let end { landings.append(end) }
        return landings.isEmpty ? [origin] : landings
    }
}
