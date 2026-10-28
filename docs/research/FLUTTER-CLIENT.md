# BlueBubbles Flutter client: internal behaviour reference

Source studied: `third_party/bluebubbles-app` at commit `7e5f1bc` (Flutter/Dart,
read-only). All paths below are relative to that directory. This file covers the
client's logic. The server API is documented separately from the server source;
endpoints are named here only where the client's use of them matters.

The repo has its own design notes. They match the code and are worth reading:
`docs/ARCHITECTURE.md`, `docs/DECISIONS.md`, `docs/MESSAGE_SEND_FLOW.md`,
`docs/MESSAGE_RECEIVE_FLOW.md`.

Each section ends with a **For the Rust client** list: what to copy and what to
avoid.

---

## 0. Architecture in one page

- **State and DI.** GetIt holds singleton services (`lib/services/services.dart`
  has shorthand getters such as `ChatsSvc`, `SettingsSvc`, `HttpSvc`, `SocketSvc`).
  GetX `Rx*` observables drive `Obx()` rebuilds. Thin per-entity reactive wrappers
  (`lib/app/state/chat_state.dart`, `message_state.dart`, `handle_state.dart`,
  `attachment_state.dart`) mirror DB fields as observables. Only `update*Internal()`
  methods called by services write to them.
- **Background isolate.** `lib/services/isolates/global_isolate.dart`. Most DB writes
  and all outgoing HTTP sends run in a long-lived Dart isolate. "Actions"
  (`lib/services/backend/actions/*`) run in the isolate and return ObjectBox IDs.
  "Interfaces" (`lib/services/backend/interfaces/*`) send the request and then
  re-read ("hydrate") the IDs on the main thread with `box.get(id)`. Idle timeout is
  5 min (`idleTimeout = Duration(minutes: 5)`) and startup timeout is 30 s. A second
  isolate (`IncrementalSyncIsolate`, idle timeout zero) runs incremental syncs.
- **DB.** ObjectBox on Android and desktop (`lib/database/io/*`). Web stubs are in
  `lib/database/html/*`. Shared DTOs are in `lib/database/global/*`.
- **Inbound pipeline.** socket event → `ActionHandler.handleEvent`
  (`lib/services/backend/action_handler.dart`) → `IncomingMessageHandler`
  (`lib/services/backend/incoming_message_handler.dart`, FIFO, 5 concurrent, serial
  per GUID) → `chat.addMessage()` (DB) → `ChatsService` / `MessagesService` state
  updates → `NotificationsService`.
- **Outbound pipeline.** UI → `OutgoingMessageHandler.queue()`
  (`lib/services/backend/outgoing_message_handler.dart`, one send at a time) →
  optimistic DB insert with a temp GUID → HTTP in the isolate → race with the socket
  echo → swap the temp GUID for the real GUID.

**For the Rust client:** the isolate split exists because Dart cannot do DB work off
the UI thread any other way. Most of the complexity in this codebase is ID
hydration, stale objects across isolates, and "is this the isolate copy of the
service?" bugs. In Rust, use one async runtime with a DB actor or connection pool
(SQLite is fine) and send typed events to the GPUI side. Keep the idea of small
per-entity reactive state (GPUI `Model<T>` per chat or message). Do not copy the
duplicated service graph.

---

## 1. Connection and auth

### 1.1 Setup and login

File: `lib/app/layouts/setup/pages/sync/server_credentials.dart`.

On desktop the setup screen offers two paths. QR scanning is mobile only
(`mobile_scanner` has no desktop support, and the button is gated
`!kIsWeb && !kIsDesktop`).

1. **Manual entry: URL and password.** `connect(url, password)`:
   - Strips a trailing `/`. Checks the URL with `isValidServerAddress` (scheme must be
     http or https and the host must not be empty).
   - `sanitizeServerAddress` (`lib/helpers/network/network_helpers.dart`) removes
     `"` characters and trims. If there is no scheme, it adds `https://` for
     `ngrok.io`, `trycloudflare.com` and `zrok.io` hosts. Otherwise it adds
     `http://`.
   - Saves the password to `settings.guidAuthKey` and the URL to
     `settings.serverAddress` (`saveNewServerUrl`, in
     `lib/helpers/backend/settings_helpers.dart`).
   - Calls `GET /server/info` and `GET /fcm/client`. A 401 from `/server/info` gives
     "Authentication failed. Incorrect password!". Any other non-200 gives "Failed
     to connect". If FCM data is missing and the URL is ngrok or trycloudflare, it
     reports "Firebase is required when using Ngrok or Cloudflare!". Otherwise it
     parses the FCM data and saves it (see 1.4). Then it calls
     `SocketSvc.restartSocket()` and moves to the sync page.
2. **"Sign in with Google"** (`lib/helpers/ui/oauth_helpers.dart`). The OAuth scopes
   are `cloudplatformprojects`, `firebase` and `datastore`. The client lists
   `https://firebase.googleapis.com/v1beta1/projects`. For each project it reads the
   server URL from RTDB (`https://<rtdb>.firebaseio.com/config.json?token=…`, field
   `serverUrl`) or from Firestore (`…/databases/(default)/documents/server/config`,
   `fields.serverUrl.stringValue`). The user picks a project and then types the
   password. Desktop uses a WebKit2GTK web view (`desktop_webview_auth`, a fork
   that uses webkit2gtk-4.1).

**QR payload (mobile):** a JSON array. Index 0 is the password and index 1 is the
server URL: `final List result = jsonDecode(response); password = result[0];
serverURL = sanitizeServerAddress(address: result[1]);`. The client ignores any
further elements.

### 1.2 How the password goes on the wire

`lib/services/network/http_service.dart`:

```dart
params['guid'] = SettingsSvc.settings.guidAuthKey.value;   // buildQueryParams()
```

- Every REST call sends the password as the **`?guid=<password>` query parameter**.
  The base URL is `"$origin/api/v1"`.
- Headers are `settings.customHeaders` (user editable). The client adds
  `ngrok-skip-browser-warning: true` when the URL contains `ngrok`, and
  `skip_zrok_interstitial: true` for `zrok`.
- Dio timeouts come from `settings.apiTimeout` (default **30000 ms**) for connect,
  send and receive. Attachment upload and download multiply this by **12**.
- TLS: `badCertificateCallback = shouldAcceptCertificate`
  (`lib/services/network/http_overrides.dart`). It accepts **any** invalid
  certificate whose host matches the configured server host, including wildcard
  handling. There is no pinning or trust-on-first-use.
- `runApiGuarded` retries a request **once** if it gets a 502 and the URL contains
  `trycloudflare`.
- `ApiInterceptor` turns error responses into a JSON envelope
  `{status, error:{type,error,message}}` for JSON requests. It redacts `guid` and
  `password` from the logged params.
- `originOverride` is used for LAN detection. If `settings.localhostPort` is set,
  `NetworkTasks.detectLocalhost()` (`lib/helpers/network/network_tasks.dart`) first
  pings the `local_ipv4s` / `local_ipv6s` that `/server/info` reports, over https
  and then http. If none answers, it scans the /24 subnet on that port, at most once
  every 2 min. The first address that answers `pong` becomes the origin for both
  HTTP and the socket.

### 1.3 Socket.IO client

File: `lib/services/network/socket_service.dart`. Library: `socket_io_client`
**3.1.4** (pub). This is a Socket.IO v4 client and uses the **EIO=4** protocol.

