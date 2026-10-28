# Plan

Each phase ends with tests passing, a commit series, and a summary of
what is done and what is needed from the user.

## Phase 0: Research and foundations (done)

- Study the server API, the Flutter client and its issue tracker
  (`docs/research`, `docs/BUGS.md`, `ARCHITECTURE.md`).
- Workspace, Nix flake, vendored GPUI Linux patches, fonts and icon.
- `bb-socket`: Socket.IO 4 client with reconnects and suspend detection.
- `bb-api`: REST client and lenient JSON types.
- `bb-store`: the SQLite cache with GUID upserts and send reconciliation.
- `bb-ui`: the Messages palette and bubble shape.

## Phase 1: Engine and mock server (done)

- `bb-mock`: a fake BlueBubbles Server (REST and Socket.IO) that tests
  drive: incoming messages, typing, receipts, dropped connections, send
  failures. Also runs standalone with demo data for screenshots.
- `bb-core`: the engine. Connection state, first sync, catch-up and
  reconcile, socket event handling, outbox with per-chat queues, typing
  timeouts, read-state debouncing, contacts and name resolution
  (E.164), notification decisions.
- Integration tests: the engine against the mock server.

## Phase 2: The app, text conversations

- Login screen (server address, password, test connection), password in
  the keyring.
- Sidebar: conversations with avatars, names, previews, times, unread
  dots, search.
- Transcript: bubbles with grouping, tails, timestamps, group events,
  Delivered/Read, typing bubble, failed sends with retry, older history
  on scroll.
- Composer: Enter to send, Shift+Enter for a newline, drafts, typing
  indicators.
- Screenshots against the mock server, light and dark.

## Phase 3: Rich messages

- Attachments: images inline (HEIC as JPEG), files, audio and video as
  file cards that open externally; paste and drag-and-drop to send.
- Tapbacks: picker on right-click or double-click, badges on bubbles,
  emoji tapbacks shown.
- Replies: quoted bubble, reply from the context menu.
- Edits and unsends shown; edit and unsend own messages when the
  server's Private API allows.
- Link previews from the server's rich-link payloads.

## Phase 4: Desktop integration

- Notifications over D-Bus: per-chat merging, Open and Mark as Read,
  inline reply where supported, activation tokens.
- Single instance with handover; keep running in the background (opt-in).
- Settings: appearance, text size, notifications, connection, about.
- Keyboard shortcuts.

## Phase 5: Packaging

- `nix build` of the wrapped app, `.desktop` file and icons.
- `scripts/package-deb.sh`: Ubuntu `.deb` built in a container.
- README with setup and troubleshooting.
