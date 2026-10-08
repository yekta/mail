## What is this?

Wonnet is an email client that feels instant. Every action in the apps reads and writes a local
SQLite copy of the mail; the network only syncs in the background. A server of ours syncs with
the mail providers (Gmail, and JMAP servers such as Fastmail and Stalwart) and keeps the apps'
copies current over a WebSocket. Its look is the old Newton Mail's (CloudMagic, 2016–2020):
calm, plain, nothing that isn't needed.

These programs make it up, plus the code the apps share:

| Program | Where it runs | What it does |
| --- | --- | --- |
| Server (`apps/server`, the `mail-server` binary) | Our machines, beside Postgres | Signs people in, syncs their accounts with the providers, sends, snoozes, searches |
| Mac app (`apps/macos`) | The user's Mac | The interface |
| iOS app (`apps/ios`) | The user's iPhone and iPad | The interface |
| Apple kit (`packages/apple`, `MailUI`) | Inside the Mac and the iOS app | Their state and their views |
| Core (`crates/core`) | Inside every app | The local copy, the link to the server, rendering rows, pages and drafts |
| Marketing site (`apps/marketing`) | wonnet.app | The home page, privacy and terms |

The first mail account added makes the user; later accounts join that user with a one-time link
ticket. The server keeps each account's credentials sealed (AES-GCM with `SECRET_KEY`) and
session tokens hashed. README.md covers how to set things up.

Production is the `Wonnet` project on Unbind:

| Service | Address | What it runs |
| --- | --- | --- |
| `Server` | https://api.wonnet.app | The server, built from `Dockerfile` on every push to `main` that touches it |
| `Marketing` | https://wonnet.app | The marketing site, built from `apps/marketing/Dockerfile` on every push to `main` that touches it |
| `Postgres` | | The server's database |

The server runs one replica: an account's worker must run in one place. Gmail needs
`GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` set on the `Server` service. When a deploy changes
required variables, deploy the code first and change the variables after: Unbind restarts the
service on every variable change.

## Repo Structure:

### crates/protocol

What the server and the core agree on.

- `types.rs`: accounts, labels, messages, bodies and drafts as they are synced. A message's
  labels are roles (`inbox`, `sent`, `drafts`, `trash`, `spam`) and the ids of custom labels.
  Archived means none of inbox, trash or spam.
- `ops.rs`: the changes a user makes (`Op`) and what each does to a message. The server and the
  core apply the same function.
- `wire.rs`: every message on the sync socket. Bump `PROTOCOL_VERSION` when an old app or
  server could no longer understand the other.
- `api.rs`: the server's HTTP JSON. `tls.rs`: installs ring as the TLS provider, once.

### apps/server (Rust, axum, sqlx on Postgres)

- `sign_in.rs`, `google.rs`: adding a Gmail account, Google's OAuth with PKCE. The code the app
  is sent back with works once, and only with the secret that started it.
- `api.rs`: the code exchange, link tickets, JMAP accounts, the dev login, Gmail's push hook,
  uploads for outgoing attachments and the download of a message's attachments.
- `db.rs`: Postgres. Every synced row has a `rev` from one sequence and a `deleted` tombstone.
  Every write to synced rows runs in `UserTx`, which holds the user's advisory lock (so one
  user's revs commit in order and a cursor never skips one) and calls `pg_notify('changes')`.
- `changes.rs`: a batch of what changed after a cursor, read in one REPEATABLE READ snapshot.
- `hub.rs`: the LISTEN connection that wakes a user's sockets and an account's worker.
- `sync.rs`: the sync socket. `ops.rs`: applying a client's op, once, and queueing it for the
  provider. Preferences and drafts are synced rows too.
- `rules.rs`: what happens to mail new to the server: a muted thread's leaves the inbox, a blocked
  sender's goes to trash, a reply ends its thread's snooze. `unsubscribe.rs`: leaving a list.
- `search.rs`: Gmail's search operators turned into SQL.
- `workers.rs`: one task per account: send the waiting ops, sync, fetch the newest bodies, sleep.
  What the provider says is not written over a message whose op the provider doesn't have yet.
- `providers/`: `gmail.rs` and `jmap.rs`, both turned into `RemoteMessage`s. `mime.rs` reads raw
  MIME (bodies, attachments, headers) and builds what is sent.
- `scheduler.rs`: snoozed mail coming back and mail waiting to be sent (undo-send, send-later,
  remind me).
- `seed.rs`: `mail-server seed` gives a Stalwart server a demo user with a few hundred messages.
- `migrations/`: the schema. `e2e/`: the real router on a fresh database per test, with a fake
  Google; `e2e/stalwart.rs` runs Stalwart, the server and the core together.