```dart
OptionBuilder()
  .setQuery({"guid": password})
  .setTransports(['websocket', 'polling'])
  .setExtraHeaders(HttpSvc.headers)
  .disableAutoConnect()
  .enableForceNew()                 // needed, else io() reuses a cached Manager with the old auth
  .enableReconnection()             // unlimited attempts
  .setReconnectionDelay(1000)
  .setReconnectionDelayMax(60000)
```

Events it listens to:

- On desktop only (on Android, FCM delivers these): `group-name-change`,
  `participant-removed`, `participant-added`, `participant-left`, and
  `incoming-facetime` (its payload is a JSON string that needs a second
  `jsonDecode`).
- Always: `new-message`, `updated-message`, `typing-indicator`,
  `chat-read-status-changed`, `imessage-aliases-removed`,
  `ft-call-status-changed`, `new-findmy-location`.

The socket only receives. The client sends nothing over it except the heartbeat.
Everything else goes over REST. `sendMessage()` exists and handles
`{encrypted:true}` replies by AES-decrypting with the password
(`decryptAESCryptoJS`), but nothing calls it.

**Reconnect logic**, with three separate concerns:

1. Transient failure: socket.io's own backoff (1 s, doubling with jitter, capped at
   60 s, unlimited attempts).
2. **Rebuild.** The URI and auth are fixed when the Manager is created, so a new URL
   or password means `closeSocket()` + `startSocket()`. A server-initiated
   disconnect sets `skipReconnect`, after which socket.io never retries
   (`_socketGaveUp`). Only a rebuild recovers from that.
3. **URL rediscovery.** After errors, it waits **30 s** and then asks Firebase for a
   new `serverUrl`. The delay doubles up to **15 min**, and each round is capped at
   **60 s**. On desktop it reads RTDB `config/serverUrl` with `firebase_dart`, or
   Firestore `server/config`. It rebuilds if the URL changed or socket.io gave up.
   If it is using a LAN override, it probes the override again.
- **Client heartbeat.** The server's engine.io ping takes up to about 3 min to notice
  a dead link, so every **30 s** the client sends `emitWithAck("get-fcm-client", {})`
  with a **5 s** ack timeout. After **2** misses in a row it rebuilds the socket.
  A code comment explains why this event: "get-server-config would delete the
  server's live password".
- **Connectivity** (`connectivity_plus`). Changes are debounced for **3 s**. When
  the socket is in `error` or `disconnected` it rebuilds, so the next attempt does
  not wait for the backoff. Desktop ignores "none" results (NCSI false negatives on
  Windows). On Linux the plugin throws when NetworkManager is missing (for example
  a Flatpak with no D-Bus access), and the client only logs it.
- **Bad password.** The server drops the socket with `io server disconnect` and no
  reason. On that disconnect the client pings `/server/ping`. A 401 sets
  `authFailed` and shows "Incorrect password".

**What happens on (re)connect:** `handleStatusUpdate(connected)` calls
`NetworkTasks.onConnect()`, which calls **`SyncSvc.startIncrementalSync()`**. So a
reconnect does start a sync, but with the cooldown below.

**Desktop window focus = "resume".** `DesktopWindowListener.onWindowFocus` calls
`LifecycleSvc.open()` → `StartupTasks.onAppResume()`
(`lib/helpers/backend/startup_tasks.dart:479`). This calls
`SocketSvc.checkConnection()` (an immediate heartbeat), re-runs localhost
detection, and calls `startIncrementalSync(useGlobalIsolate: true)`. Blur calls
`close()`, which marks the active chat "dead" and stops typing. On desktop it does
**not** disconnect the socket. Android does disconnect.

### 1.4 Firebase config (desktop)

`GET /fcm/client` returns a google-services.json-shaped object. `FCMData.fromMap`
(`lib/database/io/fcm_data.dart`) reads these fields:
`project_info.project_id`, `project_info.storage_bucket`,
`project_info.firebase_url`, `client[0].api_key[0].current_key`,
`client[0].oauth_client[0].client_id` (cut at the first `-`), and
`client[0].client_info.mobilesdk_app_id`. Desktop uses this only to find the
server URL. It does not use it for push.

### 1.5 Known fragilities

- The password sits in every URL (`?guid=`). This includes attachment download
  URLs and the socket handshake, so it ends up in proxy and tunnel logs.
- Any certificate is accepted for the server host, so a MITM is possible.
- The 30 s **sync cooldown** (see 2.2) is measured from the *start* of the last
  sync. If the connection drops and comes back within 30 s of a sync, the reconnect
  sync is skipped. Messages sent during the gap that the socket missed then wait
  for the next trigger (window focus, the next reconnect after the cooldown, or
  Ctrl+G).
- Much of the socket code is defensive patching around socket.io Manager caching
  and `skipReconnect`. The long comments in `socket_service.dart` record past bugs:
  the app stayed offline for good, or listeners were attached to a stale socket.

**For the Rust client:** use `rust_socketio` (async), or speak engine.io v4 over a
plain websocket. Put the password in the handshake query as `guid`. Run your own
heartbeat (ack-based, 30 s / 5 s / 2 misses) and always rebuild the client after a
server-initiated disconnect. On every successful connect, run an incremental sync
with **no** cooldown that can skip it. Coalesce concurrent triggers instead
(single-flight, plus a "dirty" flag that runs once more after the current sync).
Consider sending the password in a header, if the server accepts one, rather than
the query string, and offer TOFU certificate pinning.

---

## 2. Sync

Files: `lib/services/backend/sync/*`, `lib/services/backend/actions/sync_actions.dart`.

### 2.1 Initial full sync

`FullSyncManager` (`full_sync_manager.dart`):

1. `GET /chat/count` gives `totalChats`.
2. `streamChatPages` makes **200 chats per page** with
   `POST /chat/query {offset, limit:200, with:["lastMessage"], sort: null}`. The
   server's default sort applies (web asks for `"lastmessage"`).
   `syncTimeFilter` is an optional client-side filter on `lastMessage` date. Setup
   defaults it to 6 months (`15552000000` ms). Choices are 1 month to all.
3. `Chat.bulkSyncChats(page)` saves the chats.
4. For each chat (skipping `chatIdentifier` that starts with `urn:biz`), it makes
   one page of messages:
   `GET /chat/<guid>/message?after=0&before=<syncStart>&offset=0&limit=N&with=attachments,handle,message.attributedBody,message.messageSummaryInfo,message.payloadData`
   where **N defaults to 25** (a setup slider from 0 to 50).
   - A chat with no handles is soft-deleted. With "skip empty chats" (default on),
     a chat with 0 messages is soft-deleted.
   - `SyncInterface.bulkSyncData(chatData, messagesData)` writes the messages.
   - If `syncGroupChatIcons` is set, it also fetches `GET /chat/<guid>/icon`.
5. `complete()` calls `ContactsSvcV2.syncContactsToHandles()`, then
   `ChatsSvc.reset(); ChatsSvc.init(force:true)` (a full in-memory reload).
6. Before starting, `startFullSync()` sets `lastIncrementalSync = now`. The ROWID
   marker stays **0**, so the first incremental sync takes the timestamp path.

Older history is never bulk-downloaded. It comes from the server on demand when the
user scrolls (2.4).

### 2.2 Incremental sync

`SyncService.startIncrementalSync` (`sync_service.dart`), which runs
`SyncActions.performIncrementalSync` in an isolate (`IncrementalSyncManager`):

- Guards: it returns at once if a sync is already running, or if the last sync
  started less than **30 s** ago (`_incrementalSyncCooldown`). A *failed* sync
  clears the cooldown.
