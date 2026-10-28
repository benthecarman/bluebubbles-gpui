# Known Flutter desktop bugs and how this client avoids them

This is a curated list of known bugs in the official BlueBubbles Flutter desktop client (2.1.x) that matter to a desktop user, especially on Linux and Ubuntu. Each bug is paired with the part of this client's design that avoids it. It was compiled on 2026-10-27 from the 130 raw entries in `docs/research/BUGS-RAW.md`. Those entries came from 184 open issues, 101 closed `Desktop` issues, 110 issues closed since 2025-06-01, and keyword searches.

"Status" is the issue's state upstream. A merged entry is open if any of its issues is still open. Crate names refer to `ARCHITECTURE.md`. Mobile-only bugs, web-only bugs and pure feature requests are left out (see the raw file's appendix).

## Connection

- **[#3256](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3256)** Stuck on "Connecting..." after sleep until "Fetch Latest URL" (closed, all desktop)
  After a suspend or a few hours offline, socket.io's retry loop gave up and nothing restarted it, so the app sat offline and showed no error.
  **Design:** bb-socket's `ClockWatch` sees the wall clock jump ahead of the monotonic clock and reconnects immediately. Missed server pings count as a dead link, and the jittered backoff never gives up. Only a refused password stops it.

- **[#2952](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2952)** Clients disconnect for good when the server URL changes (closed, all desktop)
  The client only retried on `error`, but a tunnel drop often emits just `disconnected`.
  **Design:** bb-socket handles every way a session can end (close, error, ping timeout) through one reconnect path with backoff. The client has no Firebase URL discovery, so if a quick tunnel gets a new URL, the user must enter it. A named tunnel or DDNS host avoids this.

- **[#3353](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3353), [#2958](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2958), [#2914](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2914), [#2661](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2661), [#2242](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2242), [#2517](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2517)** Google sign-in / Firebase failures break connectivity (open, all desktop)
  Dynamic-URL discovery depends on a Google OAuth session and the Firebase project list. On desktop, that session expires every few days or fails to load. Setup also crashed when the server had no Firebase config.
  **Design:** There is no Firebase and no Google sign-in. bb-api connects to the URL the user entered and nothing else.

- **[#2964](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2964)** Connection indicator flaps; REST outage not shown (closed, Windows)
  The indicator followed socket state only and had no debounce. A dead REST API with a live socket looked healthy.
  **Design:** bb-core's connection state machine combines the socket state with a `/server/info` REST health check. The UI shows a banner only after the app has been offline for a few seconds.

- **[#2887](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2887)** Custom headers (Cloudflare Access) not sent during setup (closed, all desktop)
  The first setup handshake went out without the headers, so it got a 403, and the headers were then cleared.
  **Design:** bb-api's `ServerConfig` headers become default headers on the HTTP client, so they go on the first request. bb-socket sends the same headers on the WebSocket upgrade. The ngrok and zrok skip headers are added automatically.

- **[#2668](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2668)** Server URLs with a path prefix get 404 (open, all desktop)
  The client built API URLs from the origin and dropped `/bb`.
  **Design:** bb-api keeps the path and appends `/api/v1/`. bb-socket builds `<prefix>/socket.io/`. A unit test covers this.

- **[#3384](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3384)** Connection page shows stale server version (open, all desktop)
  The page reads a cached copy that is never updated after a fetch.
  **Design:** Server info comes from the engine's health check and is stored like other state. Views re-read it when it changes and keep no copy of their own.

## Sync and missing or stale messages

- **[#3308](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3308), [#2760](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2760), [#2871](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2871), [#2565](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2565)** Read state, edits and unsends made while offline are lost (open, all desktop)
  Incremental sync only asks for `ROWID > last`. Changes to existing rows are lost unless they arrive live on the socket. That leaves stale unread dots, unsent attachments that still show, edited links with old previews, and broken timestamps after an unsend.
  **Design:** bb-core's catch-up also runs a reconcile query for messages whose delivered, read, edited or retracted dates changed since the cursor. bb-store merges them forward-only by GUID. Unsends are detected from `messageSummaryInfo.retractedParts`, not from `dateRetracted` alone. Chats deleted on Apple devices still depend on the server (see the last section).

- **[#2751](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2751), [#2915](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2915), [#2638](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2638), [#2958](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2958)** Chat list preview shows the previous message (open, Linux, Windows)
  The chat moves to the top with an unread dot, but the preview keeps the old text and time. This is risky for one-time codes. The tile's in-memory state is separate from the DB row.
  **Design:** There is one source of truth. Every input is a single bb-store transaction that returns `Changes`. The sidebar re-reads every chat those changes name, so it cannot hold a preview that differs from the store.

- **[#2620](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2620)** Open chat stops updating after clicking a notification (closed, Windows)
  A second controller for the same chat took the events, and the visible view was bound to an orphan.
  **Design:** There is no registry of per-chat controllers. Every view re-queries bb-store when a change names its chat, so any number of views of one chat stay current.

- **[#2680](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2680), [#2816](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2816)** Messages missing even after sync; "Reset" leaves stale data (closed, all desktop)
  The in-app reset did not clear everything, so users had to delete the app-data folder by hand.
  **Design:** Each server has its own SQLite file in bb-store, and reset deletes that file. Catch-up runs single-flight on every connect, reconnect, focus and timer. Nothing skips it, unlike Flutter's 30 s cooldown.

- **[#3012](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3012)** Inbound messages hidden for contacts with both phone and email (closed, all desktop)
  The client dropped messages whose handle did not resolve to a chat participant. Only outbound messages showed.
  **Design:** bb-store files each message under the chat GUID in its payload and never drops one because of an unknown handle. The sender label falls back to the raw address.

- **[#3008](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3008)** Unknown-sender chats vanish from the list (open, all desktop)
  The filter state and the list state drift apart until the filter is toggled.
  **Design:** The chat list is a bb-store query. Filters are parameters of that query, not a second list kept in sync by hand.

- **[#3231](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3231)** One delete mode blocks resync for good (open, all desktop)
  One of the two delete paths sets `dateDeleted`, and sync then skips that row forever.
  **Design:** bb-core has one delete command. Catch-up upserts never skip a row because of a local flag.

- **[#2776](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2776), [#2660](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2660), [#2667](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2667)** Messages out of order after switching between phone and desktop (open, all desktop)
  Sorting mixed local temp-message times with server times. High Sierra servers also report `dateCreated` after `dateDelivered`.
  **Design:** The transcript is ordered by the server's `dateCreated`. A sent message's temp row is renamed in place when the server echoes it and takes the server's date. The High Sierra date bug is in the server.

- **[#2269](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2269)** Search hit loads only 25 messages each way (open, all desktop)
  The user cannot scroll past the loaded context.
  **Design:** bb-store pages history by date with keyset paging, so a view anchored at any message can page in both directions.

## Duplicates

- **[#2702](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2702)** New chat sends the first message twice (open, all desktop)
  A second click on Send, made because the first seemed to fail, sent again. Nothing guarded the in-flight send.
  **Design:** The outbox row and its `temp-<uuid>` GUID are created on the first click and shown at once. A retry reuses that row's `tempGuid`, which the server refuses while it is in flight. On a timeout, bb-core checks the chat's recent messages from me before it marks a send failed.

- **[#2917](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2917), [#3016](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3016)** Received message plays two sounds (closed, Windows)
  An in-chat receive sound and the notification sound fired separately.
  **Design:** The only sound is the freedesktop notification, raised once per message because of the `notified` flag in bb-store. No second sound path exists.

## Notifications

- **[#3115](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3115), [#3114](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3114), [#3073](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3073)** Notification reply, click and "Mark read" do nothing (closed, Windows)
  Toast activations were not routed back to the running instance.
  **Design:** The app owns its `org.freedesktop.Notifications` D-Bus connection and receives `ActionInvoked` for Open and Mark as Read, plus inline reply on KDE. The actions call the same bb-core commands as the UI.

- **[#2314](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2314), [#1622](https://github.com/BlueBubblesApp/bluebubbles-app/issues/1622)** Clicking a notification starts a second instance or throws (closed, Linux, Windows)
  Activation launched a new process instead of signalling the running one.
  **Design:** D-Bus action signals reach the running process directly. Any second launch hands over to it through the Unix-socket single-instance lock.

- **[#3271](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3271)** Window freezes ("Not Responding") after hours; needs `kill -9` (closed, Linux)
  A media_kit player that had been disposed was reused to play a custom notification sound.
  **Design:** The app has no in-process media player. The notification daemon plays the sound.

- **[#2396](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2396)** No notification for the "active" chat while minimized (closed, Windows)
  Notifications were suppressed for the open chat even when the window was hidden.
  **Design:** A notification is suppressed only when the window has focus and is showing that chat.

- **[#2656](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2656)** Typed text invisible after opening a chat from a notification (open, Windows)
  The old view's `dispose()` deleted the new view's controller in the global GUID registry, so the text field was bound to an orphan.
  **Design:** There is no controller registry. The window has one composer, bound to the selected chat. Notification activation only changes the selection.

- **[#2757](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2757)** After a reset, old messages fire notifications and memory grows to 50 GB (closed, Windows)
  A resync replayed history as new messages. The memory growth came from server 1.9.6.
  **Design:** Notifications come from bb-store, not the socket. A message notifies once (`notified` flag) and only if it is newer than the last time the app was connected, so a first sync or a resync is silent.

## Performance

- **[#3333](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3333), [#3321](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3321)** Keystrokes freeze for 10-60 s (closed, Windows)
  Duplicate or orphaned per-chat controllers each did work on every keystroke. This is the same root cause as #2656.
  **Design:** The composer is gpui-kit's textarea. A keystroke changes only the editor's state, and no per-chat controllers exist to multiply the work.

- **[#3362](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3362)** UI freezes ~10 s every couple of minutes (full contact refresh with avatars) (closed, Linux)
  After every incremental sync, the app downloaded every contact with avatars and re-matched all handles on the UI isolate.
  **Design:** bb-core fetches contacts without avatars on a schedule. It fetches avatars only for contacts that changed, and does all of this on Tokio, off the UI thread.

- **[#2645](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2645), [#2632](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2632)** Slow with many chats (open, all desktop)
  The latest message was computed per chat with an unindexed ObjectBox query, and the UI thread made synchronous DB reads.
  **Design:** bb-store runs indexed SQLite queries on GPUI's background executor. The sidebar re-reads only the chats a change names, and the UI thread never waits on disk or network.

- **[#2748](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2748), [#2372](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2372), [#1806](https://github.com/BlueBubblesApp/bluebubbles-app/issues/1806), [#3272](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3272)** High idle CPU and GPU, even minimized (closed, Linux, Windows)
  Cursor blink, media and preview widgets kept driving frames, and one core sat at 100% on Arch with a chat open.
  **Design:** GPUI repaints a window only when a view is notified or an animation runs. Media are decoded still images, not live players.

## Scrolling and rendering

- **[#2497](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2497), [#2550](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2550), [#2296](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2296), [#2522](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2522), [#2559](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2559)** Wheel scrolling broken or jumpy; reversed middle-click; no PgUp/PgDn (open, Linux, all desktop)
  Custom scroll physics broke wheel scrolling on Ubuntu, and Flutter cannot smooth the wheel without adding input lag.
  **Design:** The transcript is GPUI's `list` and takes the platform's scroll deltas directly. No custom physics layer sits in between.

- **[#2264](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2264)** Flathub build freezes after screen lock or minimize (closed, Linux)
  Bugs in the `screen_retriever` and Wayland plugins stopped redraws, and one caused a segfault.
  **Design:** GPUI's own Wayland and X11 platform layer (`third_party/gpui-pre-linux`) drives the window. No Flutter window plugins are involved.

- **[#2699](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2699)** Opening an image freezes the X11 desktop; viewer won't close (open, Linux)
  **Design:** Images are decoded and downscaled on the background executor. Opening one never blocks the event loop.

- **[#3259](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3259)** Photo bursts render overlapped (open, Windows)
  The layout height was computed before the image sizes were known.
  **Design:** A bubble reserves space from the attachment's `width` and `height` in bb-store, or a fixed placeholder while they are 0. GPUI's list re-measures the row when the image arrives.

- **[#3195](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3195)** Server-supplied avatars render squished (open, Windows)
  **Design:** Avatars are decoded off the UI thread and cropped to a square that keeps their aspect ratio before they are drawn.

- **[#2968](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2968), [#2926](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2926), [#2438](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2438), [#2598](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2598)** Unusual message types break or mislabel the chat (open, all desktop)
  A location-share balloon threw during build and blanked the whole chat. Chatbot messages show as "Unsupported Interactive Message", and the wrong text appears when location sharing stops.
  **Design:** Every field in bb-api's models defaults leniently, and each row renders on its own. An unknown balloon falls back to its text or to a labelled placeholder bubble. Chatbot balloons render as plain text.

## Composer and keyboard

- **[#3280](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3280)** Num Lock breaks shortcuts and Enter-to-send (open, Linux)
  `LogicalKeySet` needs an exact match, and Num Lock counts as a pressed key.
  **Design:** Shortcuts match on Ctrl, Alt, Shift and Super only and ignore lock keys.

- **[#2860](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2860)** Numpad Enter inserts a newline (open, Linux)
  The code compared only `LogicalKeyboardKey.enter`.
  **Design:** The composer treats numpad Enter as Enter.

- **[#3089](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3089), [#3142](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3142), [#2203](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2203), [#2035](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2035)** Ctrl+V doesn't paste text; pasting images or files fails (closed, Linux, Windows)
  Each Ctrl+V sent an image clipboard request and a text request back to back, which starved the text read.
  **Design:** A paste makes one clipboard read and picks text, image or file list from the offered types. The vendored GPUI patch reads `text/uri-list` as file paths on Wayland and X11. Files can also be dropped onto the composer.

- **[#2419](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2419), [#2543](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2543), [#2524](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2524)** Composer loses focus after send or on mouse-out; cursor blinks while inactive (open, all desktop)
  Keyboard-scrolling support took focus away from the text field.
  **Design:** Focus stays in the composer after a send. Transcript scrolling keys are window-level bindings and do not take focus.

- **[#2801](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2801)** Send delay ignored when sending with Enter (open, all desktop)
  The Enter path skipped the button's logic.
  **Design:** Enter and the Send button issue the same bb-core outbox command.

## Attachments and media

- **[#3014](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3014), [#2625](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2625), [#2659](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2659)** libmpv crashes the app on startup; black or broken video (open, Linux)
  In-process libmpv thumbnailing hit SIGABRT on one MP4 at every launch, EGL fallback made video black, and HDR renders wrongly.
  **Design:** The app has no in-process video player or decoder. Videos open in an external player, so one bad file cannot crash the app.

- **[#2938](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2938)** Image shows "failed to display" until the chat is reopened (closed, Linux, Windows)
  The download finishing raced the widget resolving the file. HEIC and TIFF also need conversion.
  **Design:** The server sends no event when an attachment download finishes. bb-core retries the download with backoff, writes the file to the disk cache, and records the change in bb-store, so the bubble re-renders. HEIC is requested from the server as JPEG.

- **[#2386](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2386), [#1611](https://github.com/BlueBubblesApp/bluebubbles-app/issues/1611)** One stuck attachment blocks every send in every chat (open, all desktop)
  All chats shared one global serial send queue.
  **Design:** The outbox table has one queue per chat, so a stalled upload holds only its own chat. A 504 on a slow attachment is checked against recent messages before the send is marked failed.

- **[#3145](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3145), [#2789](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2789)** Saving an attachment writes nothing (closed, Linux, Windows)
  The save path was resolved inside the Flatpak or MSIX sandbox.
  **Design:** The app ships as a normal `.deb` and as a Nix build, with no filesystem virtualization. Save writes to the path the file chooser returns.

- **[#2458](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2458)** Special characters in attachment names break downloads (closed, Windows)
  **Design:** The disk cache stores files by attachment GUID, not by server filename. The name is sanitized only when the user saves.

- **[#2883](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2883)** Full-screen viewer shows a low-resolution image (open, Windows)
  The viewer probably showed the cached thumbnail.
  **Design:** The disk cache holds the full file. Downscaling happens at decode time for the target size, so the viewer decodes the original at its own size.

## Reactions

- **[#2829](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2829), [#3275](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3275)** Emoji tapbacks unsupported; reactions show as "null" (open, all desktop)
  The model allows only the six classic types, and the text mapping returns null for anything else.
  **Design:** bb-api keeps `associatedMessageType` as a string, including raw codes such as `"2006"`. The emoji comes from the reaction's text, and an unknown code falls back to a generic "Reacted" label. The server can only send the six classic tapbacks. The UI offers tapbacks only when `/server/info` reports the Private API.

## Contacts and names

- **[#3301](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3301), [#2509](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2509), [#2799](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2799)** Wrong or missing names after moving to a new server (closed, all desktop)
  Local handle rows were keyed to the old server's ROWIDs, and a reset did not rebuild them.
  **Design:** bb-store keeps one database file per server, keyed by URL and reported identity, so servers never share rows. Contacts link by normalized address, not by server ROWID.

- **[#3019](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3019)** 1:1 chats show raw numbers on Linux (closed, Linux)
  Handle sync never ran automatically, and its button was hidden in developer tools.
  **Design:** bb-core runs contact matching on a schedule and after each contact fetch. The user does not need to trigger it.

- **[#2461](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2461), [#2321](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2321), [#1621](https://github.com/BlueBubblesApp/bluebubbles-app/issues/1621)** Contacts don't match; chats land in "unknown senders" (closed, Linux, Windows)
  Numbers without `+1` did not match handles stored in E.164.
  **Design:** bb-core normalizes phone numbers on both sides before it compares them.

- **[#2136](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2136)** Family members all get the same photo (closed, all desktop)
  Matching on any shared number (for example, a landline) gave every family member one photo.
  **Design:** An address shared by several contacts counts as ambiguous, so it does not pick one contact's name and photo.

- **[#2610](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2610)** New conversation shows the number where the name should be (open, Windows)
  **Design:** The title, initials and photo come from one resolved contact row in bb-store.

## Read receipts and typing

- **[#3268](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3268)** Unread dot doesn't clear when read on another device (closed, all desktop)
  The handler wrote to the DB row, but the tile rendered from separate in-memory state.
  **Design:** `chat-read-status-changed` becomes a bb-store write, and the sidebar re-reads the chat. Local mark-read calls are debounced. Read state changed while offline comes back through catch-up (#3308 above).

- **[#3278](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3278)** Typing indicators get stuck (open, all desktop)
  The iPhone side keeps showing "typing" (server bug #866). Flutter also has no timeout for incoming typing, so a missed `display:false` leaves the bubble up.
  **Design:** bb-core clears an incoming typing bubble after 60 s without an update, or when a message from that sender arrives. bb-socket's `emit` fails while disconnected and does not queue, so a stale "typing" is never sent late.

## Settings and setup

- **[#3286](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3286)** Picking an emoji can wipe settings and the database (closed, Linux)
  A plugin rewrote the shared-preferences file without a lock. That dropped `finishedSetup`, and the missing key triggered a destructive DB reset.
  **Design:** Settings are rows in SQLite with a single writer, and every write is a transaction. A missing setting falls back to its default. Only an explicit user reset deletes data.

- **[#3167](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3167), [#3057](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3057), [#3058](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3058), [#3059](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3059), [#2963](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2963)** Settings not saved across restarts (closed, Windows)
  Atomic temp-file renames failed under MSIX virtualization. Settings, "Launch at startup" and notification volume were lost, and reconnecting after a reboot crashed.
  **Design:** Settings are written in SQLite transactions, so they are either saved or not, never half-written.

- **[#2629](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2629), [#2759](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2759), [#2879](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2879), [#2832](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2832)** Process runs but no window ever appears (closed, Windows)
  A corrupted `shared_preferences.json` stopped startup with no recovery.
  **Design:** There is no JSON preferences file. SQLite's journal keeps the settings tables consistent across crashes.

- **[#3054](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3054)** Dark theme not restored on cold start (Wayland) (closed, Linux)
  The saved mode was never read back, and the portal's "no preference" mapped to light.
  **Design:** Saved appearance is read from the settings table before the first window opens. "Follow system" applies only when the user picks it.

- **[#2942](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2942)** Google sign-in opens a blank WebKitGTK window or crashes (closed, Linux)
  WebKitGTK's DMABUF renderer failed ("Failed to create GBM buffer").
  **Design:** There is no WebView and no Google sign-in.

## Packaging and startup

- **[#3386](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3386)** Flatpak exits with status 1 whenever media plays (closed, Linux)
  libmpv built against one FFmpeg version loaded another and called `exit(1)`.
  **Design:** The app links neither libmpv nor FFmpeg.

- **[#3299](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3299)** Flatpak hangs at 100% CPU, no window (closed, Linux)
  The newer fontconfig in the runtime loops on a stale `~/.cache/fontconfig`.
  **Design:** The `.deb` uses the host's own fontconfig and its cache. bb-ui loads the bundled fonts from memory.

- **[#3383](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3383), [#3274](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3274)** Stuck on "Starting..." on Ubuntu / Mint (open, Linux)
  The cause is unknown. The maintainer suspects the GPU or the Flutter renderer.
  **Design:** This is a different stack (GPUI on Vulkan, with no Flutter engine or GTK embedder), and the `.deb` is built in an `ubuntu:26.04` container against Ubuntu's libraries. GPU driver problems can still happen, so this is avoided only as far as the cause is in Flutter.

- **[#2304](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2304), [#2213](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2213), [#2577](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2577), [#3270](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3270), [#3230](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3230), [#2856](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2856), [#2433](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2433), [#2747](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2747)** Library mismatches crash or block launch (closed, Linux)
  These include a missing GLIBC_2.34, Snap SEGVs, `libmpv.so.1` vs `.so.2` on aarch64, and a tarball that needs system webkit2gtk and libmpv.
  **Design:** The app has no webkit2gtk, libmpv or Flutter engine to mismatch. cargo-deb builds the `.deb` against the target release's libraries, and `nix build` wraps exactly the libraries GPUI loads.

- **[#2927](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2927)** One incoming message crash-loops the app at "Loading chats" (closed, Linux)
  An upstream `window_manager` plugin bug crashed the app on every restart.
  **Design:** There is no window plugin layer. A message bb-api can't fully parse still upserts with defaults and renders as a placeholder.

- **[#3288](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3288), [#2521](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2521)** Multiple instances run against one database (closed, Linux)
  The single-instance lock existed only in the Snap and Flatpak builds, not in Nix, AUR or tarball builds.
  **Design:** Every build takes the Unix-socket single-instance lock. A second launch hands over to the running one.

## Window and lifecycle

- **[#3247](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3247), [#3060](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3060), [#1803](https://github.com/BlueBubblesApp/bluebubbles-app/issues/1803), [#3246](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3246), [#2818](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2818)** App can't be quit from the tray or the window (open, Linux, Windows)
  A window close on a hidden GTK window never reached the `exit(0)` path, so the socket and sync kept running until SIGKILL.
  **Design:** Quit is an app-level exit, not a window close, and is always available in the app. Closing the window keeps the app running only if the user turned on "keep running in the background". A tray is optional and planned for later.

- **[#3258](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3258), [#1968](https://github.com/BlueBubblesApp/bluebubbles-app/issues/1968)** Relaunch or notification click doesn't raise the window (closed, Linux, Windows)
  The second launch was blocked but never asked for activation.
  **Design:** A second launch passes its xdg-activation token through the Unix socket. A notification click passes its activation token. The vendored GPUI patch uses that token, so GNOME and other Wayland compositors raise the window.

## Privacy and security

- **[#3289](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3289)** Error popups show the server password (open, all desktop)
  The password is a `?password=` query parameter, and error text showed the full URL.
  **Design:** The password lives in the Secret Service keyring. bb-api strips URLs from errors (`without_url`) and redacts the password in `Debug` output, and no log line or dialog shows a URL. The server only accepts the password as a query parameter, so it still travels in URLs and can appear in proxy logs.

- **[#2889](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2889)** Logs go to journald with phone numbers and message text (closed, Linux)
  Debug stdout was on in release builds, and Flatpak forwarded it to journald.
  **Design:** The password is redacted from every log line and error. The default log level records event kinds and GUIDs, not message bodies or addresses.

## Not addressed / out of scope

- **FaceTime and Find My** are not part of this client: [#3371](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3371), [#3304](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3304), [#2843](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2843), [#2593](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2593).
- **Windows only** (this client targets Linux): window effects and custom chrome [#2466](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2466), [#2560](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2560), [#2672](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2672), [#2740](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2740); emoji panel and IME [#3374](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3374), [#2624](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2624); GIFs and live photos [#2484](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2484), [#2489](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2489); Store crash [#3130](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3130); ARM [#2902](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2902); install sources [#3203](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3203), [#3129](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3129); CLI parsing [#2539](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2539); share button [#3254](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3254); start minimized [#2913](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2913).
- **Flutter engine or early-build bugs** with no counterpart here: [#2983](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2983), [#1544](https://github.com/BlueBubblesApp/bluebubbles-app/issues/1544), [#1536](https://github.com/BlueBubblesApp/bluebubbles-app/issues/1536).
- **Flatpak and Snap distribution**: no Flatpak or Snap package exists yet. This covers the autostart Exec path [#2716](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2716) and stale or EOL runtimes [#2814](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2814), [#2594](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2594), [#2281](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2281).
- **Tray**: not built yet. When it is, it should use a StatusNotifierItem with pixmaps, not file paths [#2840](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2840), [#2717](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2717).
- **Thread merging and services**: Messages.app keeps separate chats per address and service, and the server reports RCS as iMessage. Merged views are a later feature: [#3328](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3328), [#2756](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2756), [#2931](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2931), [#2855](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2855).
- **Features not designed yet**: fallback URLs [#347](https://github.com/BlueBubblesApp/bluebubbles-app/issues/347), mentions while editing [#2683](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2683), local contacts [#2705](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2705), voice notes [#3263](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3263), [#3264](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3264), [#2880](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2880), link previews [#3196](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3196), [#3251](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3251), app lock [#2647](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2647), jump-to-bottom control [#3373](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3373), window position (Wayland lets the compositor place windows) [#2649](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2649), [#2960](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2960), per-contact colours [#3170](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3170), locale date formats [#2746](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2746), RTL text [#3047](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3047).
- **Accessibility**: GPUI has no AT-SPI support yet, so screen readers and dictation tools can't use the app [#2965](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2965).
- **Too little detail, or Android-specific**: [#2923](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2923), [#2795](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2795), [#2420](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2420).

## Server-side issues the client works around

- **Duplicate `new-message` per send.** The server emits a slim echo with `tempGuid`, a full copy without it, and the HTTP reply, in any order. The first to arrive renames the temp row; the others are no-op upserts by GUID.
- **Re-emitted `new-message` for old messages.** The server's seen cache expires after an hour, so later changes to a message arrive as new messages. bb-store upserts and never appends blindly.
- **Outgoing rows before they are sent.** Unsent from-me rows can arrive early, and failures come later as `message-send-error`. Forward-only merges keep the final state.
- **No replay after reconnect.** The server buffers nothing. bb-core's catch-up and reconcile queries fill the gap, and notifications come from the store, so messages fetched in catch-up still notify.
- **No authoritative read or deletion snapshot** ([#3308](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3308); server PR #853). Read, edit and unsend changes are reconciled by date. Whole-chat deletions made on Apple devices still can't be detected.
- **No event when an attachment download finishes.** bb-core retries `/attachment/:guid/download` with backoff.
- **Error-shaped success.** A send can return HTTP 500 with a full message in `data`. bb-api returns that message, which is stored as sent but failed under its real GUID.
- **Slow attachment sends return 504, then succeed.** bb-core checks recent from-me messages before it marks a send failed.
- **`any;` chat GUIDs on macOS 26.** Outgoing events are not rewritten, so the client normalizes cached chat GUIDs.
- **Participants missing in message payloads** (a serializer index bug). Chats and participants are fetched with `/chat/query`, not taken from messages.
- **Unsends** come from `messageSummaryInfo.retractedParts`, not `dateRetracted` ([#2760](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2760)).
- **Old macOS dates.** High Sierra reports `dateCreated` after `dateDelivered` ([#2660](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2660), [#2776](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2776)). This can't be fully fixed on the client.
- **Password only as a query parameter** ([#3289](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3289)). The client hides it but can't stop it from going over the wire.
- **Stuck remote typing** ([#3278](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3278), server #866), **Google-sourced avatars and contact export** ([#2558](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2558), [#2369](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2369)), and **Firebase URL pushes** ([#2952](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2952)) are server bugs.
- **Messages database reset regenerates chat GUIDs** ([#2766](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2766)). Old chats stop updating and new duplicates appear. The client cannot link them.
- **Server 1.9.6 caused unbounded client memory** ([#2757](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2757)). Fixed in 1.9.7. 