- `DEV_LOGIN=1` gives a session without an account. It is for tests and local work only.

### crates/core

A static library for the apps (`ffi.rs`: JSON commands in through `mail_core_command`, JSON
events out through the callback given to `mail_core_start`) and a Rust library for tests.

- `core.rs`: one loop that owns all state. Anything that waits on the network runs in a task of
  its own and sends its result back to the loop.
- `store.rs`: the SQLite copy, with FTS5 search and a `threads` table kept from the messages,
  with each mailbox's counts and the inbox's splits. An op is written at once and kept in the
  outbox; the server's state of its messages waits in `bases`, so a change from the server is
  rebased under it, and a refused op is rolled back. It is a cache: a new schema rebuilds it.
- `search.rs`: Gmail's search operators over FTS5. `undo.rs`: the ops that take an action back.
- `link.rs`: the sync socket, reconnecting with backoff. `http.rs`: the server's HTTP API.
- `render/`: what the apps draw. `rows.rs` ("Alice, me (3)"), `dates.rs`, `html.rs` (sanitized
  pages, remote images blocked, designed mail on a paper card), `dark.rs` (the mail's own colours
  turned for the dark scheme, with the original a class away), `text.rs` (plain text with
  links and folded quotes), `drafts.rs` (reply, reply all, forward), `compose.rs` (what is sent:
  the signature, the quote, the HTML) and `times.rs` (snooze and send-later times, typed or
  chosen).