- Triggers: socket connect, window focus (desktop), app resume, Ctrl+G or Alt+G,
  and a manual "sync" in settings.
- Markers, stored as settings: `lastIncrementalSync` (ms) and
  `lastIncrementalSyncRowId`.
- Strategy is chosen by server version. The version code is
  `major*100 + minor*21 + patch` (`lib/models/server_details.dart`).
  - **≥ 1.6.0 (code 226) and rowId > 0.** `POST /message/query` with
    `where: [{statement:"message.ROWID > :startRowId", args:{startRowId}}]`,
    `limit: 1000`, `offset`, `sort: DESC`, and
    `with: [chats, chats.participants, attachments, handle, attributedBody, messageSummaryInfo, payloadData]`.
    The response's `metadata.total` sets the number of pages.
  - **≥ 1.2.0.** `GET /message/count?after=<ts>` gives the total. Then it pages
    through `POST /message/query {after: ts, before, offset, limit:1000}`.
  - **< 1.2.0.** Same, but the count is doubled because old servers dropped
    null-text rows.
- For each page (`syncMessages`):
  - It groups messages by chat, using `msg.chats[]`. One message can list more than
    one chat.
  - It tracks the highest `originalROWID` and the highest `dateCreated`.
  - For chats that are unknown locally or have no handles, it calls
    `GET /chat/<guid>?with=participants`, one request per chat.
  - It applies the latest `itemType==2` group-rename `groupTitle` to the chat's
    `displayName`. It records the latest group-photo event per chat
    (`groupActionType` 1 = changed, 2 = removed).
  - It upserts the chats, then calls `bulkSyncData` once per chat. It emits
    `incrementalSyncPageComplete` with `{messageIdsByChat, latestMessageIdPerChat}`
    so the UI updates chat subtitles, and adds messages to open conversations in
    chunks of 50.
- `complete()` saves the markers. `lastIncrementalSync` becomes the client time when
  the sync **started** (timestamp mode). `lastIncrementalSyncRowId` becomes the max
  ROWID seen. It adds or updates `ChatState`s, then re-fetches group icons
  (`GET /chat/<guid>/icon`) or clears them.
- After every incremental sync, it **also** runs `performContactSyncToHandles()`. On
  desktop that is a full `GET /contact?extraProperties=avatar` download. See §7.

**Gaps in this design:**

- ROWID mode only fetches **new rows**. Changes to existing rows while offline
  (delivered or read receipts, edits, unsends, `dateDeleted`, error changes,
  `didNotifyRecipient`) are **never back-filled**. The socket `updated-message` is
  the only way they arrive. Chat read state that changed elsewhere while offline is
  missed too.
- Timestamp mode uses the client clock (`syncStartedAt`) against Mac timestamps, so
  clock skew can drop messages.
- Messages that arrive through incremental sync **never produce notifications**.
  Nothing in `sync/` calls `NotificationsService`. A message received while the
  socket was down shows up silently.

### 2.3 Reconciling socket events with sync; dedup

- **Upsert by GUID everywhere.** `bulkSyncData` (`sync_actions.dart`) runs one
  ObjectBox write transaction:
  1. Handles are upserted by `uniqueAddressAndService` (`"$address/$service"`).
     Addresses are formatted before the transaction.
  2. Chats are upserted by `guid`. A non-empty incoming `displayName` wins.
     `chatIdentifier` and `style` are only filled if null. Archived is sticky.
     Participants are replaced (`handles.clear(); addAll`).
  3. Messages are found with `Message_.guid.oneOf(guids)`. Existing rows are merged
     with `Message.merge(existing, incoming)`. The message is linked to its chat and
     to its handle by server `handleId` ↔ `Handle.originalROWID`. `hasReactions` is
     set on the parent of any `associatedMessageGuid`.
  4. Attachments are upserted by GUID. **An existing attachment row is kept as is,
     not merged.**
  5. Each chat's `latestMessage` is recomputed with a query.
- `Message.merge` (`lib/database/io/message.dart:813`) only ever moves dates forward:
  `dateCreated`, `dateDelivered`, `dateRead`, `datePlayed` and `dateDeleted` take the
  later value. `attributedBody` and `messageSummaryInfo` are replaced when the
  incoming value is non-empty. `error` is overwritten. Booleans such as
  `hasAttachments` and `hasReactions` only go from false to true. `metadata` is a
  shallow dict merge.
- Socket path dedup (`incoming_message_handler.dart`):
  - `_processedGuids` is a ring of the **last 100 GUIDs**.
  - `_inflightByGuid` makes payloads for the same GUID run one after another
    (socket and FCM can race).
  - `new-message` checks whether the real GUID **or** the `tempGuid` already exists.
    If so, it is treated as an update.
  - An `updated-message` for a message not yet in the DB is parked. The cap is
    **500** entries and the timeout is **10 s**. It is replayed after the matching
    `new-message`.
  - `ActionHandler` holds back an `isFromMe` `new-message` that has no `tempGuid`
    for **500 ms**. If an `updated-message` carrying the tempGuid arrives in that
    time, the `new-message` is dropped. Side effect: every message the user sent
    from another device shows up 500 ms late.
- Messages are linked to chats by the embedded `chats[0]`
  (`Chat.fromMap(payload.data['chats'].first)`). A payload with an empty `chats`
  array throws.

### 2.4 Paging inside a conversation

`MessagesService.loadChunk(offset, limit = 25)`
(`lib/services/ui/message/messages_service.dart:1475`):

- Local query: `ChatActions.getMessagesAsync`
  (`lib/services/backend/actions/chat_actions.dart:720`). It sorts by `dateCreated`
  descending and uses **offset/limit**. The offset has
  `struct.reactions.length` added, because reactions are rows in the same table.
- If the page comes back empty, it fetches from the server with
  `GET /chat/<guid>/message` (`ChatsSvc.getMessages`), saves, and queries again.
- After the query, messages whose `handleId` matches no participant and no stored
  handle are silently **dropped** from the result. Messages can vanish when handle
  data is incomplete.
- Offset paging is not stable when rows are added or removed while scrolling (new
  incoming messages, reactions arriving). Expect duplicate or skipped rows. Keyset
  paging on `(dateCreated, rowid)` avoids this.

### 2.5 Local DB schema (ObjectBox, desktop + Android)

Dumped from `lib/generated/objectbox-model.json` (`[idx]` = indexed):

| Entity | Fields |
|---|---|
| **Chat** | id, guid[idx], chatIdentifier, isArchived, muteType, muteArgs, isPinned, hasUnreadMessage, dbOnlyLatestMessageDate, displayName, customAvatarPath, pinIndex, autoSendReadReceipts, autoSendTypingIndicators, textFieldText, textFieldAttachments, dateDeleted, style, lockChatName, lockChatIcon, lastReadMessageGuid, customBackgroundPath, dbLatestMessageId[idx], customThemeLight/Dark, wallpaperType, dynamicWallpaperId/Config; ToMany `handles` |
| **Message** | id, originalROWID, guid[idx], handleId (server handle ROWID), otherHandle, text, subject, country, dateCreated[idx], dateRead, dateDelivered, isFromMe, hasDdResults, datePlayed, itemType, groupTitle, groupActionType, balloonBundleId, associatedMessageGuid, associatedMessageType, associatedMessagePart, expressiveSendStyleId, hasAttachments, hasReactions, dateDeleted, threadOriginatorGuid, threadOriginatorPart, bigEmoji, error, errorMessage, chatId[idx], dbAttributedBody (JSON), dateEdited, dbMessageSummaryInfo (JSON), dbPayloadData (JSON), dbMetadata (JSON), wasDeliveredQuietly, didNotifyRecipient, isBookmarked, isDelivered, handleRelationId[idx], hasEffectPlayed |
| **Handle** | id, originalROWID, address, formattedAddress, country, color, defaultPhone, defaultEmail, uniqueAddressAndService[idx], service |
| **Attachment** | id, originalROWID, guid[idx], uti, mimeType, isOutgoing, transferName, totalBytes, height, width, webUrl, messageId[idx], dbMetadata, hasLivePhoto, isDownloaded, metadata, exif |
| **ContactV2** | id, nativeContactId[idx], displayName, nickname, first/last/middleName, namePrefix/Suffix, company, avatarPath, addresses, dbPhoneNumbers, dbEmailAddresses, isNative; ToMany `handles` |
| Other | FCMData, ThemeStruct/ThemeEntry/ThemeObject, CustomGroup (ToMany chats), ScheduledMessage |

