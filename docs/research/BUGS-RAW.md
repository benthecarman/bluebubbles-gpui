# BlueBubbles Flutter client: desktop-relevant known bugs (raw)

Collected 2026-10-27 from `BlueBubblesApp/bluebubbles-app` with read-only `gh`.

Sources:

- All 184 open issues.
- All 101 closed issues labelled `Desktop`.
- All 110 closed issues created since 2025-06-01.
- `--state all` keyword searches for linux, ubuntu, wayland, x11, flatpak,
  snap, fedora, arch, appimage, windows, desktop, sync, socket, duplicate,
  notification, lag, attachment, contact, scroll, reaction, and typing.

Root-cause notes marked **(code)** were checked against the vendored source at
`third_party/bluebubbles-app` (pubspec `2.1.2+92`, HEAD `7e5f1bc`). The other
root-cause notes come from the issue thread.

Context: the "full app rewrite" (#2972) shipped as 2.0.0 in mid-2026. The
maintainers closed many old issues with "should be fixed in the rewrite", so
some closed items may still be present. Closed items are listed because they
show where the design is fragile. Each entry gives its status.

Excluded:

- Android-only and iOS-only issues (FCM, battery, Samsung, flip phones, Android
  Auto, intents, and similar).
- Web-only issues (#2934, #2709, #2590, #2541, #2695, #2145, #1866).
- Pure feature requests. Desktop-relevant ones are in the appendix.

Entry format: **#N** title, then a line with status · date created · labels ·
platform, then the symptom, then the likely root cause.

---

## 1. Connection / socket / server URL

**#3256** [Have to manually Fetch Latest URL if notebook is asleep/offline for a few hours](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3256)
closed (fixed, PR) · 2026-09-04 · Desktop, PR Opened · Windows (affects all desktop)
- Symptom: After sleep or a long offline period, the app looks normal but is stuck in "Connecting...". The user has to open settings and press "Fetch Latest URL". The app shows no visible error.
- Root cause: The app had no watchdog and did not rediscover the URL after a resume or a backoff timeout. The current `socket_service.dart` **(code)** adds a heartbeat, a debounced connectivity-triggered `restartSocket()`, and a `_urlDiscoveryTimer` that calls `fetchNewUrl()` on errors. It also notes that socket.io's `reconnect_failed` permanently stops its retry loop (`_socketGaveUp`).

**#2952** [Server URL changing - clients disconnect](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2952)
closed · 2025-10-22 · — · all (Windows confirmed)
- Symptom: With a Cloudflare quick tunnel, the URL changes about once a day. Clients disconnect and never get the new URL.
- Root cause: A commenter found that `SocketService.handleStatusUpdate` retried only on `SocketState.error`. On `disconnected` it only logged, and on Windows a tunnel drop often emits only a disconnect event. Fixed for Windows. A server-side part also exists: the new URL was not always pushed to Firebase.

**#3353** [Cannot sign-in with Google](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3353)
open · 2026-10-14 · Needs More Info · all desktop + web
- Symptom: Google sign-in succeeds, then the app shows "We couldn't load your Firebase projects". "Fetch URL" returns `null`, so the client disconnects each time the server reboots and gets a new URL.
- Root cause: Dynamic-URL discovery depends on a Google OAuth session plus the Firebase project list. Any failure there (scopes, a different account, a network block) leaves the client with a stale URL. See also #2661.

**#2958** [Basic usability/stability issues](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2958) and **#2914** [Desktop app not staying logged into Google Firebase](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2914)
closed · 2025-12 / 2025-03 · — · Linux (Arch) / Windows
- Symptom: The desktop app loses its Google sign-in every few days. It then loses the server URL and connectivity without any visible alert. Each startup can require a new 2FA login. #2958 also reports a stale chat-list preview and general slowness.
- Root cause: Desktop OAuth tokens were not persisted or refreshed. Because URL discovery depends on that session, this is a single point of failure.

**#2964** [Connection indicator improvements - also possible API issue](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2964)
closed · 2025-12-15 · — · Windows
- Symptom: On a flaky network, the indicator flashes red, yellow, and green all the time. On a localhost connection, the REST API connection can drop while the socket stays connected, and the UI does not show it.
- Root cause: The indicator tracks only socket state, has no debounce, and does not check REST health.

**#2887** [Desktop app not using Custom Headers during initial setup](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2887)
closed · 2025-01-25 · — · all desktop
- Symptom: The custom headers needed for Cloudflare Access are not sent during the setup connect. Cloudflare returns 403, and the headers are cleared after each failed attempt. The workaround is a long-press on "Connect".
- Root cause: The setup flow did not apply the headers to the first HTTP and socket handshake.

**#3289** [Error popups show sensitive info](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3289)
open · 2026-09-29 · Priority: Normal · all
- Symptom: Error dialogs show the request URL, including the server password.
- Root cause: The server password is sent as a `?password=` query parameter on every REST call (also visible in #3012's curl example), and error text contains the full URL. Related to #2889, where logs went to journald.

**#2668** [Support Server URL paths (don't strip URL paths)](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2668)
open · 2024-01-29 · Effort: Medium · all
- Symptom: A server URL such as `https://host/bb` fails. Requests go to `https://host/api/v1/...` and return 404.
- Root cause: The client builds API and socket URLs from the origin only and drops the path.

**#3058** [[BETA] connection issue when restarting PC](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3058)
closed · 2026-06-25 · — · Windows
- Symptom: After a reboot the client is disconnected. "Fetch latest URL" does nothing, and Google sign-in or manual entry crashes the app.
- Root cause: The settings persistence failures in #3057 and #3059.

**#2661** [Handle Firebase connection issues better](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2661)
open · 2024-01-26 · Effort: Medium · all
- Symptom: When `firebase.googleapis.com` is unreachable, the app says "no Firebase projects found" and shows no network error.

**#3384** [Cached server info isn't updated after loading fresh details](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3384)
open · 2026-10-27 · Priority: Low · all
- Symptom: The connection settings page shows the old server version each time it opens, until the reload completes.
- Root cause: The page reads a cached copy that is never written back after a fetch.

**#347** [Fallback URLs](https://github.com/BlueBubblesApp/bluebubbles-app/issues/347)
open · 2020-10-29 · Priority: Low · all
- Symptom: The client cannot use both a LAN URL and a tunnel URL. A "localhost detection" toggle exists, but it does not fall back on errors or timeouts.

**#2242 / #2517** [Windows client won't connect without Firebase](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2242), [getClientConfig 404 crashes setup](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2517)
closed · 2022/2023 · — · Windows
- Symptom: Setup failed when the server had no Firebase configuration, because the client did not handle a 404 from `getClientConfig`.

## 2. Sync / missing or stale messages

**#3012** [Desktop client filters inbound messages from contacts with phone+email both registered to iMessage](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3012)
closed (fixed, PR) · 2026-04-25 · PR Opened, Priority: High · Windows (desktop)
- Symptom: Only the user's outbound messages render in the thread. Inbound messages are missing, although `/api/v1/chat/<guid>/message` returns them. The web client shows them. The problem survives a resync, a DB reset, and a reinstall. "Loading surrounding message context..." hangs and never sends a request.
- Root cause: The client matches handles badly for contacts with more than one handle (phone and email). The client drops messages whose handle does not resolve to the chat's participant. Related to #2756.

**#2751** [New message received - last message still showing in message list](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2751) (also #2915, #2638, #2958)
open · 2024-06-17 · UX · Windows, Linux
- Symptom: A notification arrives and the chat moves to the top with an unread dot, but the preview text and time still show the previous message. This is dangerous for one-time passcodes. #2915 and #2638 are closed as "fixed in rewrite", but #2751 is still open.
- Root cause: The tile's latest-message state is separate from the DB row and is not refreshed on every new-message path. This is the same dual-source-of-truth pattern as #3268.

**#2680** [New messages are received but not shown in reading pane](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2680) and **#2816** [Messages missing even after sync](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2816)
closed · 2024 · — · Windows / all desktop
- Symptom: Messages appear in the chat list but not in the thread, or they are missing even after a manual sync of the last 25.
- Root cause (per maintainer): An in-app "Reset" left stale data behind. The fix is to delete `%localappdata%\Packages\...\BlueBubbles` or the app-data directory by hand.

**#2620** [Clicking on desktop notification causes active chat window to stop updating](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2620)
closed · 2023-12-16 · Desktop · Windows
- Symptom: After the window is opened from a notification, the open chat stops showing new incoming or sent messages until the user leaves and comes back. The chat list also stops updating.
- Root cause: Two conversation views or controllers for the same chat (see #2656). The visible view is bound to a controller that no longer receives events.

**#3008** [Unknown number chats disappear from chat list](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3008)
open · 2026-04-25 · Needs More Info · all (rewrite regression)
- Symptom: All unknown-sender chats sometimes disappear from the main list, with no "unknown senders" entry point. Notifications for them stop. Toggling "filter unknown numbers" on and off restores them.
- Root cause: The filter state and the list state drift apart in the chat-list filter logic.

**#3231** [Two different modes for deleting messages; one breaks resyncing](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3231)
open · 2026-08-16 · Priority: Normal · all
- Symptom: Deleting from the popover can be undone by a resync. Deleting from the multi-select header sets `dateDeleted`, and nothing can undo it.
- Root cause: The app has two delete code paths, and sync skips rows that have `dateDeleted`.

**#3308** [Sync Apple read state and whole-chat deletions](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3308)
open · 2026-10-05 · — · reported for Android, design applies to all
- Symptom: The client misses read-state changes and chat deletions made on Apple devices while it was offline. Stale chat tiles and unread dots remain.
- Root cause **(code)**: For server 1.6.0 and later, `incremental_sync_manager.dart` queries `ROWID > lastSyncedRowId`, so it fetches only new rows. It never re-fetches updated rows, so it misses read-state changes, edits, unsends, and deletions. Those arrive only as live socket events. This needs a server snapshot contract (server PR #853).

**#2760** [Handle unsent messages properly (attachments)](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2760) and **#2871** [Edited links don't change embed](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2871)
open · 2024 · Requires Investigation · all
- Symptom: Unsent attachments still show. An edited link keeps the old preview and the old target.
- Root cause: The server emitted unsend events from the wrong fields (`messageSummaryInfo.retractedParts`). The client does not re-derive previews or attachment state on edits. The ROWID-only sync above also misses these when the client was offline.

**#2776** [Enhance handoff between desktop/mobile](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2776) (follows #2660 and #2667)
open · 2024-08-07 · Requires Investigation · all
- Symptom: Messages appear out of order after the user sends from mobile and then returns to the desktop.
- Root cause: The client sorts by `dateCreated`. Some servers (High Sierra) report `dateCreated` later than `dateDelivered`, and the local clock or temp-message timestamps differ from the server's. #2660 changed the sort to the earliest of the two, which reduced the problem but did not remove it.

**#2565** [Unsent message messes with timestamp of message](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2565)
open · 2023-09-27 · Priority: Normal · all
- Symptom: After a message is unsent, the timestamp separator and the content overlap or show wrong values.

**#2269** [Load entire message search context](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2269)
open · 2022-09-22 · Priority: Normal · all
- Symptom: Opening a search hit loads only 25 messages on each side, and the user cannot scroll further.

**#2766** [High CPU/lag "fix" thread](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2766)
closed · 2024-07-09 · — · all
- Note: If the user resets the macOS Messages database, the server makes new chat GUIDs. Desktop clients then keep the old chats, which stop updating, next to new duplicate chats. The client has no way to detect a GUID change.

## 3. Duplicate messages / duplicate chats

**#2702** [Starting a new chat with a new contact sends the opening message multiple times](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2702)
open · 2024-03-16 · UX, Priority: Normal · all
- Symptom: In the new-chat creator, a second click on Send (because the first seemed to fail) sends the message again.
- Root cause: No in-flight guard or idempotency key exists while the chat is created and the message is sent.

**#3328** [RCS group chats split into two conversations (`SMS;+;` and `RCS;+;`)](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3328)
closed (duplicate) · 2026-10-12 · — · all
- Symptom: On macOS 15 and later, one group appears as two threads, one per service. Messages split between them depending on fallback.
- Root cause: The client keys chats by GUID (`service;style;identifier`). Messages.app stores one row per service with the same `chatIdentifier`. Related to #2756 and #2931.

**#2756** [Show chats to same contact but different address/service as one thread](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2756)
open · 2024-06-22 · Effort: High · all
- Symptom: A contact who uses both email and phone appears as two conversations. iMessage on Mac merges them.

**#2917** [Received message notification sound issues](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2917) and **#3016** (Android twin)
closed · 2025-04 / 2026-05 · Desktop · Windows
- Symptom: A received message plays two sounds (the in-chat "receive sound" plus the notification sound), or the same sound twice.
- Root cause: Two independent sound paths with no shared rule for focus and visibility.

## 4. Notifications

**#3371** [[Windows] FaceTime call notification never dismisses when answered on another device](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3371)
open · 2026-10-21 · — · Windows + Arch Linux (comment)
- Symptom: When a call is answered or ended on the iPhone, the desktop notification stays and keeps ringing until the user dismisses it.
- Root cause **(code)**: `handleFaceTimeStatusChange` in `action_handler.dart` clears the notification only when `status_id == 6` and a `uuid` is present. Other terminal states are ignored. The legacy `incoming-facetime` path creates the notification keyed by caller name (`callUuid ?? caller`), so a uuid-based clear cannot find it.

**#3271** [Random lockups (Not Responding) on CachyOS/KDE using flatpak](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3271)
closed · 2026-09-10 · Desktop · Linux (CachyOS, Fedora 44)
- Symptom: The app runs for hours, then the window turns grey and "Not Responding", and the user must `kill -9` it.
- Root cause: The log shows `Assertion failed: "[Player] has been disposed"` from `NotificationsService.playDesktopNotificationSound`, a media_kit player. A shared player was disposed and then reused for a custom notification sound.

**#3115 / #3114 / #3073** [Replying from notification dismisses without sending](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3115), [clicking notification doesn't navigate](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3114), ["mark read" doesn't mark read](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3073)
closed (fixed, PR) · 2026-07 · Desktop, Priority: High · Windows (2.0 beta)
- Symptom: Toast inline reply and toast actions did nothing.
- Root cause: The toast activation payload was not routed back into the running instance or isolate in the rewrite.

**#2656** [Unable to type in Windows app after clicking on notification](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2656)
open (fix committed) · 2024-01-23 · Desktop, PR Opened · Windows
- Symptom: After a click on a notification, typed text is invisible until the user goes back to the chat list and opens the chat again.
- Root cause: Fix commit `9fb7adce` ("Fix #2656"), which maintainers also call the main fix for the 2.1.3 input lag (#3333). Per-chat `ConversationViewController`s live in the GetX global registry keyed by chat GUID. When a new view opened for the same chat, the old view's `dispose()` deleted the new controller, which left the text field bound to an orphaned controller. The fix makes the controllers permanent and deletes only "if current".

**#2396** [No Windows notification for active chat when minimized to tray](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2396)
closed · 2023-02-23 · — · Windows
- Root cause: "Chat is active" suppressed notifications without checking window visibility or focus.

**#2314 / #1622** [Linux exception when selecting a notification](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2314), [multiple instances start when clicking a notification](https://github.com/BlueBubblesApp/bluebubbles-app/issues/1622)
closed · 2022 / 2021 · — · Linux / Windows
- Root cause: Notification activation launched a second process instead of signalling the running one (single-instance IPC).

**#2509 / #2799** [Notifications display wrong contact name / scrambled names](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2509)
closed · 2023 / 2024 · — · Windows + Android
- Root cause: The user switched servers without resetting the client. Local handle and contact IDs then pointed at the wrong people (see #3301).

**#2923** [Text-detection notification whitelist broken](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2923)
closed · 2025-05-01 · Desktop · Windows

**#2795** [Notification timestamps don't update](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2795)
open · 2024-09-05 · Requires Investigation · likely Android, possibly grouped toasts elsewhere
- Symptom: When one notification is cleared, the remaining notifications show the newest message's time.

## 5. Performance / slowness

**#3333 / #3321** [v2.1.3.0 shows more lag](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3333), [Input lag - Windows 2.1.2.0](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3321)
closed (fixed) · 2026-10-12 · Desktop, Priority: High · Windows
- Symptom: Keystrokes freeze for 10 to 60 seconds, and then the input appears all at once.
- Root cause: The maintainer points to `9fb7adce`, the controller-lifecycle fix in #2656. Duplicate or orphaned per-chat controllers each did work on every keystroke.

**#3362** [Unconditional full contact refresh (with avatars) after every incremental sync freezes UI ~10 s](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3362)
closed (not planned) · 2026-10-15 · — · Linux (Flatpak 2.1.3)
- Symptom: About every 2 minutes, the UI freezes for 6 to 11 seconds (2,277 contacts). The log shows "Affected 0 handles".
- Root cause **(code)**: `sync_service.dart` (around line 192) calls `performContactSyncToHandles()` and `performContactSyncToServer()` after every incremental sync, whatever the result. Each call does a `fetchAll(withAvatars: true)`, base64-decodes and writes every avatar, and re-matches all handles on the UI isolate. A `checkContactChanges()` short-circuit exists but is not called. The maintainer says this is not what caused the lag, but the cost is real.

**#2983** [Noticeable lag on Desktop v1.15.5](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2983)
closed · 2026-04-03 · — · Windows
- Symptom: About 30 seconds after launch, the UI gets slower and slower until typing lags. Downgrading to 1.15.4 fixes it.
- Root cause: The maintainer suspected a Flutter 3.27-to-newer regression.

**#2645** [Client slows down with many chats](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2645) and **#2632** [Add index to Message model](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2632)
closed / open · 2024-01 · Optimization · Windows / all
- Symptom: With 1,465 chats and 148k messages, the chat list takes several seconds to move an unread chat to the top.
- Root cause: Profiling found that about 50% of CPU time was in `Chat.getMessages`, and that `Chat.latestMessage` is an expensive ObjectBox query. `dateCreated` has no index, and the latest message is computed per chat and not stored on the chat row.

**#2748 / #2372 / #1806** [High idle CPU (9-12%) when minimized](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2748), [100% CPU core on Arch with chat open](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2372)
closed · 2021–2024 · Desktop · Windows, Linux
- Symptom: CPU stays high while the app is idle or minimized, after viewing chats with media or links. On Arch Linux, one core runs at 100% while a chat is open and drops when Settings opens.
- Root cause: The text field and cursor animation, plus media and preview widgets, keep driving frames. Flutter keeps rendering while the window is hidden. #3272 (GPU about 3% when focused, from the cursor blink) is the same effect, which the maintainers call "expected".

**#2757** [Windows client using 50 GB of memory](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2757)
closed · 2024-06-24 · — · Windows
- Symptom: After a reset, memory use grew without limit and old messages fired notifications. Updating the server fixed it, but the client had no upper bound on memory.

## 6. Scrolling / rendering / window drawing

**#2264** [Flathub app freezes after screen lock / minimize](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2264)
closed · 2022-09-18 · — · Linux (GNOME, Fedora)
- Symptom: After a screen lock or minimize, the window stops redrawing and accepting input. A Wayland segfault also occurred.
- Root cause: The `screen_retriever` and Wayland plugin bugs in that version.

**#2699** ["Done" button doesn't close viewed images; image open freezes desktop](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2699)
open · 2024-03-13 · Desktop, Requires Investigation · Linux (KDE/X11)
- Symptom: The image viewer often does not close. In 1.14, opening an image froze the whole X11 desktop for about 30 seconds, and the reset flow froze for good.

**#3373** [Scroll to bottom](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3373)
open · 2026-10-21 · UX, Desktop · all desktop
- Symptom: The jump-to-bottom arrow disappears before the user can click it. Esc used to scroll to the bottom but now leaves the chat.

**#2559** [Smooth scrolling](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2559)
open · 2023-09-20 · UX, Web App, Desktop · all desktop
- Symptom: Mouse-wheel scrolling jumps in fixed steps with no smoothing. Touchpad scrolling is fine.
- Root cause: A Flutter limitation. The maintainers found that every package added input latency.

**#2497 / #2550 / #2296 / #2522** [Improved scrolling breaks scrolling](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2497), [middle-click scroll reversed](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2550), [missing scrollbars, PgUp/PgDn not working in Settings](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2522)
closed · 2022–2023 · Desktop · Linux (Ubuntu)
- Symptom: Custom scroll physics broke wheel scrolling on Ubuntu. Middle-click autoscroll moved the wrong way. Settings pages had no scrollbars or keyboard scrolling.

**#3259** [Burst of photos: vertical spacing off](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3259)
open · 2026-09-06 · Desktop · Windows
- Symptom: A burst of photos sent from iPhone or Mac renders shifted up and overlaps.
- Root cause: The layout height is computed before the image sizes are known (no size placeholder).

**#3195** [Most contact avatars are vertically squished](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3195) (regression of #2408)
open · 2026-08-10 · Desktop, Requires Investigation · Windows
- Symptom: Avatars that come from the server render squished. Avatars set locally are fine. The files on disk are 240x240 JPEGs.
- Root cause: An image fit or aspect-ratio bug in the avatar widget for server-supplied bytes.

**#2968** ["An unexpected error occurred when rendering" for location-sharing contacts](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2968)
closed · 2025-12-30 · — · Windows
- Symptom: Every chat with a contact who shares location with the user fails to render.
- Root cause: An unhandled message or item type (location-share balloon) throws in the build. One bad message breaks the whole chat view.

**#2740** [Window box shadow blinks in time with the cursor](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2740)
open · 2024-05-03 · Unconfirmed, Desktop · desktop (custom title bar)

**#2420** [Replying to very long messages creates huge padding](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2420)
closed · 2023 · — · all

**#2466 / #2560 / #2672** [White line at top of window](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2466), [dark mode transition fails with window effects](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2560), [Mica effect shows extra titlebar](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2672)
closed · 2023–2024 · Desktop · Windows
- Root cause: The custom window chrome and acrylic/Mica effects plugins.

## 7. Text input / composer / keyboard

**#3280** [[Linux] Num Lock breaks keyboard shortcuts and Enter-to-send](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3280)
open · 2026-09-20 · Desktop, Requires Investigation · Linux (Hyprland, native Wayland)
- Symptom: With Num Lock on, Ctrl+N, Ctrl+comma, and Enter-to-send all fail. GTK itself receives normal key events.
- Root cause **(code, partly inferred)**: The global shortcuts in `main.dart` (lines 356–369) use `LogicalKeySet`, which needs an exact match of the pressed-key set. Flutter's GTK embedder tracks Num Lock as a pressed or locked logical key, so the set does not match. The reason Enter also fails is not confirmed. A likely cause is the same modifier-state leak, which the composer's handler or the IME path does not tolerate.

**#2860** [Numpad Enter does not send message](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2860)
open · 2024-11-20 · Requires Investigation · Linux (likely all desktop)
- Symptom: Numpad Enter inserts a newline, like Shift+Enter.
- Root cause **(code)**: `keyboard_shortcut_handler.dart` (lines 37 and 89) and `message_edit_field.dart` compare only `LogicalKeyboardKey.enter`. They never check `numpadEnter`.

**#3374** [Deleted text comes back after inserting from the Windows emoji panel (Win+.)](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3374)
open · 2026-10-21 · Desktop · Windows
- Symptom: The user types `:test:`, presses Ctrl+A and Delete, then picks an emoji with Win+. The field shows `:test:😭`.
- Root cause: The IME composing region is stale. The `:shortcode:` autocomplete state and the TSF composition are out of sync with the cleared `TextEditingValue`.

**#2624** [Delete/Backspace key randomly stops working](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2624)
open · 2023-12-19 · Desktop, Needs More Info · Windows
- Symptom: Backspace and Delete stop working while typing still works. A commenter triggered it by pasting from a voice-dictation app (Wispr Flow). A restart sometimes fixes it.
- Root cause: A key-event or IME state desync. Flutter's `HardwareKeyboard` can be left with a stuck pressed-key state.

**#2965** [Form fields not keyboard-accessible with NVDA](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2965)
open · 2025-12-20 · UX, Desktop · Windows
- Symptom: Setup fields are not in the tab order and cannot accept typing through NVDA. Dictation tools cannot find the composer.
- Root cause: The Flutter accessibility and TSF integration of custom text fields.

**#3089** [[Linux] Ctrl+V doesn't paste text](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3089)
closed (fixed in 2.0.0) · 2026-07-16 · — · Linux
- Symptom: Ctrl+V did nothing for text. Right-click Paste worked.
- Root cause: Each Ctrl+V issued an image clipboard read (`gtk_clipboard_request_image`) and also let Flutter's text paste run. The two back-to-back GTK clipboard requests starved the text read.

**#3142 / #2203 / #2035** [Right-click paste of image missing](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3142), [pasting attachments doesn't work on Linux](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2203)
closed · 2022–2026 · Desktop · Windows / Linux

**#2419** [Enter-to-send doesn't return focus to text field](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2419) and **#2543 / #2524** [Composer loses focus on mouse-out](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2543), [cursor blinks when window inactive](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2524)
open / closed · 2023 · UX, Desktop · all desktop
- Root cause of #2543: The keyboard-scrolling support stole focus.

**#2801** [Send delay ignored when sending with Enter](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2801)
open · 2024-09-11 · Priority: Low · all
- Root cause: The Enter key path calls send directly and skips the button's delay and undo logic.

**#3047** [RTL text mis-ordered in bubbles and input](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3047)
open (PR) · 2026-06-07 · UX · all
- Root cause: Widgets use the ambient LTR direction and do not detect paragraph direction from the content.

**#2683** [Display mentions correctly when editing a message](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2683)
open · 2024-02-23 · Effort: Medium · all

**#1544 / #1536** [Ctrl+Backspace inserts a null char](https://github.com/BlueBubblesApp/bluebubbles-app/issues/1544), [stuck caps / no backspace](https://github.com/BlueBubblesApp/bluebubbles-app/issues/1536)
closed · 2021 · Desktop · desktop (early builds)

## 8. Attachments / media

**#3014** [Desktop app crashes on startup loading video thumbnail](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3014)
closed · 2026-04-28 · — · Linux (Flatpak)
- Symptom: Opening a chat or starting the app crashes with SIGABRT in `libmpv` (through `libmedia_kit_video_plugin.so`). Only one QuickTime/MP4 with Apple `mebx` metadata streams causes it, but the crash repeats on every launch.
- Root cause: The app thumbnails in-process with libmpv and has no isolation. One bad file can stop the app from starting.

**#2625** [Black screen on all videos (Flatpak)](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2625)
closed · 2023-12-21 · Desktop · Linux
- Root cause: media_kit fell back to S/W rendering because "EGL display or context is invalid", the same message seen in #3014.

**#3145** [Images and video not downloading on desktop](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3145) and **#2789** [Saving attachments on Linux doesn't work](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2789)
closed (fixed) · 2026-07 / 2024-08 · Desktop, Priority: High · Windows MSIX / Linux (SUSE)
- Symptom: The save dialog completes, but no file is written.
- Root cause: The path is resolved inside the sandbox or virtualized MSIX filesystem, so writes go to the wrong place.

**#2458** [Special characters in attachment name cause download failure](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2458)
closed · 2023 · — · Windows
- Root cause: Server filenames such as `cm-chat-media-video-1:...:0.MOV` contain `:`, which Windows does not allow. The client did not sanitize the name.

**#2938** [Images fail to display the first time](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2938)
closed ("fixed in rewrite") · 2025-06-19 · — · Linux (Fedora Flatpak) + Windows
- Symptom: After a download, the app shows "failed to display image" until the user leaves the chat and comes back.
- Root cause: A race between the download finishing and the image widget resolving the file. HEIC and TIFF need conversion on desktop.

**#2883** [Low image resolution in Windows viewer](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2883)
open · 2025-01-22 · Needs More Info · Windows
- Symptom: The full-screen viewer shows a much smaller image than the original.
- Root cause: The viewer probably shows the cached preview or thumbnail and not the full file.

**#2386** [Attachment send stuck "Waiting for iMessage"; then all sends fail until restart](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2386) and **#1611** [Messages in other chats wait on an attachment send](https://github.com/BlueBubblesApp/bluebubbles-app/issues/1611)
closed / open · 2023 / 2021 · Optimization · desktop / all
- Root cause: All chats share one global, serial outgoing queue. A stalled attachment upload (15 seconds or more, or forever) blocks every later send in every chat.

**#3263 / #3264 / #2880** [Voice note fails: unsupported 320 kbps AAC](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3263), [recording error leaves UI stuck in recording state](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3264), [no way to send voice message on Windows](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2880)
closed · 2025–2026 · Desktop · Windows (likely Linux too)
- Root cause: The code hardcodes `RecordConfig(bitRate: 320000)`. `audioRecorder.start()` is not awaited or caught, and the UI flag was set before the recorder started.

**#3254** [Share button does nothing for attachments on desktop](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3254)
closed · 2026-08-31 · Desktop · Windows

**#3196** [Some links enter redirect loops and show null errors](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3196) and **#3251** [Favicon keeps shrinking in preview](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3251)
open / closed · 2026-08 · Requires Investigation · desktop
- Symptom: Link-preview fetching does not handle redirect hops, and the bubble shows a null-pointer error instead of the bare link.

**#2659** [HDR videos don't render properly](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2659)
open · 2024-01-25 · Priority: Low · all

**#2484 / #2489** [GIFs from Win+. emoji panel sent as stills](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2484), [live photo download error](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2489)
closed · 2023 · Desktop · Windows

## 9. Reactions / tapbacks

**#2829** [Support emoji tapbacks](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2829)
open · 2024-10-23 · Priority: High, Effort: High · all
- Symptom: iOS 18 emoji and sticker tapbacks are not supported. They render wrongly or not at all.
- Root cause: The data model allows only the six classic tapback types.

**#3275** [Reactions show as "null" in notification and preview](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3275)
open · 2026-09-15 · UX, Priority: High · labelled Android, but the preview text is shared code
- Symptom: Heart and Like reactions show as "null" in notifications and in the chat-list preview.
- Root cause: The reaction-to-text mapping returns null for the associated-message type or emoji. The likely cause is the same model gap as #2829.

**#3073** (see section 4): notification actions for reactions and mark-read were partly broken in 2.0 beta.

**#2855** [Enable tapbacks for SMS/RCS](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2855)
open · 2024-11-16 · — · all (feature gap)

## 10. Contacts / names / avatars

**#3301** [Wrong contacts appearing in chat / existing contacts not referenced](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3301)
closed · 2026-10-01 · Desktop · desktop
- Symptom: After a move to a new server, chats show the wrong names or bare numbers, which skews statistics. After "Reset App", the app shows no names at all, even after "Refresh Contacts" ("Matched 193, Updated 0").
- Root cause: Local handle and contact rows keyed to the old server's ROWIDs. Contact matching does not rebuild the handle-to-contact links for existing chats.

**#3019** [Linux: "Sync Handles & Contacts" unreachable; 1:1 chat names never resolve](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3019)
closed · 2026-05-07 · — · Linux (snap 1.15.7)
- Symptom: 1:1 chats show raw `+1XXXXXXXXXX`. Group chats are fine because participant data comes with the chat.
- Root cause: The handle sync never ran automatically, and its button was buried in Developer Tools.

**#2461 / #2321 / #1621** [Desktop contacts not syncing](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2461)
closed · 2021–2023 · — · Windows, Linux
- Symptom: Some or all contacts do not match, and chats land in "unknown senders".
- Root cause (#1621): Phone-number normalization. Contacts stored without `+1` did not match handles stored in E.164.

**#2136** [Multiple contacts get the same contact photo](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2136)
closed · 2022 · — · desktop
- Root cause: Family contacts share a landline number. Matching on "any shared number" gave every family member one photo.

**#2558** [Only a handful of contact avatars appear](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2558)
closed (not planned) · 2023 · — · Windows
- Root cause: The server's contacts plugin cannot fetch avatars for Google-sourced contacts. Numbers that contain spaces also failed. This is mostly a server-side problem.

**#2610** [New conversation shows the phone number where the name should be](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2610)
open · 2023-12-04 · UX · Windows
- Symptom: The initials and photo are correct, but the name line shows the number.

**#2705** [Can't add or manage contact names on the desktop client](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2705)
open · 2024-03-19 · Desktop · desktop
- Symptom: The desktop client depends only on server iCloud contacts or an Android export. It has no local contact entry.

**#3170** [Avatar bubble color reset by group chat](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3170)
open · 2026-08-07 · UX · reported on Android, logic is shared
- Symptom: A custom per-contact color is overwritten by the group's random color after a restart.

## 11. Read receipts / unread / typing

**#3268** [Unread dot does not clear when a chat is read on another device](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3268)
closed (fixed) · 2026-09-09 · — · all (Windows Store 2.1.1 confirmed)
- Symptom: The unread dot stays until the user opens the chat or restarts the app.
- Root cause **(code, now fixed)**: The `chat-read-status-changed` handler wrote only to the ObjectBox row (`toggleHasUnreadAsync`). The chat-list tile renders from a separate in-memory `ChatState`, so it never updated. The handler now goes through `ChatsSvc.setChatHasUnread`.

**#3308** (see section 2): read state set on Apple devices while the client was offline is never reconciled.

**#3278** [Typing indicators do not go away (on the iPhone side)](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3278)
open · 2026-09-18 · Android · all clients that send typing (private API)
- Symptom: The remote side keeps seeing "typing" after the user leaves the app.
- Root cause: The server issue bluebubbles-server#866. The client also does not send a stop-typing event on blur, close, or exit.

## 12. Settings / setup / persistence

**#3286** [[Linux] Picking an emoji can wipe settings](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3286)
closed (fixed, PR) · 2026-09-24 · Desktop, PR Opened · Linux (likely Windows too)
- Symptom: After an emoji is picked, the next launch logs "Clearing database because setup is not finished...". The app then **wipes ObjectBox** and returns to setup.
- Root cause: `emoji_picker_flutter` saves recent emojis through the legacy `SharedPreferences` API. On Linux that API keeps a snapshot of the file per isolate and rewrites the whole file from it, without a lock and not atomically. This overwrote keys written by the custom `DesktopSharedPreferencesStore`. Losing `finishedSetup` then triggered a destructive DB reset.

**#3167** [Windows 2.0.x preferences fail to save (MSIX rename error)](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3167)
closed (fixed, PR) · 2026-08-05 · Desktop · Windows (MSIX)
- Symptom: The app does not save settings and disconnects after a few minutes.
- Root cause: An atomic write (temp file plus rename) failed with "cannot move the file to a different disk drive". MSIX virtualizes `%APPDATA%\Roaming` into `...\Packages\...\LocalCache\Roaming`, so the temp file and the target resolved to different locations.

**#3057 / #3059** [Settings not saved across restart](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3057), ["Launch at startup" not working](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3059)
closed · 2026-06-25 · Desktop · Windows (2.0 beta)

**#2963** [Notification volume not saved across restarts](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2963) (also #3174 on Android, still open)
closed · 2025-12-10 · — · Windows

**#2629 / #2759 / #2879 / #2832** [Process starts but no UI ever appears](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2629)
closed · 2023–2025 · Desktop · Windows
- Symptom: The process runs in Task Manager with no window, taskbar entry, or tray icon. The problem survives a reinstall.
- Root cause: `shared_preferences.json` was "nulled" (corrupted), and startup did not recover. The fix is to delete `%APPDATA%\BlueBubbles` by hand.

**#3054** [Persisted dark theme not restored on cold start (Linux/Wayland)](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3054)
closed · 2026-06-21 · — · Linux (wlroots)
- Root cause: `AdaptiveTheme(initial: system)` never read the saved mode back. The freedesktop `color-scheme` portal reports "no preference", which maps to light.

**#2942** [Google sign-in opens blank window on Linux](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2942)
closed · 2025-07-07 · — · Linux (Arch GNOME Flatpak), also Windows
- Symptom: The sign-in window is blank, or the whole app crashes, and later the window misbehaves.
- Root cause: The embedded WebKitGTK reports "Failed to create GBM buffer". The workaround is `WEBKIT_DISABLE_DMABUF_RENDERER=1`.

**#2647** [Secure App: cancelling unlock leaves chat list stuck "Loading chats..."](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2647)
closed · 2024-01-19 · Desktop · Windows

**#2539** [Start minimized also searches for a contact named "minimized"](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2539)
closed · 2023 · — · Windows
- Root cause: Command-line arguments were parsed as a share or search intent.

**#3203** [Update prompt unaware of install source](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3203) and **#3129** [Detect multiple Windows install sources](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3129)
closed / open · 2026-07/08 · Desktop · Windows
- Symptom: The update prompt asks for a release that the Store does not have yet. MSIX and EXE installs can both be present with different data directories, and the app silently copies data from one to the other.

## 13. Packaging / install / startup crashes

**#3383** [[Linux] v2.1.4.0 Flatpak: app opens but stays on "Starting..."](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3383) and **#3274** [Taking so long to load on Ubuntu](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3274)
open · 2026-10-27 / 2026-09-13 · Needs More Info, Desktop · Linux (Mint Cinnamon, Ubuntu)
- Symptom: The app never gets past the splash screen. 2.1.3 also did not load, while older versions worked. The output shows `Gdk-CRITICAL gdk_device_get_axis`.
- Root cause: Unknown. The maintainer suspects the GPU or renderer and asked the user to try `FLUTTER_LINUX_RENDERER=software`.

**#3299** [Linux Flatpak hangs at 100% CPU, no window ever shown](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3299)
closed (not planned) · 2026-09-30 · Needs More Info · Linux (Ubuntu 26.04, KDE Wayland)
- Root cause: A stale `~/.cache/fontconfig`. The newer fontconfig in the GNOME 51 runtime loops on old cache files. The fix is `rm -rf ~/.cache/fontconfig`; `fc-cache -r` is not enough. NVIDIA was suspected first.

**#3386** [[Linux][Flatpak] exits with status 1 when mpv starts](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3386)
closed (fixed) · 2026-10-27 · — · Linux (Flatpak)
- Symptom: The process silently quits with status 1 whenever any media player starts, for example for a notification sound or a video.
- Root cause: libmpv was built against FFmpeg 8.1.3 from the GNOME 51 SDK, but `LD_LIBRARY_PATH=/app/bluebubbles/lib` loaded the bundled FFmpeg 8.1.2. mpv calls `exit(1)` on a libavcodec version mismatch.

**#3270 / #3230** [Snap crashes on start (CachyOS KDE 6)](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3270), [SEGV on Kubuntu 26.04](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3230)
closed · 2026-08/09 · Desktop, Priority: Urgent · Linux (snap)

**#2927** [Desktop client crash on Linux (crash loop at "Loading chats")](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2927)
closed · 2025-05-15 · — · Linux (Flatpak)
- Root cause: The upstream bug `window_manager#460`. One incoming message put the app into a crash loop on every restart.

**#2856 / #2433** [Can't run on Asahi Linux (`libmpv.so.1` missing)](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2856)
closed · 2023–2024 · Desktop · Linux aarch64
- Root cause: The arm64 build was made on an older OS and linked against `libmpv.so.1`, while x86_64 used `.so.2`.

**#2747** [Standalone tarball won't launch](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2747)
closed · 2024-05-15 · — · Linux
- Root cause: The tarball needs system `libwebkit2gtk-4.1` and `libmpv`, and its plugin `.so` files must sit next to the binary. The docs were out of date, and the tarball is mainly an input for the Flatpak build.

**#2304 / #2213 / #2577** [GLIBC_2.34 not found on Fedora 37 Flathub](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2304), [segfault on Ubuntu 22.04](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2213), [snap crashes, then won't launch](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2577)
closed · 2022–2023 · — · Linux

**#2814 / #2594 / #2281** [Flatpak runtime EOL](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2814), [Flathub version stale](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2594)
closed · 2022–2024 · — · Linux
- Symptom: The Flathub and Snap packages often lagged behind GitHub releases or ran on EOL runtimes.

**#2716** [Flatpak autostart file uses the wrong command](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2716)
closed · 2024-04-09 · — · Linux (Flatpak)
- Root cause: The app wrote `Exec=/app/bluebubbles/bluebubbles [minimized]`, a path inside the sandbox. It should use `flatpak run app.bluebubbles.BlueBubbles minimized` or the background portal.

**#3288 / #2521** [Allows multiple instances (Nixpkgs)](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3288), [Flatpak relaunch while in tray: `LateInitializationError`](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2521)
closed · 2026-09 / 2023-07 · Desktop · Linux
- Root cause: The single-instance lock (`startup_tasks.dart`) was added only for the Snap and Flatpak builds, so other packages (Nix, AUR, tarball) can run two instances against one DB.

**#3130** [Windows 2.0.0 crashes on startup](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3130)
closed · 2026-07-25 · Desktop, Priority: Urgent · Windows (Store)
- Root cause: A crash in `ucrtbase.dll` (0xc0000409) from `WindowsTaskbar.resetOverlayIcon()`, probably because the msix package shipped outdated redistributables. The Store auto-update rolled it out to all users.

**#2902** [Windows on ARM support](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2902)
open · 2025-03-16 · Priority: Low · Windows ARM
- Root cause: Blocked by Flutter stable, `media_kit_libs_windows_video` (x64-only mpv and ANGLE), and `ffmpeg_kit` (x64-only zip).

## 14. Window / tray / lifecycle

**#3247** [Linux/Wayland: tray "Close App" does not quit the process](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3247)
closed (fixed, PR #3248) · 2026-08-25 · — · Linux (KDE Wayland, AUR)
- Symptom: After close-to-tray, the tray "Close App" leaves the process running (socket, isolates, sync) until it is killed with SIGKILL.
- Root cause: `windowManager.close()` on a hidden GTK window does not fire `onWindowClose`, and `setPreventClose(false)` races. The `exit(0)` call therefore never runs.

**#3060 / #1803** [Linux app does not close (X or tray)](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3060)
closed · 2026-06 / 2021-12 · — · Linux (Mint Cinnamon Flatpak, Arch GNOME)
- Note: This is the same bug class as #3247, and it came back across versions.

**#3246** [Windows: unable to close or kill the app after first install](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3246)
open (PR) · 2026-08-24 · Desktop, Priority: High · Windows (MSIX)
- Symptom: After first-run setup, neither the taskbar nor the tray can close the app, and the user must end it in Task Manager.

**#2818** [Windows tray context-menu actions don't work](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2818)
closed · 2024-10-02 · Desktop · Windows
- Root cause: The upstream bug `tray_manager#53`. With close-to-tray on, the app could not be fully closed.

**#2840 / #2717** [Tray icon shows as three dots on GNOME](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2840), [Flatpak tray icon not working](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2717)
closed · 2024 · Desktop · Linux (GNOME AppIndicator, Flatpak)
- Root cause: The Flatpak lacked `--talk-name=org.kde.StatusNotifierWatcher`. The icon path pointed inside the sandbox (`/app/.../icon.png`), which the host tray cannot read.

**#2649** [Window position not remembered (multi-monitor)](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2649) (#2960 duplicate)
open · 2024-01-20 · Desktop · Windows, Linux Mint (size and position)
- Note: Native Wayland does not let clients set their position, so only size can be restored there.

**#3258** [Linux: relaunch doesn't raise the existing window](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3258)
closed (reporter retracted) · 2026-09-04 · Desktop · Linux (snap, GNOME)
- Symptom: The second launch is blocked, but the app never asks for activation; `_NET_WM_STATE` is empty. This may have been a Mutter change.

**#1968 / #2913** [Notification or tray click doesn't foreground a minimized window](https://github.com/BlueBubblesApp/bluebubbles-app/issues/1968), ["Startup minimized" not working](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2913)
closed · 2022 / 2025 · Desktop · Windows

## 15. Other (privacy, data-model gaps, misc)

**#2889** [Logs go to systemd's journal and include phone numbers and messages](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2889)
closed · 2025-02-06 · — · Linux (Flatpak)
- Root cause: Debug console output (stdout) was on in release builds, and Flatpak forwards stdout to journald. Related to #3289 (password in URLs) and #1704 (FR for redacted logs).

**#2926** [Chatbot messages shown as "Unsupported Interactive Message"](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2926)
open · 2025-05-12 · Priority: Normal · all (web confirmed)
- Root cause: `balloonBundleId = com.apple.messages.chatbot` is treated as an interactive balloon. It should be rendered as plain text.

**#2931** [RCS messages displayed as iMessage](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2931)
open · 2025-05-26 · Priority: Normal · all
- Root cause: The client only knows the `SMS` and `iMessage` services.

**#2598** ["You shared your location" shown when sharing stops](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2598)
open · 2023-12-02 · Priority: Normal · all

**#2746** [Date formats ignore system locale](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2746)
open · 2024-05-15 · Effort: Medium · all

**#2438** [Digital Touch not shown in conversation](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2438)
open · 2023-04-01 · Android · all (desktop shows a placeholder, Android does not)

**#3304 / #2843 / #2593** [Find My ordering and layout](https://github.com/BlueBubblesApp/bluebubbles-app/issues/3304), [Find My layout bugs](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2843), [Find My error with snap](https://github.com/BlueBubblesApp/bluebubbles-app/issues/2593)
open / closed · 2023–2026 · UX, Desktop · all / Linux snap
- Symptom: The map defaults to 0,0, the device list disappears, and AirPods show up as duplicate "single" items. This is low priority for a messaging client.

---

## Server-side bugs that affect clients

- **#2760**: The server emitted unsend events from the wrong fields. This was fixed in a dev build, and the client must handle `retractedParts`.
- **#2952**: Cloudflare tunnel URL changes were not reliably pushed to Firebase.
- **#3278**: Typing indicators get stuck on the remote side (bluebubbles-server#866).
- **#3308**: The server has no authoritative read-state or deletion snapshot. Server PR #853 proposes one.
- **#2660 / #2776**: Older macOS versions (High Sierra) report `dateCreated` later than `dateDelivered`, which breaks ordering.
- **#3328 / #2931**: On macOS 15 and later, RCS adds separate `RCS;+;` chat rows. The server reports RCS as iMessage.
- **#2926**: The server passes `com.apple.messages.chatbot` messages through as balloons.
- **#2558 / #2369**: The server's contacts plugin cannot fetch Google-sourced avatars, and contact export failed.
- **#2766**: Resetting Messages or iCloud sync regenerates chat GUIDs and floods clients with events.
- **#2757**: Server 1.9.6 caused unbounded client memory use. Server 1.9.7 fixed it.
- **#2668**: Not strictly a server bug, but neither side supports URL path prefixes.

---

## Recurring themes

1. **Two sources of truth for UI state.** The DB row and the in-memory GetX/Rx state drift apart. Examples: the unread dot (#3268), a stale chat-list preview (#2751, #2915, #2638), an open chat that stops updating (#2620), and typing into an orphaned controller (#2656). The worst input lag (#3333) came from per-chat controllers in a global registry keyed by chat GUID with racy create and delete. **Design:** Use one store, the SQLite cache, and have every view observe queries on it. Socket and sync handlers write only to that store. Use no registry keyed by GUID.
2. **Incremental sync fetches only new rows.** The `ROWID > last` sync, together with "Reset App" that does not fully reset (#2680, #2816, #3301), leads to missing messages and stale edits, unsends, read state, and deletions (#3308, #2760, #2871). Users are told "delete your app-data folder" again and again. **Design:** Make sync idempotent with upserts keyed by GUID. Run a periodic reconcile window of updates by `dateEdited`, `dateRead`, and `dateRetracted`, not by ROWID alone. Key the local DB by server identity, so a new server gets a clean DB automatically.
3. **Connection fragility.** Problems include sleep and resume (#3256), disconnect versus error handling (#2952), and dynamic URLs that depend on Google OAuth plus Firebase (#3353, #2958, #2914). Custom headers were not applied during setup (#2887), and the password goes in the query string (#3289). **Design:** Use an explicit connection state machine with a resume/network-change trigger, a heartbeat, a REST health check, and debounced UI. Accept several fallback URLs. Send auth in a header, and never log or show it.
4. **Settings persistence is not atomic.** On Linux and Windows, settings were silently lost or corrupted (#3286, #3167, #3057, #2963, #2629). A missing `finishedSetup` key even triggered a DB wipe. **Design:** Store settings in SQLite (transactions) or write them atomically from one owner. Never do destructive work when a key is missing.
5. **The native media stack (libmpv, FFmpeg, media_kit) in-process.** It caused crashes on startup (#3014), silent `exit(1)` (#3386), lockups (#3271), black video (#2625), and ARM blockers (#2902). One bad file can crash the app on every launch. **Design:** Thumbnail and transcode in a subprocess or with a sandboxed decoder. Play notification sounds with a light audio path (for example rodio or the freedesktop sound theme), not a video player.
6. **Linux packaging and the desktop environment.** Issues include stale Flatpak runtimes, fontconfig cache loops (#3299), GPU and renderer problems (#3383), WebKitGTK DMABUF (#2942), tray D-Bus permissions and sandboxed icon paths (#2717, #2840), autostart Exec lines inside the sandbox (#2716), and single-instance locks only in some packages (#3288). **Design:** Avoid a WebView, and do OAuth in the system browser with a loopback redirect. Use the StatusNotifierItem tray with icon pixmaps, not file paths. Use a D-Bus single-instance lock that raises the existing window, and the background/autostart portal.
7. **Window lifecycle on Wayland.** The app does not quit from the tray (#3247, #3060, #1803, #3246). Position is not restored (#2649), and relaunch does not raise the window (#3258). **Design:** Make Quit an explicit app-level exit and not a window close. Use xdg-activation tokens for raising the window.
8. **Keyboard handling.** Problems include Num Lock (#3280), numpad Enter (#2860), IME and emoji-panel desync (#3374, #2624), clipboard races (#3089), and accessibility (#2965). **Design:** Match shortcuts on modifiers, ignoring lock state. Treat `NumpadEnter` as Enter. Use one clipboard read path that is MIME-aware. Test with IME, compose keys, and AT-SPI.
9. **CPU and work on the main thread.** Causes include contact refresh with avatars after every sync (#3362), expensive latest-message queries per chat (#2632, #2645), and idle CPU from animations (#2748, #2372). **Design:** Store `last_message_id` and `unread` on the chat row, add indexes on `(chat_id, date_created)`, and do contact refresh off the main thread only when the data changed (ETag or count). Render only on change.
10. **Identity of chats and handles.** Problems include handles with both phone and email (#3012, #2756), RCS/SMS twin chats (#3328), number normalization (#1621), shared numbers (#2136), and server migration (#3301, #2509). **Design:** Normalize to E.164 and match contacts by best-match rules. Add an optional "merge by contact or `chatIdentifier`" view layer above the raw chat GUIDs.

---

## Appendix: desktop-relevant feature requests (not bugs)

- **#3373**: A persistent jump-to-bottom control, and Esc to scroll to the bottom.
- **#2559**: Smooth mouse-wheel scrolling.
- **#2753**: Configurable keybindings.
- **#2866**: Arrow keys to move between images in the gallery.
- **#2858**: An option to disable the tray icon fully (Linux and GNOME).
- **#2561**: A monochrome tray icon.
- **#3204**: Move action buttons out of the top-right on desktop.
- **#2785**: A global scale factor.
- **#2746**: Locale date formats.
- **#2705**: Local contact management on desktop.
- **#2476**: Pins that sync across clients.
- **#2981 / #2715**: Custom lists, views, sorts, and chat groups.
- **#2756**: Merge threads by contact.
- **#2829**: Emoji tapbacks.
- **#2855**: Tapbacks for SMS and RCS.
- **#2812 / #3122**: Send and display multi-attachment galleries or stacks.
- **#2551**: Captions on attachment replies.
- **#2548**: Display scheduled messages.
- **#2269**: Unlimited scrolling in search context.
- **#2683**: Mentions while editing.
- **#2893**: Pull-to-refresh or manual resync.
- **#2822**: Back up and export the local DB.
- **#347**: Fallback URLs.
- **#2668**: URL path prefixes.
- **#3154**: Enable private-API features automatically when the server supports them.
- **#2925**: Declarative configuration (Nix-friendly).
- **#3312**: Winget.
- **#3129**: Detect conflicting install sources.
- **#2902**: Windows on ARM.
- **#2121**: Back and forward navigation in setup.
- **#1956**: A restart-app button.
- **#2965**: Screen-reader and keyboard accessibility.
- **#1704**: A redacted log mode.
- **#2935**: Hide link previews in high-performance mode.
- **#1156**: An in-app PDF viewer.
- **#2564**: Video player improvements.
- **#2326 / #3277 / #3013**: Disable or tune animations, and reduce motion.
- **#2976**: Show Focus or Do Not Disturb status.
- **#2906**: A VCF contact card viewer.
- **#2973**: Split FaceTime and Find My into separate apps.

---

## Counts by category

Each entry above counts once, including entries that group several issues.
"Open" lists the entries that still have an open issue.

| Category | Entries | Open entries |
|---|---|---|
| connection/socket | 13 | 6 (#3353, #3289, #2668, #2661, #3384, #347) |
| sync/missing messages | 12 | 8 (#2751, #3008, #3231, #3308, #2760/#2871, #2776, #2565, #2269) |
| duplicate messages/chats | 4 | 2 (#2702, #2756) |
| notifications | 9 | 3 (#3371, #2656, #2795) |
| performance/slowness | 6 | 1 (#2632) |
| scrolling/rendering | 11 | 6 (#2699, #3373, #2559, #3259, #3195, #2740) |
| text input/composer | 12 | 9 (#3280, #2860, #3374, #2624, #2965, #2419, #2801, #3047, #2683) |
| attachments/media | 12 | 4 (#2883, #1611, #3196, #2659) |
| reactions/tapbacks | 3 | 3 (#2829, #3275, #2855) |
| contacts/names | 8 | 3 (#2610, #2705, #3170) |
| read receipts/typing | 2 (+#3308 cross-ref) | 1 (#3278) |
| settings/setup | 10 | 1 (#3129) |
| packaging/install | 13 | 2 (#3383/#3274, #2902) |
| window/tray | 8 | 2 (#3246, #2649) |
| other | 7 | 6 (#2926, #2931, #2598, #2746, #2438, #3304) |
| **Total** | **130** | **57** |
