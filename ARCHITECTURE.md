# Architecture

BlueBubbles GPUI is a native Linux client for the
[BlueBubbles Server](https://github.com/BlueBubblesApp/bluebubbles-server),
written in Rust with [GPUI](https://github.com/zed-industries/zed). It
replaces the Flutter desktop app. Its look and behaviour copy Apple's
Messages on macOS. `docs/BUGS.md` lists the Flutter app's desktop bugs and
which part of this design avoids each.

## Crates

```
crates/app      bluebubbles-gpui: GPUI views, notifications, single instance
crates/ui       bb-ui: the Messages palette, type, fonts and bubble shapes
crates/core     bb-core: the engine (sync, outbox, typing, contacts)
crates/store    bb-store: the SQLite cache, the only state views read
crates/api      bb-api: REST client and the server's JSON types
crates/socket   bb-socket: Socket.IO 4 client with reconnects
crates/mock     bb-mock: a fake BlueBubbles Server for tests and demos
```

Only `app` and `ui` depend on GPUI. Everything below them is plain async
Rust on Tokio and is tested without a display.

## Data flow

```
 socket events ─┐
 sync pages ────┼─► Engine (Tokio task) ──► Store::apply(batch)  ── one SQLite transaction
 send replies ──┘        ▲                        │
                         │ commands               ▼ Changes { chats, messages, ... }
                    GPUI views ◄──────────── broadcast; views re-query what changed
```

- **One source of truth.** Every input (socket event, sync page, HTTP
  reply to a send) becomes a batch that the engine applies to SQLite in
  one transaction. The transaction returns what changed. Views keep only
  rows they read from the store, and re-read the rows a change names. No
  view mutates its copy, and there is no registry of per-chat controllers.
  The Flutter app's stale previews, stuck unread dots and frozen chats came
  from two copies of state drifting apart.
- **Upserts by GUID with merge rules.** Dates only move forward
  (`dateDelivered`, `dateRead`, `dateEdited`, `dateRetracted`), so events
  can arrive in any order, twice, or through both the socket and sync
  without harm.
- **The UI thread never waits on the network or the disk.** Store reads
  for a view run on GPUI's background executor; images decode and
  downscale there too.

## Connection

`bb-socket` speaks Engine.IO 4 over WebSocket directly. It reconnects with
jittered backoff, treats missed server pings as a dead link, and
reconnects at once when the wall clock jumps ahead of the monotonic clock
(the laptop slept). A refused password stops retries until the user acts.

The engine's connection state machine combines the socket's state with a
REST health check (`/server/info`), and the UI shows a debounced banner
only when it has been offline for a few seconds.

The server only accepts the password as a query parameter (`guid` for
REST, `password` for the socket, which the server URL-decodes twice). The
client keeps it in the Secret Service (OS keyring), strips URLs from every
error, and never logs or shows a URL that contains it.

The server sends each of our sends back up to three times (the HTTP
reply, a slim `new-message` with `tempGuid`, a full `new-message`
without it) in any order, and re-sends old messages as `new-message`
when they change an hour later. The store's upserts absorb all of it.

## Sync

Sync is idempotent and runs single-flight: a trigger during a sync marks
it dirty and it runs once more afterwards. Nothing skips it.

- **First run** fetches the chat list with each chat's last message and
  participants, newest first. Conversation history is fetched page by page
  as the user scrolls (keyset paging on date), with the first page of the
  top chats prefetched.
- **Catch-up** runs on every connect and reconnect, on window focus
  (debounced), and every few minutes while connected. The server keeps no
  event log, so it is two `/message/query` requests: new rows
  (`message.ROWID > :cursor`, the server's row ids, immune to clock skew),
  and rows whose `date_delivered`, `date_read`, `date_edited` or
  `date_retracted` changed since the last catch-up (minus an overlap).
  Receipts, edits and unsends made while offline are not lost. Edit and
  retract dates exist only on macOS 13+, so the query adapts to
  `/server/info`'s `os_version`.
- **Notifications** come from the store, not from the socket: a message
  notifies once (a `notified` flag in its row), whether it arrived by
  socket or by catch-up, as long as it is newer than the last time the
  app was connected.
- Each server has its own database file, keyed by the server's URL and
  its reported identity. Pointing the app at another server never mixes
  their data, and "reset" deletes one file.

## Sending

The outbox is a table, so pending sends survive a restart.

1. The message is inserted at once with a `temp-<uuid>` GUID and shown.
2. A queue per chat sends it over REST with `tempGuid`; one stuck upload
   never blocks other chats.
3. The server's echo (socket `new-message`) and the HTTP reply both carry
   the real message. Whichever arrives first renames the temp row in
   place; the second is a no-op. Views keep their row identity through the
   rename, so the bubble doesn't flicker.
4. On a timeout, the engine checks the chat's recent messages from me
   before marking a send failed, so a retry doesn't send twice.

Tapbacks, edits, unsends, replies and effects need the server's Private
API helper. The engine reads its availability from `/server/info` and the
UI only offers what will work.

## UI

The window copies Messages: a sidebar of conversations (avatar, name,
preview, time, unread dot) and a transcript with blue (iMessage), green
(SMS) and gray bubbles. Consecutive messages from a sender within a minute
group together and only the last has a tail. Timestamps separate gaps over
30 minutes. "Delivered" and "Read" sit under the last message they apply
to. Tapbacks open from right-click or double-click. Replies show the
quoted message above. A typing bubble animates while someone types and
clears after 60 s without an update.

- The transcript is GPUI's `list` with bottom alignment and tail
  following. Older pages splice in above without moving the reading
  position.
- The composer is gpui-kit's textarea: Enter sends, Shift+Enter adds a
  line, numpad Enter is Enter, and shortcuts ignore Num Lock. Files can be
  pasted or dropped onto it.
- Images are fetched once into a disk cache and decoded off the UI
  thread. HEIC photos are requested from the server as JPEG.

## Desktop integration

- Notifications use `org.freedesktop.Notifications` over D-Bus: one
  notification per chat, replaced as messages arrive, with Open and Mark
  as Read actions, and inline reply where the server supports it (KDE).
  A click raises the window with the notification's activation token.
- A second launch hands over to the running instance through a Unix
  socket, with its activation token, so the window comes forward on
  Wayland.
- Closing the window keeps the app running only when the user turned on
  "keep running in the background"; Quit always exits.
- Settings live in SQLite and are written in transactions, so they cannot
  be half-written.

## Packaging

- `nix build` gives the app wrapped with the libraries GPUI loads at run
  time (Wayland, Vulkan, X11, fontconfig).
- `scripts/package-deb.sh` builds an Ubuntu `.deb` in an `ubuntu:26.04`
  container with cargo-deb, so the binary links Ubuntu's libraries.

## Vendored code

`third_party/gpui-pre-linux` is GPUI's Linux platform with three patches
(window raising with activation tokens, early Wayland window naming, and
pasting copied files); `third_party/README.md` lists them.