Notes:
- `associatedMessageGuid` is normalised when parsed:
  `json["associatedMessageGuid"].replaceAll("bp:", "").split("/").last`. The part
  number comes from `p:N/` when `associatedMessagePart` is missing.
- `Message.text` is `sanitizeString(attributedBody[0].string ?? json.text)`, which
  removes U+FFFC.
- On desktop the store lives in `getApplicationSupportDirectory()/objectbox`. On
  Linux that is `~/.local/share/app.bluebubbles.BlueBubbles/objectbox` (or the
  Flatpak/Snap equivalent). Attachments are in `…/attachments/<guid>/<name>`, and
  there are sibling `avatars`, `contact_avatars`, `messages/<guid>/embedded-media`,
  `url_previews`, `logs` and `temp` folders
  (`lib/services/backend/filesystem/filesystem_service.dart`).
- Settings are not in the DB. They live in a custom JSON SharedPreferences store with
  a cross-isolate lock file (`lib/services/backend/settings/desktop_shared_preferences_store.dart`).

**For the Rust client:** use SQLite with `UNIQUE(guid)` and do every write as an
upsert inside one transaction per page, with merge rules like `Message.merge`
(dates only move forward). Keep a server ROWID high-water mark **and** a "last
seen updated" timestamp. On reconnect, also back-fill updates, for example with
`/message/query` on `dateRead`/`dateDelivered`/`dateEdited` after T, if the server
supports it (check the sibling server doc). Notify for messages found by sync whose
`dateCreated` is newer than the last-connected time and that are not from me. Use
keyset paging for conversation history.

---

## 3. Sending

Main file: `lib/services/backend/outgoing_message_handler.dart`. UI entry:
`lib/app/layouts/conversation_view/widgets/message/send_animation.dart`.

### 3.1 Lifecycle

1. **Build.** `SendAnimation.send()` creates one `Message` **per attachment**, then
   one for text and subject if either is non-empty. For a reply, only the **first**
   attachment message (or the text message when there are no attachments) carries
   `threadOriginatorGuid` and `threadOriginatorPart = "<replyPart>:0:0"`. All of
   them carry `expressiveSendStyleId`.
2. **Temp GUID.** `generateTempGuid()` sets `guid = "temp-" + randomString(8)`. For
   an attachment, `attachment.guid = message.guid`. `queue()` assigns the GUID for
   text through `_ensureTempGuid`. "Sending" means
   `isFromMe && guid.startsWith("temp")`.
3. **Optimistic insert.** `_persistOutgoingMessages()` calls `chat.addMessage()` (a
   DB write with the temp GUID), then `MessagesSvc.addNewMessage()` and
   `ChatsSvc.updateChatLatestMessage()`. A reaction is added to the parent's
   `associatedMessages`. Prep is retried 3 times (250 ms × attempt). If it still
   fails, the message is shown as failed and is not dropped.
4. **Serial queue.** `_processNext()` sends **one item at a time across all chats**.
   After **5 s** a timer pushes `chat.sendProgress` to 0.9.
5. **HTTP**, from the isolate (`SendMessageActions` → `MessageApi`):
   - Text: `POST /message/text`
     `{chatGuid, tempGuid, message, method, [effectId, subject, selectedMessageGuid, partIndex, ddScan]}`.
     The bracketed fields are only included when private API and private send are
     enabled. A `message` that is empty but has a subject is sent as `" "`.
   - Mentions: `POST /message/multipart`
     `{chatGuid, tempGuid, parts:[{text, mention, partIndex}], effectId, subject, selectedMessageGuid, partIndex, ddScan}`.
   - Attachment: `POST /message/attachment` multipart form
     `attachment=<file>, chatGuid, tempGuid, name, method`, and with private API
     also `effectId, subject, selectedMessageGuid, partIndex, isAudioMessage`.
     Upload progress is sent from the isolate to the bubble.
   - Tapback: `POST /message/react {chatGuid, selectedMessageText, selectedMessageGuid, reaction, partIndex}`.
     No tempGuid is sent.
6. **Method choice** (`_resolveMethod`): `"private-api"` if
   `enablePrivateAPI && (privateAPISend | privateAPIAttachmentSend)`, **or** if the
   message has a subject, a reply target or an effect. Otherwise `"apple-script"`.
   `ddScan = !isMinSonoma && text.hasUrl`. On macOS < 11 a text that starts or ends
   with a URL is split into two messages (`_buildOutgoingMessages`).
7. **Race and match.** Path A: the socket `new-message` or `updated-message` carries
   `tempGuid`. Path B: the HTTP response body `data` is the server message. Both
   lead to `completeSendProgressIfExists(tempGuid)`, and the second arrival does
   nothing. `_matchMessageWithExisting(tempGuid, real)`:
   - If the real GUID already exists, it merges, deletes the temp row, and re-keys
     the UI state.
   - Otherwise it renames the temp row's GUID in place (`Message.replaceMessage`).
   - If that throws, it falls back to a fresh insert.
   - It moves the attachment directory `attachments/temp-xxx` →
     `attachments/<realGuid>` (`_matchAttachmentWithExisting`), and fixes the chat's
     latest-message pointer.
   - `MessagesSvc.updateMessage(real, oldGuid: temp)` moves the same `MessageState`
     under the new key, so the bubble does not remount.
8. **Errors** (`_finalizeOutgoingFailure`, `lib/helpers/network/network_error_handler.dart`).
   The GUID **stays temp**. `error` is set to a code: 502 → badGateway, 504 or a
   timeout → gatewayTimeout, 404 → notFound, connection error → connectionRefused,
   else clientError. `errorMessage` is filled in. If the window is not focused or
   the chat is not open, a "Failed to send" notification is posted. With
   `cancelQueuedMessages`, the rest of that chat's queue is failed with "Canceled
   due to previous failure".
   - **Retry is manual only.** The user taps the error. The old row is deleted and a
     new item is queued (`message_error_helper.dart`).
   - If a send times out on the client but succeeded on the server, the later socket
     echo (matched by tempGuid) fixes the row. A manual retry before that echo
     arrives sends a **duplicate**.
9. **Cancel.** `cancelMessage(tempGuid)` and `cancelPendingForChat()` only work on
   items that have not been sent yet.

### 3.2 Replies

`selectedMessageGuid = threadOriginatorGuid` and
`partIndex = int.parse(threadOriginatorPart.split(':')[0])`.
`threadOriginatorPart` uses the format `"<part>:<start>:<len>"`. The client builds
`"$part:0:0"`. A reply needs private API, so the method is forced to `private-api`.

### 3.3 Tapbacks