- `api.rs`: the JSON the apps and the core exchange.
- `demo.rs`: made-up mail. With `demo` in its config (the apps' `--demo`) the core shows it and
  never connects.
- `examples/drive.rs` drives the core from a terminal.

### packages/theme

`tokens.json` is the source of every colour, in shadcn's names, measured from Newton Mail.
`build.mjs` writes `tokens.css` (the core includes it in message pages; the server's error page
too) and the Apple kit's `Tokens.swift`. Never edit either by hand: change `tokens.json` and run
`node packages/theme/build.mjs`.

### packages/apple (Swift: SwiftUI, with AppKit and UIKit for the thread list)

`MailUI`, the Swift package both apps are made of. `Sources/MailUI/Shared` is what both use,
`Mac` and `iOS` what only one does, each file of those inside `#if os(…)`.

- `Shared/Core`: `CoreBridge.swift` calls the Rust core, `Models.swift` is what it answers,
  `MailStore.swift` is the state the views show (`+Actions.swift`: what can be done to threads),
  `Preferences.swift` the synced settings, `Notifier.swift` notifications and the badge.
- `Shared/Platform`: `Platform.swift` (what the two systems call differently; sizes are written
  as on the Mac and `Platform.scale` enlarges them on iOS) and `Symbol.swift` (the icons, from
  Lucide's font, `Fonts/lucide.ttf`; a new one is a case with its character from the same
  lucide-static version's `font/codepoints.json`).
- `Shared/Theme`: `Tokens.swift` (generated), `Theme.swift` (`Space` and the sizes) and
  `Typography.swift` (`TextStyle`, the text scale).
- `Shared/Views/UI`: the design system's components, listed in `packages/apple/DESIGN.md` with
  its rules; `Gallery.swift` shows them all (⌘K, "Component gallery").
  `Shared/Views/List/RowText.swift`: a thread row's text, for both lists.
- `Shared/Views/Thread/ThreadActions.swift`: every action on threads, one list for the keys, the
  toolbars, the menus and the palette. `Palette` (⌘K and the shortcuts sheet), `Pickers` (snooze
  and labels), `Person` (the contact pane).
- `Shared/Views/Thread`: the thread, and `MessageWebView.swift`, the pooled web views that draw
  bodies and report their height.
- `Shared/Views/Sidebar`, `Compose`, `Onboarding`, `Settings`: SwiftUI.
- `Mac/`: `MailMacApp.swift` (the window, the top bar), `Keyboard.swift` (the single keys),
  `ThreadListMac.swift` (an `NSTableView`), `AppUpdater.swift` (finds the latest GitHub release,
  downloads and installs it when asked) and `UpdateRow.swift` (the update at the foot of the
  sidebar: the offer, the download, the restart).
- `iOS/`: `MailIOSApp.swift` (the navigation stack) and `ThreadListIOS.swift` (a `UITableView`
  with the swipes).

### apps/marketing (Astro, Tailwind)

wonnet.app: `src/pages` has the home page, `privacy.astro` and `terms.astro`. Black and white in
DM Sans, following the system's appearance; its colours are in `src/styles/global.css`, apart
from the apps' tokens. The screenshots come from `docs/screenshots`; the favicons are drawn by
`scripts/icons.mjs` (`pnpm icons`). It is served as static files by static-web-server
(`server.toml`).

### apps/macos and apps/ios

- `apps/macos`: a SwiftPM executable and `scripts/build-app.sh`, which builds the core and the
  app into `build/Wonnet.app`. `Resources/AppIcon.icon` is the icon of both apps, made in Icon
  Composer: the W in black on a light tile, white on a dark one in dark mode.
- `apps/ios`: `project.yml` for XcodeGen; the Xcode project is generated, never committed.
  `scripts/build-core.sh` builds the core for the platform Xcode builds for.

## General Rules:

- Keep it simple. Do not overcomplicate things.
- The apps must never wait on the network. Every action reads and writes the local copy and the
  server catches up. Nothing slow runs on the main thread.
- Rendering logic belongs in `crates/core`, not in an app, so that every future app gets it.
- The Mac app and the iOS app do the same things. What one gets, the other gets in the same
  change, and what both do is written once, in `packages/apple/Sources/MailUI/Shared`.
- A button, a field or a toast in the apps comes from `Shared/Views/UI`. A view does not style
  a control by hand; what is missing is added to the component. The rules (sizes, spacing,
  hover, pending, sheets, errors) are in `packages/apple/DESIGN.md`.
- Colours come from `packages/theme/tokens.json`, never a hex value in a view. Text sizes come
  from `TextStyle`, distances from `Space`.
- The server holds people's mail. Credentials are sealed, tokens hashed, and a sign-in code
  works once and only with its secret. Anything that changes this needs a test in
  `apps/server/src/e2e`.
- Do not leave paragraphs of comments on top of the code. Prefer clear names; keep the comments
  that are needed concise. Comments move with the code.
- Use guard statement patterns in any code you write.
- Do not edit generated code: `Cargo.lock`, `packages/theme/tokens.css`, `Tokens.swift`. Never
  edit an applied migration in `apps/server/migrations`; add a new one.
- Do not write useless tests; tests cover input/output behaviour.
- Reinvent the wheel but do not reinvent the car. A simple problem gets no new library; a
  complex but common one probably has a modern library already.
- Do not insert yourself into our code, commits or PRs in any way.
- Never commit or push unless asked to.
- After you make code changes, run the checks below and fix what they raise.

## Development

Needs Rust stable, Docker (for Postgres and Stalwart), Node 24 and pnpm. The apps need a Mac with
Xcode; the iOS app also XcodeGen and `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`.

    docker compose up -d                          # Postgres on 5441, Stalwart on 8441
    cargo run -p mail-server -- seed              # demo@example.com with a few hundred messages
    cargo run -p mail-server                      # with the variables of .env.example exported
    cargo run -p mail-core --example drive        # the core, driven from a terminal
    cd apps/marketing && pnpm install && pnpm dev # the marketing site
    apps/macos/scripts/build-app.sh --open        # the Mac app
    open apps/macos/build/Wonnet.app --args --demo  # the Mac app with made-up mail, no server
    cd apps/ios && xcodegen generate && open Mail.xcodeproj

Checks (`cargo test` needs the compose services; it makes a throwaway database per test, and
the Stalwart test runs when `STALWART_URL` is set):

    export DATABASE_URL=postgres://mail:mail@localhost:5441/mail STALWART_URL=http://localhost:8441
    cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
    node packages/theme/build.mjs --check
    cd apps/marketing && pnpm check && pnpm build

The `macOS` and `iOS` workflows build the apps on pull requests that change them. On `main`,
`release.yml` signs and notarizes the Mac app and publishes it as a GitHub release, and uploads the
iOS app to TestFlight (`apps/ios/scripts/testflight.sh`). Its signing secrets and the
`APPLE_TEAM_ID` and `IOS_BUNDLE_ID` variables are set on the repository.

## Commit Messages

Commit messages start with the part of the system they touched, followed by a short imperative
sentence describing the change:

    server: Refuse a sign-in code that was started by another app
    core: Fold quoted text in plain text mail
    core | apple: Show when a send failed

The parts are the folders in `apps`, `crates` and `packages`: `server`, `marketing`, `macos`,
`ios`, `core`, `protocol`, `apple` and `theme`. Use `ci` for the workflows and `docs` for README.md
and AGENTS.md.

The title should be concise. The description explains the work in more detail (only if
required) while still being concise. Use simple language, do not try to sound smart.
