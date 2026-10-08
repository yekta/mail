# The design system

What the Mac app and the iOS app are drawn with. Everything here lives in
`Sources/MailUI/Shared`: a view composes these and adds nothing of its own. What is missing is
added to the component, not to the view.

## Rules

- Colours come from `Tokens` (generated from `packages/theme/tokens.json`). Never a hex value,
  never a system colour.
- The page is `background`: the bars, the sidebar, the list. An open thread, a message being
  written and a form's card are `card` on it. A hairline on the page is `border`; inside a
  card it is `cardBorder`.
- Text comes from the scale, `TextStyle`: `Text("…").textStyle(.caption)`. A view picks a
  style, never a size. A style carries its usual colour; `color:` overrides it.
- Distances come from `Space`: `xs` 4, `s` 8, `m` 12, `l` 16, `xl` 20, `xxl` 28. Heights of
  controls come from `ControlSize` and `Theme`.
- Nothing clickable is under 28pt tall. 36pt is the default on the Mac; iOS enlarges everything
  by `Platform.scale`. `ControlSize` is `.small` (28), `.regular` (36) or `.large` (44).
- Everything clickable has a hover state, shown at once: no animation on colours. A button's
  face reads `@Environment(\.hovered)`, set by `PressStyle`, which every button uses.
- Hovered and selected things are filled from two pairs of tokens. Small things (buttons, tabs,
  chips, segments, the sidebar's rows) hover in `accent` and are selected in `accentStronger`.
  Large things (thread rows, choice rows, a banner) hover in `accentLarger` and are selected
  (or pressed, or highlighted) in `accentLargerStronger`: the same steps dimmer, because a fill
  across a whole row reads stronger than one behind a button. From dimmest to strongest they go
  `accentLarger`, `accentLargerStronger`, `accent`, `accentStronger`, so a small thing's hover
  shows on a selected row. An accent is never thinned with an opacity: if a step is missing, add
  a token for it. A control with a colour of its own (the star) hovers in a wash of it: the
  colour at `colorTintOpacity`, lighter in the dark scheme.
- Things side by side have no gap between them: an `HStack(spacing: 0)` of buttons, rows with
  no spacing. The space is inside each control, so hover areas and hit areas touch. Buttons
  with a face (a circle, a pill) show `Theme.buttonGap` (2pt) between the faces; each button
  pads half of it inside its hit area, and its hit shape is the whole rectangle. The gap is
  only drawn, never dead: a view never adds spacing between buttons.
- Every button can carry an icon and has a `pending` state; while pending, the spinner stands
  where the icon goes and the button waits.
- Everything is shown as soon as it is known; what is being fetched again is shown quietly
  (a chip's spinner, a mini spinner in a row), never by replacing what is there.
- Errors are always shown, with what went wrong: a `Notice` in error tone where the user is
  looking, or a toast with the message.
- The platform's controls are used when they fit, not by default: the switch (`ToggleRow`),
  alerts, menus, iOS's navigation bar for sheets' titles and buttons, Quick Look, the file
  pickers. Segmented controls, dropdowns, fields and buttons are ours.
- Sheets go through `Sheet`: on iOS it gets the system's title and Close in the navigation
  bar, on the Mac our `HeaderBar` and Esc. `SheetSize` picks the window on the Mac and the
  detents on iOS.

## Components

| Component | What it is |
| --- | --- |
| `ActionButton`, `ActionMenu` | Words, an optional icon, four variants (`primary`, `outline`, `ghost`, `destructive`), three sizes, `pending`, `wide`; the menu opens one |
| `IconButton`, `IconMenu`, `IconCircle` | An icon in a thin circle; `active`, `quiet`, `circled`, `pending` |
| `PlainButton` | Only its content, still with hover, press and disabled states, and the minimum height; `tint` colours its hover |
| `StarButton` | A thread's star, in the star colour when starred or hovered |
| `InputField` | A text field with its label above, or a text box of `lines` |
| `SearchField` | The search icon, the words, Clear; `.sheet` on a hairline with list keys, `.bar` as the top bar's capsule |
| `Dropdown` | A choice from a short list, with a label |
| `Segmented` | A few choices on a line |
| `ToggleRow` | A switch with its words |
| `Disclosure` | A line that opens more |
| `Chip` | A small pill: an address, a file, a label; `remove`, `action`, `pending`, `invalid` |
| `ItemRow` | A thing in a list with its controls at the end |
| `NavRow` | A row that goes somewhere, with a count and a selected state |
| `ChoiceRow`, `ChoiceList` | One choice under a field, and the list that scrolls to the highlighted one |
| `PopupCard` | A card over the page for suggestions |
| `TabStrip` | Tabs on a line with counts |
| `PillTabs` | Tabs on a line with an icon and a count, the chosen one filled in a rounded rectangle; dragged to reorder, with a menu on each |
| `BottomTabs` | Tabs across the bottom of the screen, full width, each an icon over its name and count (iOS) |
| `SectionHeading`, `FormSection` | A section's name in capitals, with its controls |
| `Card`, `FormButtons` | A rounded card, and Save and Cancel under a form |
| `Notice` | A line about something, muted or in error tone, with an optional action |
| `EmptyState` | An icon and a calm line for an empty list |
| `Banner`, `ToastView` | A line across the window that stays, and the note at the bottom that goes |
| `Avatar` | Initials in a circle of the address's colour |
| `SwatchRow` | The account colours on a line, to pick one |
| `Rule`, `.rule(edge)` | A hairline, in `border` or in `cardBorder` |
| `Sheet`, `HeaderBar` | Every sheet's shell, and the bar across the top of a page on the Mac; with a `center`, the sides split what it leaves |
| `FlowLayout` | Wrapping layout for chips |

The gallery shows all of them in every state: open the palette (⌘K) and pick "Component
gallery".