- Types are **strings**, not numbers. The server converts Apple's numeric types
  (`lib/helpers/ui/reaction_helpers.dart`):

  | string | emoji | verb | removal string |
  |---|---|---|---|
  | `love` | ❤️ | loved | `-love` → "removed a heart from" |
  | `like` | 👍 | liked | `-like` |
  | `dislike` | 👎 | disliked | `-dislike` |
  | `laugh` | 😂 | laughed at | `-laugh` |
  | `emphasize` | ‼ (client shows ❗) | emphasized | `-emphasize` → "removed an exclamation from" |
  | `question` | ❓ | questioned | `-question` |

  (Apple: 2000–2005 add, 3000–3005 remove. The server maps these. Confirm in the
  server doc.) `sticker` is recognised (`isSticker`). **iOS 17/18 emoji and sticker
  tapbacks are not supported.** They show up as unknown associated messages.
- Sending (`message_popup_holder.dart:181`): a temp `Message` with
  `associatedMessageGuid = parent.guid`, `associatedMessageType = reaction`,
  `associatedMessagePart = part`. **Removal:** picking the reaction you already have
  sends `"-$type"` (`message_popup.dart:393`: `selfReaction == e ? "-$e" : e`).
- Rendering (`getUniqueReactionMessages`): sort the reactions newest first and keep
  the newest per sender (`isFromMe ? 0 : handleId`). If that newest one starts with
  `-`, the sender has no reaction. Reactions are stored per part
  (`associatedMessagePart`) and drawn per part.
- Quick tapback on double-click: `settings.quickTapbackType`. Keyboard:
  Ctrl+Shift+1..6 react to the most recent message.

### 3.4 Edits, unsend, effects

- Edit: `POST /message/<guid>/edit {editedMessage, backwardsCompatibilityMessage, partIndex}`.
  Unsend: `POST /message/<guid>/unsend {partIndex}`. Both are optimistic
  (`MessagesService.editMessage` / `unsendMessage`). On failure the change is
  reverted and the error code is set to `10006` or `10007`. The server's reply is
  passed through `IncomingMsgHandler`.
- Rendering comes from `messageSummaryInfo[0]`. `editedParts` plus
  `editedContent[part]` give the edit history (the list minus its last entry,
  because the last entry is the current text). The part shows an "Edited" label
  (`message_properties.dart`). `retractedParts` replaces that part with an unsent
  part. The chat event reads "You unsent a message. Others may still see the
  message on devices where the software hasn't been updated" or "X unsent a
  message". The chat list subtitle shows "Unsent message" when there is no text and
  `dateEdited` is set.
- Effects: `effectMap` in `lib/helpers/types/constants.dart`, for example
  `slam → com.apple.MobileSMS.expressivesend.impact`, `loud`, `gentle`,
  `invisible ink → …expressivesend.invisibleink`, and screen effects
  `com.apple.messages.effect.CK{Echo,Spotlight,HappyBirthday,Confetti,Heart,Lasers,Fireworks,Sparkles}Effect`.
  A bubble effect plays once and then sets `hasEffectPlayed` on the message. The
  previews show "Message sent with Invisible Ink".

**For the Rust client:** copy the tempGuid scheme (`temp-<8 chars>`) and the "rename
in place, keep the same UI node" swap. Make the swap idempotent under both orders
(HTTP first or socket first). Do not send everything through one global serial
queue. Use one queue per chat, so one stuck upload does not block every other
chat. Before retrying, check the server for a message with the tempGuid, or wait
for the echo, to avoid duplicates. Support the newer emoji tapbacks, which the
Flutter client lacks.

---

## 4. Rendering

### 4.1 Message parts and display text

`MessageState.buildMessageParts()` (`lib/app/state/message_state.dart:700`):

1. If `attributedBody[0].runs` is not empty, the runs are sorted by `range[0]`.
   `range = [start, length]`.
   - Runs with the same `attributes.messagePart` are joined into one `MessagePart`.
   - A run with `attributes.attachmentGuid` (`__kIMFileTransferGUIDAttributeName`)
     becomes an attachment part. The text there is U+FFFC.
   - A run with `attributes.mention` adds a `Mention{address, range}` relative to
     the part.
   - The subject goes on the first part.
2. Apply `messageSummaryInfo` edits and retractions (see 3.4).
3. If there are no runs:
   - For a plain message with no payloadData, not a legacy URL preview, not
     interactive and not a group event, there is one part per attachment
     (`part = index`).
   - Interactive messages get an empty part 0.
   - If `fullText` is not empty (or the message is a group event), a final text part
     is added. `fullText = sanitize([subject, text].join("\n"))`.
4. Adjacent parts that are only images or videos are merged into a gallery
   (`isMediaGallery`).

**Bug to avoid:** `attributedBodyToMessagePart` uses `body.runs.forEachIndexed((i, e)
async {...})` and has an `await Attachment.findOneAsync()` inside. That part is
added **after** the function returns, so an attachment that is not already loaded
can be missing from its part until a rebuild.

Text spans (`lib/helpers/ui/message_widget_helpers.dart`):
- The subject is bold, followed by `\n`.
- Mentions are bold. On desktop, clicking a mention does nothing.
- Links come from this regex:
  `((https?://)|(www\.))[-a-zA-Z0-9@:%._+~#=]{1,256}\.[a-zA-Z0-9()]{1,6}([-a-zA-Z0-9/()@:%_.~#?&=*\[\]]*)\b`.
  They are underlined and open externally. Desktop has no other entity detection
  (ML Kit is mobile only).
- Big emoji: at most 3 emoji and nothing else. The font is scaled ×3.
- An optional downloaded "Apple Color Emoji" font is applied to emoji runs.
- Invisible ink is a bubble effect.

URL previews (`lib/app/layouts/conversation_view/widgets/message/interactive/`):
- The client prefers the server's `payloadData.urlData` (Apple's LP metadata:
  title, summary, image attachment).
- If that is missing, it fetches page metadata itself (`lib/helpers/network/metadata/*`:
  OpenGraph, JSON-LD, oEmbed, a URL safety guard, and a disk cache in
  `url_previews`). This is limited by `linkPreviewPolicy` (default
  `contactsOnly`).
- A "legacy" preview means `balloonBundleId == com.apple.messages.URLBalloonProvider`
  with `hasDdResults`, or `(hasDdResults || isFromMe)` with text that is only a URL.

Chat list subtitle and notification text (`getNotificationText`, in
`lib/helpers/types/extensions/extensions.dart:343`), checked in this order:
1. Group event text.
2. "Message sent with Invisible Ink".
3. `interactiveText` (for example `"Website: <title> (<host>)"` or the app name from
   `balloonBundleIdMap`).
4. "Unsent message" or "Empty message".
5. `"Website: <title≤50> (<domain>)"`.
6. Attachment counts such as `"2 Images & 1 Video"`, or "1 Sticker".
7. Reactions: `"<Name> loved “<text of that part>”"`. This uses the reacted part's
   runs and does a **synchronous DB lookup** of the parent.
8. `sender + fullText`.

`withSender` adds the prefix `"You: "` or `"<Name>: "`.

### 4.2 Grouping and timestamps

- **Timestamp separator:** shown when the gap to the older message is **> 30 min**
  (`timestamp_separator.dart`). The label is "Today h:mm" or the date and time.
  Samsung skin shows one per day.
- **Tail:** shown when there is no newer message, the newer message has a different
  sender, the newer message is a group event or a reply, or it is **more than 1 min
  later** (`Message.showTail`).
