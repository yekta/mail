# Mail

An email client that feels instant, for the Mac and iOS. Its look is the old Newton Mail's: calm, plain, nothing that isn't needed. Open source, under the MIT license.

![The inbox on the Mac](docs/screenshots/mac-inbox.png)

| | |
| --- | --- |
| ![A thread](docs/screenshots/mac-thread.png) | ![The inbox in dark mode](docs/screenshots/mac-dark.png) |

- **Nothing waits on the network.** The apps keep a local copy of your mail in SQLite. Opening, archiving, starring and searching read and write that copy at once; a server syncs it with your mail provider in the background.
- **Gmail and JMAP.** Gmail through Google's API, and any JMAP server (Fastmail, Stalwart). Microsoft 365 and IMAP are planned.
- **Newton's habits.** A unified inbox with an account colour on every thread, snooze, undo send, send later, and single keys on the Mac: `j`/`k` to move, `e` archive, `s` star, `#` trash, `u` unread, `h` snooze, `r` reply, `a` reply all, `f` forward, `c` compose, `/` search.
- **Private by default.** Remote images stay blocked until you ask for them, and message bodies are sanitized before they are drawn.

These are its programs:

| Program | Where it runs | What it does |
| --- | --- | --- |
| Mail.app (`apps/macos`) | Your Mac | The interface |
| Mail for iOS (`apps/ios`) | Your iPhone and iPad | The interface |
| `mail-server` (`apps/server`) | A server, beside Postgres | Signs you in, syncs your accounts with their providers, sends, snoozes and searches |

Both apps are built on one Rust core (`crates/core`): the local copy, the sync, and everything they draw, so clients for other systems can share it.

## Getting started

You run the server yourself. You need Docker, Rust, Node 24, and for the apps a Mac with Xcode (and [XcodeGen](https://github.com/yonaskolb/XcodeGen) for iOS).

### 1. Start Postgres and the server

```sh
docker compose up -d                 # Postgres on localhost:5441, and Stalwart for trying JMAP
cp .env.example .env                 # then fill it in, see below
set -a; . ./.env; set +a
cargo run --release -p mail-server   # on http://localhost:3000
```

| Variable | What it is |
| --- | --- |
| `PUBLIC_URL` | Where the server can be reached, without a trailing slash. Google sends you back to `{PUBLIC_URL}/auth/google/callback` |
| `DATABASE_URL` | Postgres. The server runs its migrations when it starts |
| `SECRET_KEY` | Seals the mail credentials the server keeps. Any long random text (`openssl rand -hex 32`); changing it loses access to every account |
| `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` | The Google OAuth client, below. Without them Gmail can't be added |
| `GMAIL_PUBSUB_TOPIC`, `GMAIL_HOOK_TOKEN` | Optional: Gmail pushes changes to a Pub/Sub topic whose push subscription calls `{PUBLIC_URL}/hooks/gmail?token={GMAIL_HOOK_TOKEN}`. Without them Gmail is polled every 30 seconds |
| `INITIAL_SYNC_LIMIT` | How many messages an account's first sync fetches before older mail is filled in. 2000 by default |
| `DEV_LOGIN` | `1` gives sessions without an account, for tests. Never set it in production |

The `Dockerfile` builds the server for deploying.

### 2. Create the Google OAuth client

1. In the [Google Cloud console](https://console.cloud.google.com), create a project and enable the **Gmail API** (APIs & Services → Library).
2. Under **Google Auth Platform → Branding**, fill in the app's name and your support email.
3. Under **Audience**, choose **External** and leave the publishing status on **Testing**. Add every Google account that will use the app under **Test users**: up to 100.
4. Under **Data Access**, add the scopes `openid`, `email` and `https://www.googleapis.com/auth/gmail.modify`.
5. Under **Clients**, create a client of the type **Web application**, with the authorized redirect URI `{PUBLIC_URL}/auth/google/callback` (for example `http://localhost:3000/auth/google/callback`).
6. Put its client ID and secret in `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET`.

In Testing mode, Google ends a refresh token after **7 days**: the account then shows "Sign in again" and you remove and add it.

**Before a public launch**, the app has to leave Testing mode. `gmail.modify` is a restricted scope and the server stores the mail it fetches, so Google requires its verification and a yearly **CASA security assessment** (Cloud Application Security Assessment) by an authorized lab. Plan weeks for it, and its cost.

### 3. Build the apps

```sh
apps/macos/scripts/build-app.sh --open
```

The Mac app starts on `http://localhost:3000`; `MAIL_SERVER_URL=https://mail.example.com apps/macos/scripts/build-app.sh` builds one that starts on another server, and the server can be changed under **Advanced** on the first screen.

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cd apps/ios && xcodegen generate && open Mail.xcodeproj
```

Xcode builds the core for the iOS app itself. Set your team under Signing to run it on a device, and `MAIL_SERVER_URL` in `project.yml` for its server.

### 4. Add an account

Open the app and choose **Continue with Google**, or add a JMAP server with its address, your email and a password (for Fastmail, an app password and `https://api.fastmail.com`). The first account makes your user; accounts added later in Settings join it.

To try it without a real account, give the compose Stalwart a demo mailbox and add it as a JMAP account:

```sh
cargo run -p mail-server -- seed     # demo@example.com, password quiet-harbor-lantern-42
```

Server `http://localhost:8441`, email `demo@example.com`.

## Development

[AGENTS.md](AGENTS.md) describes the code, its rules and its checks. In short:

```sh
export DATABASE_URL=postgres://mail:mail@localhost:5441/mail STALWART_URL=http://localhost:8441
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
node packages/theme/build.mjs --check
cargo run -p mail-core --example drive   # drive the core from a terminal
```

## License

MIT