- **Sender name (groups):** shown above a message that is not a group event when the
  sender differs from the older message, the older message is a group event, or the
  gap is ≥ 30 min (`message_holder.dart:81`). `sameSender`: both from me, or both
  not from me with the same `handleId`.
- **Sort:** `Message.sort` uses `min(dateCreated, dateDelivered)`.

### 4.3 Delivered and read indicators

`MessagesService._recomputeDeliveredIndicators()` (`messages_service.dart:1071`)
keeps two independent "owners" among outgoing messages:
- **Read:** the from-me message with the newest `dateRead`.
- **Delivered:** the from-me message with the newest `dateDelivered`, falling back to
  `dateCreated` when `isDelivered` is true without a date. Only messages that are not
  read count.

Only those messages show a label (`delivered_indicator.dart`):
- "Read <date>".
- "Delivered[ Quietly] [<date>]". Quietly means `wasDeliveredQuietly &&
  !didNotifyRecipient`. On iOS skin the date is shown only with
  `showDeliveryTimestamps`.
- "Sending…" for 1:1 chats on non-iOS skins, shown for at least 700 ms.
- "Kept <date>" for kept audio.

The chat list status uses `indicatorToShow`: READ > DELIVERED > SENT.

### 4.4 Group events

`Message.buildGroupEventText(name)` (`message.dart`). `isGroupEvent` is
`groupTitle != null || itemType > 0 || groupActionType > 0`.

| itemType | groupActionType | text |
|---|---|---|
| 1 | 0 | `{name} added {other} to the conversation.` |
| 1 | 1 | `{name} removed {other} from the conversation.` |
| 2 | – | `{name} named the conversation "{groupTitle}".` / `{name} removed the name from the conversation.` |
| 3 | 0/null | `{name} left the conversation.` |
| 3 | 1 | `{name} changed the group photo.` |
| 3 | 2 | `{name} removed the group photo.` |
| 4 | 0 | `{name} shared your/their location.` |
| 5 | – | `{name} kept an audio message.` |
| 6 | – | `{name} started a FaceTime call.` |

`name` is "You" when the message is from me. `other` is found by
`Handle.findOne(originalROWID: otherHandle)?.displayName`, falling back to "someone".
A participant event also starts `serverSyncParticipantsAsync()` to re-fetch the
member list. A photo event re-fetches `GET /chat/<guid>/icon`.

### 4.5 Names, contacts and phone numbers

- `Handle.displayName` (`lib/database/io/handle.dart:54`):
  - `urn:biz…` gives "Business".
  - Otherwise, a native contact's nickname, then its displayName, then
    `contactsV2.first.computedDisplayName`.
  - Otherwise, an email address as is, or `formattedAddress ??
    formatPhoneNumber(address)`.
  - `reactionDisplayName` and `shortName` prefer the first name.
- `formatPhoneNumber` (`lib/helpers/types/helpers/contact_helpers.dart`) uses
  libphonenumber (`dlibphonenumber`) in **INTERNATIONAL** format. The default region
  is the device locale's country (`Get.deviceLocale?.countryCode ?? "US"`), or
  `handle.country`.
- Chat title (`Chat.getTitle`, `chat.dart:256`):
  - `displayName`, if set.
  - Otherwise, from the handles: one → that name. 2–4 → `"A, B & C"` using short
    names. More than 4 → `"A, B, C & N others"`.
  - With no handles, it is `chatIdentifier` ("Business Chat" for `urn:biz`).
- `isGroup = style == 43 || handles.length > 1`. The service (iMessage, SMS, RCS)
  comes from the chat GUID prefix.
- **Contact ↔ handle matching**
  (`lib/services/backend/actions/contact_v2_actions.dart`):
  - Emails: compared lowercased and trimmed.
  - Phones: `normalizePhoneNumber` keeps only `[\d+]`. `_getPhoneNumberVariants`
    adds and removes `+`, and **removes 1–3 leading digits as a possible country
    code**. Each variant is checked against a handle map.
  - This is **lax and can match the wrong contact.** For example, two numbers that
    differ only in a 1–3 digit prefix will both match. A safer approach: parse both
    sides to E.164 with libphonenumber, using the user's region as the default,
    and compare exactly, with a national-number fallback only for numbers that have
    no country code.

### 4.6 Avatars

- Order: chat `customAvatarPath` (the group icon from the server) → contact
  `avatarPath` (from `/contact` with `extraProperties=avatar`, saved as
  `contact_avatars/<sanitized id>.jpg`, rewritten only if the SHA-256 changed) →
  initials on a colour gradient.
- Initials: the contact's initials, or the first letter of the first and last words
  of the display name.
- Colour: `handle.color` if set. Otherwise `toColorGradient(address)`: sum the
  address's code units, seed a PRNG with the sum, and pick one of 7 pairs (pink,
  blue, orange, green, yellow, red, purple)
  (`lib/helpers/ui/theme_helpers.dart:363`). The colours only show when
  `colorfulAvatars` is on (iOS skin).
- Group avatars are stacked participant avatars (`maxAvatarsInGroupWidget`, default
  4).
- Bug: `Chat.getIcon` **deletes the saved icon on any failure**, network errors
  included (`chat.dart:851`).

### 4.7 Replies

The quoted original is drawn above the reply (`reply_bubble.dart`): sender, snippet
and thumbnail. Clicking it scrolls to the original. The iOS skin draws thread
connector lines (`reply_line_painter.dart`). A reply-count badge opens a thread
popup that lists every message with the same `threadOriginatorGuid`. The quoted
text follows edits to the original.

### 4.8 Typing indicator

- Incoming: socket `typing-indicator {guid, display}` sets
  `cvc(chat).showTypingIndicator = display` (`action_handler.dart:159`). There is
  **one flag per chat** (no per-person state) and **no client timeout**. A missed
  `display:false`, for example across a reconnect, leaves the bubble on screen until
  the next event. The chat list can show it on tiles too.
- Outgoing: see §6.

**For the Rust client:** port the part builder, the group-event table, the
title, tail and timestamp rules, and the two-tier read/delivered owners as is. Add
a typing timeout of about 60 s, and clear typing when a message from that sender
arrives. Replace the async-in-forEach part building with a synchronous build over
data you already have.

---

## 5. Notifications (desktop/Linux)

Files: `lib/services/backend/notifications/notifications_service.dart`,
`desktop_notification.dart`. Plugin: `flutter_local_notifications` ^22 (Linux uses
`org.freedesktop.Notifications` over D-Bus), initialised with
`LinuxInitializationSettings(defaultActionName: 'Open')`.

**When a notification is shown** (`tryCreateNewMessageNotification`). It runs only
for socket-delivered new messages, never for sync results. The message is skipped
when:
- `isFromMe`.
- **The handle relation is not resolved.** The message is dropped and an error is
  logged. This bug means no notification for a sender whose handle is not known yet.
- It is kept audio (`isKeptAudio`).
- `chat.shouldMuteNotification(message)` (`chat.dart:289`), checked in this order:
  1. `filterUnknownSenders` and the chat is a 1:1 with no contact.
  2. Global `globalTextDetection` keywords. **This overrides mute:** it notifies only
     on a match.
  3. `muteType`: `mute`, `mute_individuals` (comma-separated addresses in
     `muteArgs`), `temporary_mute` (ISO date in `muteArgs`, unmuted on its own when
     the date has passed), or `text_detection`.
  4. A reaction when `notifyReactions` is off.
- The window is focused **and** this chat is open (`isChatActive` = active and
  alive, where alive means window focused on desktop). On desktop, other chats
  always notify. `notifyOnChatList` only applies on mobile.
- `settings.desktopNotifications` is off.

**How it is shown** (`showDesktopNotif` → `_buildAndShowToast`):
- Each chat has a **300 ms debounce**. Pending messages are merged into one toast.
- For a group, each line is prefixed with the sender's first name, and the title is
  `"<group>: <sender>"` when there is only one sender.
- The body is cut by an estimated line budget, and an attribution line "+N earlier
  messages" is added.
- Toast IDs are FNV-1a(chatGuid) times 100 slots, so the toasts for a chat can be
  cancelled as a group.
- On Linux, `getActiveNotifications` is not implemented (`activeIds()` returns
  `[]`). Only toasts from the current run can be cancelled.
- Icon: `FilePathLinuxIcon(avatarPath)`. That is the contact avatar file, or a
  composite PNG written to `temp/` and deleted after 1 min.
- Sound: `ThemeLinuxSound('message-new-instant')`, or a custom file played with
  media_kit (`desktopNotificationSoundPath`, at the set volume).
- `hideTextPreviews` replaces the body with "iMessage".

**Actions** (`settings.actionList` = `["Mark Read", love, like, laugh, …]`,
`selectedActionIndices`, default `[0,1,2]` on Linux):
- Each action button key encodes the data: `dm:<base64url(json{v:1,c:chatGuid,m:messageGuid,a:[actions]})>:action:<i>`.
  The data is in the payload, so a click still works after the app restarts.
- **Mark Read** → `chat.toggleHasUnreadAsync(false)`. This also POSTs
  `/chat/<guid>/read` when private-API read receipts are on. When the toast merges
  several messages, the only button is "Mark N Messages Read".
- **Tapback** buttons only appear with private API, and not for reaction or event
  toasts. They queue an `OutgoingReaction` on `messageGuid` with part 0.
- **Reply** input (`showReplyField`) exists only on Windows, because the Linux spec
  has no inline reply. It queues an `OutgoingMessage`.
- **Click (default action)** → `NavigationSvc.pushAndRemoveUntil(ConversationView(chat))`
  and then `showAndFocusWindow()`. Clicks that arrive before startup is finished
  are queued until `DesktopNotifications.markReady()`.
- Clearing: when a chat is marked read (opened or focused),
  `clearDesktopNotificationsForChat(guid)` cancels that chat's group. At startup,
  `cancelStale` removes leftovers (not on Linux).
- Other kinds: a failed-to-send toast (one at a time), alias deregistered, and
  FaceTime. FaceTime is a critical-urgency toast that never expires, with
  Answer/Decline buttons and the `phone-incoming-call` sound, plus an in-app
  overlay.

**For the Rust client:** use `notify-rust` or `zbus` against
`org.freedesktop.Notifications`, with the `replaces_id` and `actions` capabilities
and the `ActionInvoked` and `NotificationClosed` signals. Use
`x-kde-reply`/`inline-reply` where the server supports them (KDE does). Keep the
per-chat debounce and merge. Notify for messages found by sync too (§2.2). Do not
drop a notification because the handle is unresolved. Fall back to the formatted
address.

---

## 6. Read receipts and typing

- **Mark read** (`ChatsService.setChatHasUnread` → `Chat.toggleHasUnreadAsync`, in
  `chats_service.dart:1315` and `chat.dart:340`):
  - Triggered by opening a chat (`setActiveChatSync` →
    `setChatHasUnread(false, force:true)`), by focusing the window with a chat open
    (`onAppResume`), and by a new incoming message in the active, focused chat
    (`force:true`).
  - It updates the local `hasUnreadMessage` and clears the chat's notifications.
  - If `autoSendReadReceipts ?? settings.privateMarkChatAsRead` (per-chat override,
    else global) **and** `enablePrivateAPI`, it calls **`POST /chat/<guid>/read`**.
    For unread it calls `POST /chat/<guid>/unread`.
  - So every incoming message in a focused chat makes its own `/read` request, with
    no debounce.
  - `privateManualMarkAsRead` shows a manual button instead.
  - Incoming `chat-read-status-changed {chatGuid, read}` sets the local unread flag
    without sending anything back (`privateMark:false`).
  - A new message from me clears unread. A new message from someone else, in a chat
    that is not active, sets it.
- **Typing (outgoing)**, in `conversation_text_field.dart:282`. This needs
  `enablePrivateAPI` and `chat.autoSendTypingIndicators ??
  settings.privateSendTypingIndicators`.
  - On the first keystroke: `POST /chat/<guid>/typing`.
  - Each keystroke restarts a **3 s** timer. When it fires:
    `DELETE /chat/<guid>/typing` (stop).
  - Also stopped when the composer is disposed and on window blur
    (`TypingIndicatorSvc.stopAllTyping()` in `LifecycleService.close`).
  - Sending a message does not explicitly stop typing, but iMessage clears it on
    send.

**For the Rust client:** debounce `/read` (for example at most once every 2 s per
chat, and only while the window is focused and the chat is visible). Copy the
3 s typing-idle rule.

---

## 7. Contacts on desktop

- Source: the **server's** `GET /contact?extraProperties=avatar` (macOS Contacts
  through the server). It needs server version code ≥ 42 (`supportsContactsApi`).
  There is no local address book on Linux.
- Fields used from each entry: `id`, `displayName`/`firstName`/`lastName`,
  `phoneNumbers[].address`, `emails[].address`, `avatar` (base64). The ID falls back
  to the display name.
- When it runs:
  - At the end of a full sync.
  - After **every incremental sync** (`performContactSyncToHandles`). On desktop
    that is every window focus that gets past the 30 s cooldown, and every
    reconnect.
  - Each run downloads **every contact with its base64 avatar**. With a large
    address book, this is the biggest repeated cost in the app.
- Cache: `ContactV2` rows in ObjectBox, linked many-to-many to `Handle`, plus avatar
  files. Matching is in §4.5. Only handles whose name or avatar changed are sent to
  the UI (`notifyHandlesUpdated`).
- Contact upload to the server is Android only.

**For the Rust client:** fetch contacts at startup, then every few hours or on
demand. Fetch avatars lazily or only when they changed. If the server cannot report
changes, keep a hash of the response without avatars and download avatars only
when that hash changes.

---

## 8. Desktop-specific code, Linux packaging, weak spots

### 8.1 Window, tray, lifecycle

`lib/main.dart`:
- Window management uses `window_manager` together with `bitsdojo_window`. The
  minimum size is 300×300. Size and position are saved on every resize and move
  event. On Wayland only the size is restored (`isWaylandSession` checks
  `XDG_SESSION_TYPE` / `WAYLAND_DISPLAY`).
- Title bar: `titleBarStyle` is `custom` (hidden native bar, Flutter-drawn
  controls in `lib/app/wrappers/titlebar_wrapper.dart`) or `native`. It is applied
  early on Linux while the splash screen shows.
- A native GTK splash screen (`linux/splash_screen.cc`) shows startup status through
  the `bluebubbles/splash` MethodChannel.
- Single instance: GApplication uniqueness (`linux/my_application.cc`). A second
  launch calls `gtk_window_present` on the existing window. A `minimized` argument
  starts hidden.
- Close: with `closeToTray` (default **true**), `setPreventClose` hides the window.
  Otherwise Linux calls `exit(0)`. `minimizeToTray` defaults to false.
- Tray (`tray_manager`): left click shows and focuses. The menu has Show/Hide App
  and Close App (`exit(0)` on Linux). The icon path differs by package: Flatpak uses
  the icon name `app.bluebubbles.BlueBubbles`, Snap uses
  `$SNAP/.../assets/icon/icon.png`, and otherwise `assets/icon/icon.png`. Linux has
  **no unread badge** in the tray or taskbar (that is Windows only).
- Focus and blur → `LifecycleService.open()`/`close()`. On desktop, `isAlive` is
  just `windowFocused`.
- Launch at startup: `launch_at_startup`. Under Flatpak it uses the XDG Background
  portal (`lib/database/io/launch_at_startup.dart`).
- Keyboard shortcuts (`main.dart:356`):
  - Ctrl+, settings. Ctrl/Alt+N new chat. Ctrl+F search.
  - Ctrl/Alt+R reply to the most recent message. Ctrl/Alt+G incremental sync.
  - Ctrl+Shift+1..6 tapback the most recent message.
  - Ctrl+↓/Ctrl+Tab next chat. Ctrl+↑/Ctrl+Shift+Tab previous chat.
  - Ctrl+I chat details. Esc back.
  - Linux has its own Ctrl+V handler for pasting images.

### 8.2 Linux packaging

- `snap/snapcraft.yaml`: core24, strict confinement, GNOME extension, amd64 and
  arm64. Plugs: network, network-manager, camera, desktop, wayland, x11, opengl,
  alsa, audio-playback, audio-record. It sets `ALWAYS_USE_PULSEAUDIO=1` and binds
  in WebKit2GTK 4.1.
- `flatpak/`: only the desktop file, metainfo and icons, for app ID
  `app.bluebubbles.BlueBubbles`. The manifest lives on Flathub.
- Update check: GitHub releases, gated by what the store has (Snapcraft API or the
  Flathub appstream API).

### 8.3 Linux workarounds and gaps

- File sharing: "Can't share files on Linux yet!" (`lib/utils/share.dart`).
- Location sending is disabled on Linux (`location_button.dart:20`), and Find My
  skips geolocation.
- App lock is not available (`secure_application` has no Linux support, and it is
  skipped when `!Platform.isLinux`).
- Dynamic and accent colours are fetched after the splash, because
  `DynamicColorPlugin` blocks the GTK thread.
- media_kit audio: on Linux it pauses explicitly after the end, because looping is
  broken. Linux arm64 uses the system ffmpeg.
- Notifications: no `getActiveNotifications`, no inline reply.
- Connectivity checks throw without NetworkManager.

### 8.4 Likely sources of bugs or slowness

1. Every focus runs incremental sync **plus a full contact download with avatars**
   (§7), gated only by a 30 s cooldown. This is the main repeated cost.
2. Global serial send queue (§3.1). One slow attachment upload blocks every chat.
3. Incremental sync misses updates to existing rows (§2.2), so read and delivered
   receipts, edits and unsends that happen while disconnected are lost.
4. Messages caught up by sync never notify (§2.2).
5. The reconnect sync can be skipped by the cooldown (§1.5).
6. Offset paging, plus a "+reactions.length" adjustment, in conversation history
   (§2.4). Also, messages with handles that cannot be resolved are dropped silently.
7. `bulkSyncData` calls `handleBox.getAll()` **inside every page transaction**
   (`sync_actions.dart`), which is O(all handles) per page. The contact sync also
   loads every handle and builds phone-variant maps on every run.
8. `ChatsService.initDbWatchers()` watches the whole chats box
   (`chats_service.dart:278`). Every chat write (each message updates its chat) runs
   `count()` on the main thread. New-chat detection takes `findFirst()` by highest
   ID, so if two chats appear at once, one is missed until a reload.
9. All chats are loaded into memory at startup in batches of 100. Each batch bumps
   `chatListVersion`, which rebuilds the whole list.
10. `MessagesView` sometimes re-keys `SliverAnimatedList` (`_listKey = GlobalKey()`),
    which rebuilds the whole message list. `_recomputeDeliveredIndicators` sorts all
    loaded outgoing messages each time it runs.
11. Synchronous DB reads on the UI thread in hot paths: `getNotificationText` calls
    `Message.findOne` (for each reaction subtitle). There are `Handle.findOne` calls
    in group-event text, and many `File.existsSync` / `readAsBytesSync` calls in UI
    code (about 31 in `lib/app` and `lib/services/ui`).
12. `getBubbleSize` creates a `RenderParagraph` and lays it out on the main thread to
    size bubbles. The result is cached per GUID.
13. The GlobalIsolate shuts down after 5 min idle and restarts on next use. Startup
    re-initialises services and attaches the DB, which adds latency to the first
    send after idle.
14. IncomingMessageHandler runs **5** messages at once. Different GUIDs can finish
    out of order, which affects latest-message and notification order.
15. The 500 ms delay for every own message from another device (§2.3).
16. Accepting any certificate, and the password in the URL (§1.5).
17. Chat icon deleted on a fetch error (§4.6). The async part builder can drop
    attachments (§4.1). The typing indicator can stay on (§4.8).
18. Polling: there is none apart from the 30 s heartbeat, the URL-rediscovery
    backoff, and a subnet scan at most every 2 min when `localhostPort` is set.

---

## 9. Settings a desktop user expects

From `lib/database/global/settings.dart` and the settings panels in
`lib/app/layouts/settings/pages/*`. Defaults are in parentheses.

- **Connection:**
  - Server URL and password.
  - Custom headers.
  - API timeout (30000 ms).
  - Localhost port and "use local IPv6" for LAN detection.
  - Re-run setup or full sync, and a manual incremental sync.
  - "Show incremental sync toast" (off).
  - Server management: restart iMessage or Private API, check for updates, logs,
    statistics.
  - Firebase/OAuth panel.
- **Private API:**
  - Enable (off).
  - Send typing indicators. Send read receipts (`privateMarkChatAsRead`). Manual
    mark-as-read.
  - Subject line.
  - Send text through private API. Send attachments through private API.
  - Up-arrow edits the last sent message.
  - Quick tapback and its type.
- **Notifications:**
  - Desktop notifications (on). Sound file and volume.
  - Notify reactions (on).
  - Filter unknown senders.
  - Global keyword filter.
  - Hide text previews.
  - Toast actions (Mark Read and tapbacks). Show reply field.
  - Per chat: mute, mute individuals, temporary mute, keyword detection.
- **Desktop:**
  - Launch at startup, and start minimized.
  - Minimize to tray (off). Close to tray (on).
  - Title bar style (custom or native).
  - Window effect (Windows only).
  - Use the system accent colour.
  - Spellcheck and its language.
- **Conversation and list:**
  - Send with Return.
  - Show delivery timestamps. 24-hour time.
  - Always show avatars.
  - Unarchive on new message.
  - Scroll to last unread.
  - Cancel queued messages after a failure.
  - Send delay.
  - Replace emoticons with emoji.
  - Link preview policy (contacts only).
  - Auto-download attachments (on). Max concurrent downloads (2).
  - Pinned chats layout. Custom groups and filters. Dense tiles.
  - Status indicators on chats. Highlight selected chat.
- **Theming:**
  - Skin (iOS, Material or Samsung).
  - Light, dark or system mode. Colourful avatars and bubbles.
  - Theme studio. Per-chat themes and wallpapers.
  - Apple emoji font download.
- **Privacy:** redacted mode, which hides contact info, message content and
  attachments, and can generate fake avatars.
- **Misc:** log level and live logging, cache tuning, the storage analyser, the list
  of soft-deleted chats, the handle audit, and backup or restore of settings and
  themes to the server.
