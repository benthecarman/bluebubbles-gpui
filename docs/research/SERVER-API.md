# BlueBubbles Server: client-facing API reference

Source: `third_party/bluebubbles-server` at commit `76d2b2e` (server **v1.9.12**,
`packages/server/package.json`). Shallow clone, so there is no history to diff
against older servers.

In this document, `S/` means `packages/server/src/server/`. Every claim cites
the file it comes from. If a statement is an inference and not read directly
from code, it is marked **(inferred)**.

---

## 0. Quick facts for a Rust client

| Topic | Fact |
|---|---|
| Base URL | `http(s)://<host>:<socket_port>/api/v1/...`. One port serves REST and Socket.IO (`S/api/http/index.ts`). |
| Auth (REST) | Query param `guid`, `password`, or `token` (checked in that order). **No header auth.** (`S/api/http/api/v1/middleware/authMiddleware.ts`) |
| Auth (socket) | Handshake query `password` (or legacy `guid`). (`S/api/http/passwordAuth.ts`) |
| Socket.IO | Server is `socket.io` **^4.7.5**, so Engine.IO **v4** natively, with `allowEIO3: true`. Transports `["websocket","polling"]`. Default namespace `/`. pingInterval 60 s, pingTimeout 120 s, maxHttpBufferSize 100 MB. (`S/api/http/index.ts`) |
| Dates | Every serialized date is **ms since Unix epoch** (JS `Date.getTime()`), or `null` when unset/0. Query params `after`/`before` are also ms epoch. Scheduled-message entity dates are the exception (ISO strings, see 2.13). |
| Envelope | `{ "status": <int>, "message": <string>, "data"?: any, "metadata"?: object, "error"?: { "type": string, "message": string } }` |
| Private API detection | `GET /api/v1/server/info` → `data.private_api && data.helper_connected`. macOS version is `data.os_version`. There is **no** `macos_version` field. |
| Reactions | `associatedMessageType` is a **string**: `"love"`, `"like"`, `"dislike"`, `"laugh"`, `"emphasize"`, `"question"`, the same names with a `-` prefix for removal, `"sticker"`, or the raw number as a string for unmapped types (for example `"2006"`). |
| Sent-message matching | Send with `tempGuid`. The REST response echoes `data.tempGuid`. The socket also gets a `new-message` that carries `tempGuid`, plus a second `new-message` for the same `guid` without it, in either order. Upsert by `guid`. |

---

## 1. Transport, auth, envelope, errors

### 1.1 HTTP server
`S/api/http/index.ts`, `S/api/http/api/v1/httpRoutes.ts`

- Koa app. CORS is on for all origins (`koa-cors`). `koa.proxy = true`.
- HTTPS is used only when proxy service is "Dynamic DNS" and a custom certificate is configured. Otherwise the server speaks plain HTTP. Proxy services (Cloudflare, ngrok, zrok) terminate TLS in front of it.
- Routes are built as `/api/v1/<group prefix>/<route path>` (`HttpRoutes.createRoutes`). `GET /` serves an HTML landing page and needs no auth.
- Add `?pretty` to any request to get pretty-printed JSON (`koa-json` `param: "pretty"`).
- Unknown paths fall through to Koa's default: **404 with a plain-text body `Not Found`**, not JSON. Wrong methods get `koa-router` `allowedMethods` (405/501).

### 1.2 Authentication (REST)
`S/api/http/api/v1/middleware/authMiddleware.ts`, `S/api/http/passwordAuth.ts`, `S/api/http/authThrottle.ts`

- Token: `ctx.request.query.guid ?? .password ?? .token`. The value is trimmed. Arrays (a repeated param) count as missing. The comparison is constant-time.
- Results:
  - missing → **401** `error.message = "Missing server password!"`
  - no password configured on the server → **500** `"Failed to retrieve password from the database"`
  - wrong → **401** (`Unauthorized`)
  - ok → continue
- The request body is parsed **only after** auth passes (`BodyMiddleware`).
- **Brute-force throttle** (per client IP, in memory, cleared on server restart). It counts **distinct** wrong passwords only, so retrying the same stale password never trips it:
  - 3 in 1 s, 10 in 1 min, 50 in 24 h → blocked.
  - When blocked: **429** with `Retry-After: <seconds>` header, `error.type = "Authentication Error"`, message `"Too many failed authentication attempts. Try again later"`.
  - Loopback addresses are never blocked. `X-Forwarded-For` is honoured only from LAN/loopback peers.
  - A correct password clears the address's history.
- There is **no** general rate limiting of authenticated requests.

### 1.3 Ping, server info, versioning
- `GET /api/v1/ping` → `{"status":200,"message":"Ping received!","data":"pong"}` (`S/api/http/api/v1/routers/generalRouter.ts`). This route needs auth.
- The API version is fixed: `HttpRoutes.version = 1`. No other version prefix exists.
- `GET /api/v1/server/info` (`ServerRouter.getInfo` → `GeneralInterface.getServerMetadata`, `S/api/interfaces/generalInterface.ts`; type `ServerMetadataResponse` in `S/types.ts`):

```json
{
  "status": 200, "message": "Success",
  "data": {
    "computer_id": "string",
    "os_version": "14.5",
    "server_version": "1.9.12",
    "private_api": true,
    "helper_connected": true,
    "proxy_service": "Cloudflare",
    "detected_icloud": "user@icloud.com",
    "detected_imessage": "user@icloud.com",
    "macos_time_sync": 0.012,
    "local_ipv4s": ["192.168.1.10"],
    "local_ipv6s": ["fe80::..."]
  }
}
```

  - `private_api` is the `enable_private_api` config flag. `helper_connected` is `!!privateApi.helper`. Both must be true for Private API features.
  - `macos_time_sync` is a clock offset in seconds, or `null`.

### 1.4 Response envelope
`S/api/http/api/v1/responses/{types,success,errors,index}.ts`

- Success: `Success` → HTTP 200 `{ status: 200, message, data?, metadata? }`. `data` and `metadata` are left out when undefined. The default `message` is `"Success"`.
- `NoData` → HTTP **201** `{ status: 201, message: "No Data" }`. Only `POST /facetime/leave/:call_uuid` uses it.
- File responses (downloads, icons) are raw byte streams with `Content-Type` set. They have no envelope.
- Errors (`HTTPError` subclasses, mapped by `ErrorMiddleware`):

| Class | HTTP | `error.type` | default `message` |
|---|---|---|---|
| `BadRequest` | 400 | `"Validation Error"` | "You've made a bad request! Please check your request params & body" |
| `Unauthorized` | 401 | `"Authentication Error"` | "You are not authorized to access this resource" |
| `Forbidden` | 403 | `"Authentication Error"` | ... |
| `NotFound` | 404 | `"Database Error"` | "The requested resource was not found" |
| `TooManyRequests` | 429 | `"Authentication Error"` | ... |
| `ServerError` | 500 | `"Server Error"` | "The server has encountered an error" |
| `IMessageError` | 500 | `"iMessage Error"` | "iMessage has encountered an error" |
| `GatewayTimeout` / timeout middleware | 504 | `"Gateway Timeout"` | "The request timed-out after N ms!" |
| any other thrown error | 500 | `"Server Error"` | "An unhandled error has occurred!" |

  - `error.message` carries the detail. For validation errors it is the **first** failing rule (validatorjs).
  - Send errors (`IMessageError`) often include `data`: a serialized Message whose `error` field holds the Apple error code.
  - Full shape: `{ status, message, error: { type, message }, data? }`.
- `encrypted` (bool) appears only on **socket** ack responses (see 3.4).

### 1.5 Timeouts and body limits
`S/api/http/api/v1/httpRoutes.ts`, `.../middleware/bodyMiddleware.ts`

- Default timeouts: request 60 s, response 5 min. These are set with `req/res.setTimeout`, so they are socket idle timeouts **(inferred)**. A timeout returns 504 JSON.
- Per-route overrides:
  - `/attachment/upload`: request 30 min.
  - `/attachment/:guid/download` and `/live`: response 30 min.
  - `/attachment/:guid/download/force`: response 60 min.
  - `/server/update/install`: response 30 min.
  - `POST /contact`: request 5 min.
  - `/mac/*`: response 30 s.
- `/message/attachment` keeps the **default 5 min response timeout**, but the server-side send awaiter waits up to 20 min for attachments. A slow attachment send can therefore return 504 even though it later succeeds.
- Body limits: JSON 100 MB, text 100 MB, form 1024 MB, multipart file 1 GiB. Bodies are parsed for POST/PUT/PATCH/**DELETE**, so DELETE routes can take a JSON body.

### 1.6 Chat GUID normalisation
`S/api/http/api/v1/middleware/chatGuidMiddleware.ts`, `S/databases/imessage/helpers/chatGuid.ts`, `S/databases/imessage/index.ts#resolveChatGuid`

- Chat GUID format: `<service>;-;<address>` for a DM, or `<service>;+;<chat id>` for a group. Service is `iMessage`, `SMS`, `RCS`, or (macOS **Tahoe 26+**) `any`.
- For `/chat/*` routes the `:guid` param is rewritten to the stored GUID. Any `chatGuid` in the body or query is rewritten on **all** routes. The rule only maps between `any` and a service prefix. It never maps `iMessage` to `SMS`.
- Socket events get the same rewrite for specific keys (`chatGuidParamKeys` in `socketRoutes.ts`).
- **Outgoing events are not rewritten.** On Tahoe, GUIDs in payloads start with `any;`. A client should normalise the GUIDs it caches **(inferred)**.
- `ValidateChatGuid` rejects a bare service name (for example `"iMessage"`) on `/message/attachment`. That value usually means a multipart field was cut at `;`.

---

## 2. REST routes

All routes require auth unless stated otherwise. **PAPI** means the route has `PrivateApiMiddleware`. When the helper is not ready, such a route returns **500** `iMessage Error`, `message: "Please make sure you have completed the setup for the Private API, and your helper is connected!"`. **PAPI\*** means there is no middleware, but the interface calls `checkPrivateApiStatus()`. That fails as a 500 with the route's own error message, for example "Message Edit Error".

The `with` lists are comma-separated strings or arrays. They are compared case-insensitively (`parseWithQuery` in `.../v1/utils.ts`, `arrayHasOne` in `S/utils/CollectionUtils.ts`).

### 2.1 Chats
`S/api/http/api/v1/routers/chatRouter.ts`, `validators/chatValidator.ts`, `S/api/interfaces/chatInterface.ts`

| Method & path | Gate | Params | Response `data` |
|---|---|---|---|
| `POST /chat/query` | none | body: `with` (**array**; `"lastmessage"`/`"last-message"`), `guid` (exact chat GUID filter), `sort` (only `"lastmessage"`), `offset` (≥0, default 0), `limit` (1–1000, default **1000**) | `Chat[]`; `metadata: {count, total, offset, limit}` (raw body values; left out if not sent) |
| `GET /chat/count` | none | query `includeArchived` (default true) | `{ total, breakdown: { "iMessage": n, "SMS": n, ... } }` |
| `GET /chat/:guid` | none | query `with=participants,lastmessage` | `Chat` (+`lastMessage`) |
| `GET /chat/:guid/message` | none | query: `with` (`attachment(s)`, `message.attributedbody`/`message.attributed-body`, `message.message-summary-info`, `message.payload-data`), `sort` (`ASC`\|`DESC`, case-sensitive, default DESC), `before` (ms, ≥1), `after` (ms, ≥0), `offset` (default 0), `limit` (1–1000, default 100) | `Message[]`; `metadata: {offset, limit, total, count}` |
| `POST /chat/new` | none (PAPI\* if `method=private-api`) | body: `addresses` (**required**, string[]), `message`, `method` (`apple-script`\|`private-api`, default apple-script), `service` (`iMessage`\|`SMS`, default iMessage), `tempGuid`, `effectId`, `subject`, `attributedBody` | `Chat` with `participants` and `messages` (each message gets `tempGuid` injected) |
| `PUT /chat/:guid` | PAPI | body `displayName` (string, min 1). Groups only. | `Chat`. Waits ≤30 s for the DB change. |
| `POST /chat/:guid/participant` or `/participant/add` | PAPI | body `address` | `Chat` |
| `DELETE /chat/:guid/participant` or `POST /participant/remove` | PAPI | body `address` | `Chat` |
| `POST /chat/:guid/leave` | PAPI | — | message only |
| `POST /chat/:guid/read` | PAPI | — | message only. Also emits `chat-read-status-changed {chatGuid, read:true}`. |
| `POST /chat/:guid/unread` | PAPI | — | message only. Calls PAPI only on Ventura+. Always emits `{chatGuid, read:false}`. |
| `POST /chat/:guid/typing` | PAPI | — | start typing |
| `DELETE /chat/:guid/typing` | PAPI | — | stop typing. (A send also stops typing automatically.) |
| `GET /chat/:guid/icon` | none | — | image bytes (`Content-Type` from the attachment, fallback `image/jfif`). 404 when there is no icon. |
| `POST /chat/:guid/icon` | PAPI | multipart file field **`icon`** | message only. Big Sur+. |
| `DELETE /chat/:guid/icon` | PAPI | — | message only |
| `DELETE /chat/:guid/:messageGuid` | PAPI | — | deletes a message from the chat (local) |
| `DELETE /chat/:guid` | PAPI | — | deletes the chat. Waits ≤30 s. |
| `GET /chat/:guid/share/contact/status` | PAPI | — | `data: boolean`. Big Sur+. |
| `POST /chat/:guid/share/contact` | PAPI | — | message only |

Chat quirks:

- `/chat/query` **always includes `participants`**. It uses an INNER JOIN on participants, so chats with **zero** handles are left out (`iMessageRepo.getChats`, `S/databases/imessage/index.ts`). Archived chats are included.
- `sort=lastmessage` is applied **inside the returned page only**. The DB page is ordered by `chat.ROWID DESC` before the in-memory sort (`ChatInterface.get`). To get a global most-recent order, fetch everything (default limit 1000) or sort client-side.
- If `with` contains `lastmessage`, sorting defaults to `lastmessage`.
- `lastMessage` is the message with `MAX(message.ROWID)` in the chat. It can be a tapback, a group event, or similar. In `/chat/query` it is serialized with default config: `attributedBody`, `messageSummaryInfo`, and `payloadData` are `null`, `chats` is `[]`. `text` is still extracted from `attributedBody`. A chat with no messages gets `lastMessage: null`.
- `GET /chat/:guid` without `with=participants` still returns `"participants": []`, because the serializer defaults to including them. Do not read an empty list as "no participants". With `with=participants`, a chat that has no handles returns 404.
- `GET /chat/:guid` `lastmessage` uses `getChatLastMessage` (newest by `dateCreated`). `/chat/query` uses max ROWID. They can differ.
- `POST /chat/new` constraints:
  - With `apple-script` on Big Sur+: one address only, and `message` is required.
  - With `private-api`: `message` is required.
  - Addresses must be "valid" (no quotes, backslashes, or control chars; ≤256 chars).
  - With `private-api`, `tempGuid` does **not** create a message-match socket event. It is only echoed into the response.
- Rename/participant/delete operations poll the DB for up to 30 s and throw if nothing changed.

### 2.2 Messages
`S/api/http/api/v1/routers/messageRouter.ts`, `validators/messageValidator.ts`, `S/api/interfaces/messageInterface.ts`

| Method & path | Gate | Params | Response `data` |
|---|---|---|---|
| `POST /message/query` | none | body (below) | `Message[]`; `metadata: {offset, limit, total, count}` |
| `GET /message/count` | none | query `chatGuid`, `after`, `before` (ms), `minRowId`, `maxRowId` | `{ total }` |
| `GET /message/count/updated` | none | same, `after` **required**. Matches date_delivered/read/edited/retracted in range. | `{ total }` |
| `GET /message/count/me` | none | same as count, `is_from_me = 1` | `{ total }` |
| `GET /message/:guid` | none | query `with`: `chats`/`chat`, `chats.participants`, `attachment(s)`, `attributedBody`/`attributed-body`, `messageSummaryInfo`/`message-summary-info`, `payloadData`/`payload-data` | `Message` |
| `POST /message/text` | see 5 | body (below) | `Message` + `tempGuid` |
| `POST /message/attachment` | see 5 | **multipart** (below) | `Message` (**no `tempGuid` injected**) |
| `POST /message/multipart` | PAPI\* | JSON body (below) | `Message` + `tempGuid` |
| `POST /message/react` | PAPI | body `chatGuid`*, `selectedMessageGuid`*, `reaction`* (`love` `like` `dislike` `laugh` `emphasize` `question`, or the same with a `-` prefix), `partIndex` (≥0) | the reaction `Message` |
| `POST /message/:guid/edit` | PAPI\*, Ventura+ | body `editedMessage`*, `backwardsCompatibilityMessage`* (text older devices see), `partIndex` | updated `Message`. Waits ≤30 s for `dateEdited` to advance. |
| `POST /message/:guid/unsend` | PAPI\*, Ventura+ | body `partIndex` | updated `Message`. Waits ≤30 s. |
| `POST /message/:guid/notify` | PAPI, Monterey+ | — | `Message` (notify anyway for Focus-silenced messages; waits for `didNotifyRecipient`) |
| `GET /message/:guid/embedded-media` | PAPI | — | bytes: `image/png`, or `video/quicktime` for Digital Touch. Only for Digital Touch or handwritten messages. |
| `GET/POST /message/schedule`, `GET/PUT/DELETE /message/schedule/:id` | none | see 2.13 | |

`POST /message/query` body (`MessageValidator.queryBodyRules`):

| Field | Type | Notes |
|---|---|---|
| `chatGuid` | string | Inner-joins `chat`. If the chat does not exist, see quirk below. |
| `with` | array | `chat`/`chats`, `chat.participants`/`chats.participants`, `attachment`/`attachments`, `attachment.metadata`/`attachments.metadata`, `attributedbody`/`attributed-body`, `messageSummaryInfo`/`message-summary-info`, `payloadData`/`payload-data` |
| `where` | array of `{statement: string, args: object}` | ≤25 items. Restricted SQL grammar (`S/databases/imessage/helpers/whereClauseValidator.ts`): comparisons, `IS [NOT]`, `[NOT] LIKE/GLOB/IN/BETWEEN`, `COLLATE NOCASE/BINARY/RTRIM`, `AND/OR/NOT`, parens. Column refs must be `<alias>.<column>`. Aliases: `message`, `handle`, `attachment` (only if `with` attachments), `chat` (only if chatGuid or `with` chat), `chandle` (chat participants). DB column names and entity property names are both accepted. Params `:name`, list params `:...name`. Example: `{"statement":"message.text LIKE :term COLLATE NOCASE","args":{"term":"%hi%"}}` |
| `sort` | `ASC`\|`DESC` | default DESC, by `message.date` |
| `after` / `before` | number (ms) | inclusive on `message.date` |
| `offset` | ≥0 | default 0 |
| `limit` | 1–1000 | default 100 |
| `convertAttachments` | bool | HEIC/TIFF → JPEG, CAF → MP3 in the serialized attachment metadata (2.3) |

Message query quirks:

- If `chatGuid` is given and the chat does not exist, the handler `return`s a `Success` object **without calling `.send()`**. Koa then answers **404 with a plain-text `Not Found` body**. Handle that as "no chat".
- **Search on Ventura+ with the Private API enabled.** If exactly one `where` statement contains `message.text`, that statement is replaced by a Spotlight search through the helper. Leading/trailing `%` are stripped, and "contains" matching is token-based. The reason: `message.text` is NULL for outgoing messages on Ventura+.
- `GET /chat/:guid/message` and `/message/query` always join `handle`. Attachments are joined only on request. Without `with=attachments`, `attachments` is `[]`.

`POST /message/text` body (`sendTextRules`):

| Field | Type | Notes |
|---|---|---|
| `chatGuid` | string, required | |
| `message` | string, must be **present** (may be `""`) | AppleScript requires it non-empty. Private API requires `message` or `subject`. The PAPI `send-message` call itself rejects an empty `message` (`throwForNoMissingFields`), so subject-only sends may fail **(inferred)**. |
| `tempGuid` | string | **Required for `apple-script`.** Optional for private-api. Duplicate in-flight `tempGuid` → 400 "Message is already queued to be sent!". |
| `method` | `apple-script` \| `private-api` | default `apple-script`. **Forced to `private-api`** if `effectId`, `subject`, `selectedMessageGuid`, `ddScan`, or `attributedBody` is present. |
| `effectId` | string | `expressiveSendStyleId`, for example `com.apple.MobileSMS.expressivesend.impact` / `.gentle` / `.loud` / `.invisibleink`, `com.apple.messages.effect.CKConfettiEffect` / `CKEchoEffect` / `CKFireworksEffect` / `CKHappyBirthdayEffect` / `CKHeartEffect` / `CKLasersEffect` / `CKSparklesEffect` / `CKSpotlightEffect` (list from `third_party/bluebubbles-app`) |
| `subject` | string | |
| `selectedMessageGuid` | string | reply target (creates a thread reply) |
| `partIndex` | number ≥0 | part of the reply target |
| `ddScan` | bool | data-detector scan (Catalina+) |
| `attributedBody` | object | raw NSAttributedString-like dict passed to the helper |

The send is **synchronous**:

- AppleScript: the server waits for the poller to match the sent row by normalised text and time (2 min timeout).
- Private API: it waits up to 60 s for the GUID the helper returns to appear in the DB.

On success: `200 {message:"Message sent!", data: Message & {tempGuid}}`. If the row has `error != 0`: **500** `IMessageError` with `data` = the message. On timeout: 500 `"Message Send Error"`, `error.message` like "Message send timeout".

`POST /message/attachment` (multipart/form-data): fields `chatGuid`*, `name`* (plain file name, no path), `attachment`* (the file), `tempGuid` (**required for apple-script**), `method`, `isAudioMessage` (`"true"` → voice memo; mp3 is converted to caf), `effectId`, `subject`, `selectedMessageGuid`, `partIndex`. The same "imply private-api" rule applies. For apple-script, the sent row is matched by **attachment transfer name without extension** (20 min timeout). Use unique file names.

`POST /message/multipart` (JSON, PAPI\*):

```json
{
  "chatGuid": "iMessage;+;chat123",
  "tempGuid": "temp-uuid",
  "parts": [
    { "partIndex": 0, "text": "Hey " },
    { "partIndex": 1, "text": "@Bob", "mention": "bob@example.com" },
    { "partIndex": 2, "attachment": "<uuid>/<file>.jpg", "name": "photo.jpg" }
  ],
  "effectId": null, "subject": null, "selectedMessageGuid": null, "partIndex": 0, "ddScan": false
}
```

- Every part needs a numeric `partIndex` and either `text` or `attachment`.
- An `attachment` must be the `data.path` that `POST /attachment/upload` returned. An attachment part also needs `name`.
- A `mention` part needs `text`.
- `tempGuid` is echoed in the response only. **No** tempGuid-tagged socket `new-message` is sent for multipart (no `MessagePromise` is created).

### 2.3 Attachments
`S/api/http/api/v1/routers/attachmentRouter.ts`, `validators/attachmentValidator.ts`, `S/api/interfaces/attachmentInterface.ts`, `S/databases/imessage/helpers/utils.ts` (`convertImage`, `convertAudio`)

| Method & path | Gate | Params | Response |
|---|---|---|---|
| `GET /attachment/:guid` | none | — | `Attachment` JSON (with metadata; converted, see below) |
| `GET /attachment/count` | none | — | `{ total }` |
| `GET /attachment/:guid/download` | none (PAPI only for auto-fetching purged files) | query `original` (bool), `force` (bool, **default true**), `width`, `height` (≥1, capped 4096), `quality` (`good`\|`better`\|`best`) | file bytes |
| `GET /attachment/:guid/download/force` | PAPI | same | asks the helper to download a purged/iCloud attachment, waits ≤10 min for `transferState == 5`, then streams |
| `GET /attachment/:guid/live` | none | — | `video/quicktime` of the Live Photo companion `.mov`. 404 if none. |
| `GET /attachment/:guid/blurhash` | none | `width` (default 480), `height` (default 320), `quality` | `data: "<blurhash string>"`. 404 if not an image. |
| `POST /attachment/upload` | PAPI | multipart field **`attachment`** (plain file name) | `{ "path": "<uuid>/<filename>" }` for multipart sends |

Download behaviour:

- Booleans are truthy for `"1"`, `"true"`, `"True"` (`isTruthyBool`).
- GUID lookup tolerates prefixes such as `p:/` or `at_x_`. It matches on the last 36 chars against `guid` or `original_guid`.
- If the file is missing on disk, and `force` (default true) is set, and the Private API is enabled, the server first tries a helper download (up to 10 min). If the file is still missing: **500** "Attachment does not exist in disk!".
- Unless `original=true`, the server **converts**:
  - HEIC/HEIF/TIFF → JPEG (`image/jpeg`)
  - CAF (`com.apple.coreaudio-format`) → MP3 (`audio/mp3`)
  - Converted files are cached on the server.
- Resizing applies when `width`/`height`/`quality` is given, for non-GIF images. The output is **always PNG** (`image/png`) and is cached per size.
- `Content-Type` is the (possibly converted) MIME type. The filename is not sent in any `Content-Disposition` header **(inferred, `FileStream` sets only Content-Type)**.

**Serialization also converts.** `AttachmentSerializer` runs with `convert: true` by default (`S/api/serializers/constants.ts`). An attachment JSON in a message, including socket events, may therefore show `mimeType: "image/jpeg"` and a `.jpeg` `transferName` for a HEIC original. It also computes metadata (`height`, `width`, `metadata`) from the file. So the JSON describes what `download` (without `original`) returns. Use `original=true` to get the real file.

### 2.4 Handles
`S/api/http/api/v1/routers/handleRouter.ts`, `validators/handleValidator.ts`, `S/api/interfaces/handleInterface.ts`

| Method & path | Gate | Params | Response |
|---|---|---|---|
| `POST /handle/query` | none | body `address` (substring), `with` (array: `chat(s)`, `chat(s).participants`), `offset`, `limit` (1–1000, default 100) | `Handle[]` (+`chats`); `metadata {total, offset, limit, count}` |
| `GET /handle/count` | none | — | `{ total }` |
| `GET /handle/:guid` | none | `:guid` is an **address**. SQL `handle.id LIKE %<address without +>`, first row wins. | `Handle` |
| `GET /handle/:guid/focus` | PAPI\*, Monterey+ | — | `{ status: "silenced" \| "none" \| "unknown" }` |
| `GET /handle/availability/imessage` | PAPI | query `address`* | `{ available: bool }` |
| `GET /handle/availability/facetime` | PAPI\* | query `address`* | `{ available: bool }` |

One address can have several handle rows, one per service (iMessage and SMS). `handleId` on a message refers to a handle `originalROWID`.

### 2.5 Contacts
`S/api/http/api/v1/routers/contactRouter.ts`, `S/api/interfaces/contactInterface.ts`

- `GET /contact?extraProperties=avatar` (or `contactImage`, `contactThumbnailImage`) → all contacts. Server-DB contacts come first, then macOS Contacts.
- `POST /contact/query` body `{ "addresses": ["+15551234567", ...], "extraProperties": ["avatar"] }` → the matching contacts only. An empty `addresses` returns all.
- `POST /contact` body: one object or an array of `{firstName, lastName, displayName, phoneNumbers: string[], emails: string[], avatar}`. At least `firstName` or `displayName` is required. Response `data` = created contacts. `metadata.errors` = `[{entry, error}]`.
- Contact shape (`mapContacts`):

```json
{ "phoneNumbers": [{"address":"+15551234567","id":"..."}],
  "emails": [{"address":"a@b.com","id":"..."}],
  "firstName":"A","lastName":"B","displayName":"A B","nickname":null,
  "birthday": "...", "avatar": "<base64 or empty string>",
  "sourceType": "api" | "db", "id": "..." }
```

- vCard is socket-only: `get-vcf`, `save-vcf {vcf}`, `get-contacts-from-vcf`. See 3.5.

### 2.6 FaceTime (PAPI group, Monterey+)
`S/api/http/api/v1/routers/facetimeRouter.ts`, `S/api/interfaces/facetimeInterface.ts`

- `POST /facetime/session` → `{ link }` (new FaceTime link).
- `POST /facetime/answer/:call_uuid` → `{ link }` (answers the call, returns a join link).
- `POST /facetime/leave/:call_uuid` → **201** NoData.

### 2.7 Find My
`S/api/http/api/v1/routers/findmyRouter.ts`, `S/api/interfaces/findMyInterface.ts`, `S/api/lib/findmy/types.ts`

- `GET /icloud/findmy/devices` → `FindMyDevice[]`, or `null` on Sequoia+. The data is read from Find My cache files.
- `POST /icloud/findmy/devices/refresh` → the same, after opening the Find My app (about 23 s).
- `GET /icloud/findmy/friends` (PAPI) → `FindMyLocationItem[]` from the in-memory cache:

```json
{ "handle": "a@b.com", "coordinates": [lat, lon], "long_address": "...", "short_address": "...",
  "subtitle": "...", "title": "...", "last_updated": 1700000000000,
  "is_locating_in_progress": 0, "status": "legacy" | "live" | "shallow" }
```

- `POST /icloud/findmy/friends/refresh` → the same. It uses the PAPI on Big Sur–Ventura only.

### 2.8 iCloud (PAPI)
`S/api/http/api/v1/routers/icloudRouter.ts`, `S/api/interfaces/iCloudInterface.ts`

- `GET /icloud/account` → the helper's account info. Opaque dict; contains `vetted_aliases: [{Alias, ...}]`.
- `POST /icloud/account/alias` body `{alias}` → changes the active iMessage alias. The alias must be in `vetted_aliases`.
- `GET /icloud/contact?address=` → the contact card. `avatar` is base64.

### 2.9 macOS
`S/api/http/api/v1/routers/macosRouter.ts`

- `POST /mac/lock` → locks the Mac, 1 s after responding.
- `POST /mac/imessage/restart` → restarts Messages.app.

### 2.10 Server admin (brief)
`S/api/http/api/v1/routers/serverRouter.ts`, `S/api/interfaces/serverInterface.ts`

- `GET /server/logs?count=N` (default 100, capped) → `data`: the log text as one string.
- `GET /server/restart/soft` (restarts services), `GET /server/restart/hard` (relaunches the app). Both answer first and act 1 s later.
- `GET /server/update/check` → `{ available, current, metadata: {version, release_date, release_name, release_notes} | null }`.
- `POST /server/update/install?wait=true` → 400 `NO_UPDATE_AVAILABLE` or `UPDATE_REQUIRES_MANUAL_INSTALL`.
- `GET /server/statistics/totals?only=handle,message,chat,attachment` → `{handles, messages, chats, attachments}`.
- `GET /server/statistics/media?only=image,video,location` → `{images, videos, locations}`.
- `GET /server/statistics/media/chat` → `[{chatGuid, groupName, totals:{images,videos,locations}}]`.
- `GET /server/alert`, `POST /server/alert/read` body `{ids: number[]}`.

### 2.11 FCM (brief; not needed for a desktop client)
`S/api/http/api/v1/routers/fcmRouter.ts`

- `POST /fcm/device` body `{name, identifier}` registers a push token.
- `GET /fcm/client` returns the server's `google-services.json`, patched with `oauth_client`. 404 if not configured.

### 2.12 Backups, webhooks (brief)
- `GET /backup/theme[?name=]`, `POST /backup/theme {name (3–50 chars), data: object}`, `DELETE /backup/theme {name}`. `/backup/settings` is identical (`themeRouter.ts`, `settingsRouter.ts`).
- `GET /webhook[?url=&id=]`, `POST /webhook {url, events: string[], options?}`, `DELETE /webhook/:id`. Event names are in `S/api/http/constants.ts`.

### 2.13 Scheduled messages (brief)
`S/api/http/api/v1/routers/scheduledMessageRouter.ts`, `validators/scheduledMessageValidator.ts`

- Create/update body:
  - `type: "send-message"`
  - `payload: {chatGuid, message, method}`
  - `scheduledFor`: ms epoch, must be in the future
  - `schedule: {type: "once"|"recurring", intervalType?: hourly|daily|weekly|monthly|yearly, interval?: number≥1}`
- Entity fields: `id, type, payload, scheduledFor, schedule, status ("pending"|"in-progress"|"complete"|"error"), error, sentAt, created`. These dates are JS `Date` objects, so they serialize as **ISO strings**.

---

## 3. Socket.IO

### 3.1 Connection
`S/api/http/index.ts`, `S/api/http/passwordAuth.ts`

- URL: the same origin as REST, default path `/socket.io/`, namespace `/`. Example: `wss://host/socket.io/?EIO=4&transport=websocket&password=<pw>`.
- **Protocol:** `socket.io` server ^4.7.5, so Engine.IO 4 / Socket.IO protocol 5. `allowEIO3: true`, so EIO=3 clients (socket.io v2) also work. A Rust client using `rust_socketio` (EIO4) matches **(inferred)**.
- Transports: `websocket` and `polling`. Upgrade timeout 30 s. pingInterval 60 s, pingTimeout 120 s.
- Auth: handshake query `password` (fallback `guid`).
  - The server runs `decodeURI()` on the value **after** the query is already URL-decoded. A password with `%XX` sequences must therefore be percent-encoded twice. Otherwise avoid `%` in passwords.
  - It is checked in `allowRequest`, so a bad password rejects the Engine.IO handshake with HTTP 403 and message "Authentication failed". A blocked IP gets "Too many failed authentication attempts".
  - It is checked again on `connection`. If the server password changed in between, the socket is disconnected.
  - When the server password changes, every client is kicked (`Server.handleConfigUpdate` → `kickClients`).
- No auth payload (`auth` object) is used. Use the query string only.

### 3.2 Server → client events (broadcast to all sockets)
Event names come from `S/events.ts`. Emitters: `S/index.ts` (`emitMessage`, `handleNewMessage`, `handleUpdatedMessage`, the group-event listeners, `emitMessageMatch`, `emitMessageError`), `S/api/privateApi/eventHandlers/*`, `S/services/*`.

"Full Message" means `MessageSerializer` with `isForNotification:false`, `parseAttributedBody/parseMessageSummary/parsePayloadData: true`, `includeChats: true`, and `loadChatParticipants: false`. The chats inside have **no `participants`**. "Slim Message" means `isForNotification:true`: no non-essential fields (section 4.1), `attributedBody`/`messageSummaryInfo`/`payloadData` = `null`, and slim chats/attachments/handle.

| Event | Payload | Source / notes |
|---|---|---|
| `new-message` | Full Message | Poller found a new row (`handleNewMessage`). Also see the next row. |
| `new-message` (with `tempGuid`) | **Slim** Message + `"tempGuid"` | `emitMessageMatch`. Sent when a pending send (`MessagePromise`) matches a DB row. Fires for apple-script text/attachment sends and private-api text/attachment sends that carry a tempGuid. **Same `guid` as the plain `new-message`; order is racy.** |
| `updated-message` | Full Message | Changed `dateCreated`, `dateDelivered`, `isDelivered`, `dateRead`, `dateEdited`, `dateRetracted`, `didNotifyRecipient`, or `hasUnsentParts`. Covers delivered, read, edit, unsend, notify-anyway. |
| `message-send-error` | Slim Message (+`tempGuid` if it matched a pending send) | An outgoing row got `error > 0` (`MessagePoller.handlePreviouslyUnsent`, `MessagePromise.reject`). |
| `typing-indicator` | `{ "display": bool, "guid": "<chat guid>" }` | PAPI helper. **DMs only**: `;+;` group GUIDs are dropped. De-duplicated: re-emitted only when the value changes or after 5 s (`PrivateApiTypingEventHandler.ts`). |
| `chat-read-status-changed` | `{ "chatGuid": string, "read": bool }` | `read:true` from `ChatUpdatePoller` when `chat.last_read_message_timestamp` advances, which includes when the Mac or another device reads the chat. Also from REST read/unread. **The socket request `toggle-chat-read-status` re-broadcasts it as `{chatGuid, status}` (key `status`, not `read`).** |
| `group-name-change` | Message with `loadChatParticipants:true` (not slim, attributedBody null) | itemType 2. New name in `groupTitle`. |
| `participant-added` | same | itemType 1, groupActionType 0. `otherHandle` = the added handle ROWID. |
| `participant-removed` | same | itemType 1, groupActionType 1 |
| `participant-left` | same | itemType 3, groupActionType 0. `handle` = the person who left. |
| `group-icon-changed` | same | itemType 3, groupActionType 1 |
| `group-icon-removed` | same | itemType 3, groupActionType 2 |
| `incoming-facetime` | **a JSON-encoded string**: `"{\"caller\":\"+1555...\",\"timestamp\":1700000000000}"` | Only when config `facetime_calling` is off (`PrivateApiFaceTimeStatusHandler.handleStandard`). Parse the string again. |
| `ft-call-status-changed` | `{ uuid, status: "unknown"\|"answered"\|"outgoing"\|"incoming"\|"disconnected", status_id: 0\|1\|3\|4\|6, ended_error, ended_reason, address, handle: Handle\|null, image_url, is_outgoing, is_audio, is_video }` | Only when `facetime_calling` is on. Not emitted for outgoing or answered calls. |
| `imessage-aliases-removed` | `{ "aliases": string[] }` | An iMessage address was deregistered. |
| `new-findmy-location` | `FindMyLocationItem` (2.7) | One event per changed friend, spaced 250 ms apart |
| `hello-world` | `null` | Emitted once when the HTTP server starts listening, so only sockets that reconnect very fast see it |
| `server-update` | `"<version string>"` | Update available |
| `server-update-downloading`, `server-update-installing` | `null` | |
| `new-server` | `"<new server URL>"` | The proxy/public URL changed. The client should reconnect there. |
| `scheduled-message-created` / `-updated` / `-sent` / `-error` | ScheduledMessage entity | `-deleted` carries a list |
| `exception` | error envelope (`createServerErrorResponse`) | A socket handler threw and the client gave no ack |
| `<channel>` (`chats`, `messages`, `error`, ...) | response envelope | Only when a socket request was sent **without** an ack callback (3.4) |

Listed in `S/events.ts` but **never emitted** in this version: `settings-backup-*`, `theme-backup-*`.

### 3.3 Group-change detection
`S/databases/imessage/pollers/MessagePoller.ts#handleGroupChanges`. A row is a group event when `text` is empty and `itemType ∈ {1,2,3}`. Mapping:

- `(1,0)` added, `(1,1)` removed
- `2` rename
- `(3,0)` left, `(3,1)` icon changed, `(3,2)` icon removed

Each group event is emitted **once**, as the specific event. These rows can **also** come through `new-message`, because the main loop processes every entry **(inferred from code: `entries` feed both `handleGroupChanges` and the new/updated loop)**. Treat them as system messages in the timeline, and key off itemType/groupActionType.

### 3.4 Client → server socket requests
`S/api/http/api/v1/socketRoutes.ts`. Call `emit(event, params, ack)`. The ack receives the standard envelope plus `"encrypted": false`.

- If no ack function is passed, the server emits the response on the channel named in each handler (`"chats"`, `"error"`, ...).
- `encrypt_coms` would AES-encrypt `data` with CryptoJS using the password. The server **forces it off at startup** (`S/index.ts`, "temporary fix until the android client supports it again"), so ignore it.

| Event | Params | Ack data | Notes / REST equivalent |
|---|---|---|---|
| `get-server-metadata` | — | ServerMetadata | `/server/info` |
| `get-server-config` | — | config dict (secrets stripped) | admin |
| `get-logs` | `{count}` | string | `/server/logs` |
| `get-chats` | `{withLastMessage=false, withParticipants=true, withArchived=**false**, limit, offset, sort:"lastmessage"}` | Chat[] | `/chat/query`. Skips `urn:` GUIDs. Excludes archived by default. |
| `get-chat` | `{chatGuid, withParticipants=true}` | Chat | `/chat/:guid` |
| `get-chat-messages` | `{identifier, offset, limit=100, after, before, withChats, withAttachments=true, sort="DESC", where, withChatParticipants}` | Message[] | **marked "TODO: DEPRECATE"**. Use `/chat/:guid/message`. |
| `get-messages` | `{after or limit required, chatGuid, offset, before, withChats=true, withAttachments=true, sort="ASC", where, withChatParticipants}` | Message[] | `/message/query`. **Default sort ASC.** |
| `get-attachment` | `{identifier, withMessages}` | Attachment with **base64 `data`** | `/attachment/:guid/download` |
| `get-attachment-chunk` | `{identifier, start=0, chunkSize=1024, compress=false}` | base64 string (zlib deflate if compress); `status 200 message "No Data"` at EOF | legacy |
| `get-last-chat-message` | `{identifier}` | Message | |
| `get-participants` | `{identifier}` | Handle[] | |
| `send-message` | `{guid: chatGuid, message, tempGuid, attachment (base64), attachmentName, attachmentGuid}` | Message (apple-script only) | `/message/text` and `/message/attachment` |
| `send-message-chunk` | `{guid, tempGuid, message, attachmentGuid, attachmentChunkStart, attachmentData (base64), attachmentName, hasMore}` | null (queued) | legacy chunked upload |
| `start-chat` | `{participants: string[], service="iMessage", message, tempGuid}` | Chat | `/chat/new` |
| `rename-group` | `{identifier, newName}` | Chat | `PUT /chat/:guid` |
| `add-participant` / `remove-participant` | `{identifier, address}` | Chat | REST participant routes (the socket ones use AppleScript, which is unreliable on modern macOS) |
| `send-reaction` | `{chatGuid, tempGuid\|messageGuid, messageText, actionMessageGuid, actionMessageText, tapback}` | Message | `/message/react` |
| `toggle-chat-read-status` | `{chatGuid, status}` | (no ack) | broadcasts `chat-read-status-changed {chatGuid, status}` |
| `open-chat` | `{chatGuid}` | (no ack) | queues "open chat" on the Mac |
| `started-typing` / `stopped-typing` | `{chatGuid}` | null | `POST/DELETE /chat/:guid/typing` (PAPI) |
| `mark-chat-read` | `{chatGuid}` | null | `POST /chat/:guid/read` (PAPI) |
| `update-typing-status` | `{chatGuid}` | null | asks the helper for typing status |
| `add-fcm-device` | `{deviceName, deviceId}` | null | `/fcm/device` |
| `get-fcm-client` | — | google-services | `/fcm/client` |
| `get-vcf` / `save-vcf {vcf}` / `get-contacts-from-vcf` | | string | vCard only exists on the socket |
| `change-proxy-service` | `{service, address?}` | null | admin |
| `restart-messages-app`, `restart-private-api`, `check-for-server-update` | — | | admin |

The official clients have moved most data access to REST. A new client should use REST for every request and the socket **only for receiving events**. The socket send/reaction paths are older and AppleScript-only (except `send-reaction`).

---

## 4. Data model

The JSON field names come from serializers (`S/api/serializers/*.ts`) and the types in `S/types.ts`. Entity columns are in `S/databases/imessage/entity/*.ts`. Booleans are real JSON booleans (`BooleanTransformer`). Dates are ms epoch or `null` (`MessagesDateTransformer` → `getDateUsing2001`; DB value 0 → null).

### 4.1 Message (`MessageSerializer.convert`)

Always present:

| Field | Type | Notes |
|---|---|---|
| `originalROWID` | int | chat.db ROWID. It increases monotonically, so it is useful as a sync cursor (`/message/count?minRowId=`). |
| `guid` | string | stable message id |
| `tempGuid` | string? | only on send responses and the tempGuid-tagged `new-message` / `message-send-error` |
| `text` | string \| null | `message.text`, or the first `attributedBody[].string` when text is empty (Ventura+ outgoing rows have NULL `text`). **Sanitized**: U+FFFC (attachment placeholder) characters removed and trimmed. |
| `attributedBody` | `AttributedString[]` \| null | null unless requested (see 4.6) |
| `handle` | Handle \| null | sender. null for from-me messages in some cases (handleId 0). |
| `handleId` | int | handle ROWID. 0 = me or none. |
| `otherHandle` | int | for group events: the target participant's handle ROWID |
| `attachments` | Attachment[] | `[]` unless joined (REST `with=attachments`; socket events always include them) |
| `subject` | string \| null | |
| `error` | int | 0 = ok. Non-zero is Apple's send error code. A comment says `4` = timeout (`S/index.ts`). Other values are Apple-defined. |
| `dateCreated` | ms | |
| `dateRead` | ms \| null | for outgoing DMs, set when the recipient reads |
| `dateDelivered` | ms \| null | |
| `isDelivered` | bool | |
| `isFromMe` | bool | |
| `hasDdResults` | bool | data detectors |
| `isArchived` | bool | |
| `itemType` | int | 0 normal, 1 participant add/remove, 2 rename, 3 left/icon (see 3.3). Other values (for example 4 or 5) are Apple-defined. |
| `groupTitle` | string \| null | the new name for itemType 2 |
| `groupActionType` | int | see 3.3 |
| `balloonBundleId` | string \| null | iMessage app/extension: `com.apple.messages.URLBalloonProvider` (link preview), `com.apple.DigitalTouchBalloonProvider`, `com.apple.Handwriting.HandwritingProvider`, ... |
| `associatedMessageGuid` | string \| null | the target of a tapback/sticker. Format `p:<part>/<guid>` or `bp:<guid>`. Strip it: `replace("bp:","").split("/").last` (as `third_party/bluebubbles-app` does). |
| `associatedMessageType` | string \| null | see 4.2 |
| `expressiveSendStyleId` | string \| null | bubble/screen effect id |
| `threadOriginatorGuid` | string \| null | reply: the GUID of the message being replied to |
| `hasPayloadData` | bool | rich link / app payload exists |
| `chats` | Chat[] | present when `includeChats` (default true). Slim chats without participants. Is `[]` when the query did not join chats (for example `/message/query` without `chatGuid` or `with=chats`, or a chat's `lastMessage`). |

Added when **not** `isForNotification` (REST responses and the full socket `new-message`/`updated-message`):

`country`, `isDelayed`, `isAutoReply`, `isSystemMessage`, `isServiceMessage`, `isForward`, `threadOriginatorPart` (string, for example `"0:0:12"` = part:start:length, Big Sur+), `isCorrupt`, `datePlayed` (ms), `cacheRoomnames` (string, the group chat identifier), `isSpam`, `isExpired` (**actually `is_expirable`**), `timeExpressiveSendPlayed` (ms), `isAudioMessage`, `replyToGuid`, `shareStatus`, `shareDirection`. On Monterey+ also `wasDeliveredQuietly`, `didNotifyRecipient`.

macOS-version-dependent (added whether slim or not):

- High Sierra+: `messageSummaryInfo` (array \| null), `payloadData` (array \| null). Both null unless requested.
- Ventura+: `dateEdited` (ms \| null), `dateRetracted` (ms \| null), `partCount` (int \| null). **These fields are absent on pre-Ventura servers.**

Edit and unsend semantics (`S/databases/imessage/entity/Message.ts`):

- Edited: `dateEdited` set. `text` is the new text. The edit history is in `messageSummaryInfo` (Apple plist; opaque).
- Unsent: `dateEdited` set and `messageSummaryInfo[0].retractedParts` is a non-empty int[]. Fully unsent when also `partCount == 0`. `dateRetracted` is rarely set by Apple.

Request `message-summary-info` to detect unsends from REST. Socket updates always include it.

### 4.2 Tapbacks, stickers, replies
`S/databases/transformers/MessageTypeTransformer.ts`, `S/api/apple/mappings.ts`

| DB `associated_message_type` | JSON `associatedMessageType` |
|---|---|
| 0 | `null` |
| 1000 | `"sticker"` |
| 2000 / 3000 | `"love"` / `"-love"` |
| 2001 / 3001 | `"like"` / `"-like"` |
| 2002 / 3002 | `"dislike"` / `"-dislike"` |
| 2003 / 3003 | `"laugh"` / `"-laugh"` |
| 2004 / 3004 | `"emphasize"` / `"-emphasize"` |
| 2005 / 3005 | `"question"` / `"-question"` |
| anything else | the number as a string, for example `"2006"` / `"3006"` (macOS 15 emoji tapback), `"2007"` (sticker tapback) |

- The server does **not** expose the reaction emoji (`associated_message_emoji` is not mapped). For `"2006"`, derive the emoji from `text` (for example `Reacted 😂 to “...”`) **(inferred)**.
- Sending supports only the six classic tapbacks and their removals (`MessageInterface.possibleReactions`).
- A reaction row has `text` like `Loved “hello”` and `associatedMessageGuid = "p:0/<target>"`.
- Replies: `threadOriginatorGuid` (+ `threadOriginatorPart`). Send a reply with `selectedMessageGuid` + `partIndex`.

### 4.3 Chat (`ChatSerializer.convert`)

| Field | Type | Notes |
|---|---|---|
| `originalROWID` | int | |
| `guid` | string | `iMessage;-;+15551234567`, `SMS;-;+1555...`, `iMessage;+;chat123456789`, `any;-;...` (Tahoe) |
| `style` | int | **43 = group, 45 = DM** (`Chat.isGroup` → `style === 43`) |
| `chatIdentifier` | string | address for a DM, `chatNNN` for a group |
| `isArchived` | bool | |
| `displayName` | string \| null | group name. Empty string or null when unnamed. |
| `participants` | Handle[] | included by default (may be `[]` if not joined). Does **not** include yourself. |
| `messages` | Message[] | only for `/chat/new` |
| `lastMessage` | Message \| null | only for `with=lastmessage` |
| `isFiltered`, `groupId`, `properties` (array from plist; e.g. contains `groupPhotoGuid`), `lastAddressedHandle` | | left out in slim (notification) form |

Service: the GUID prefix gives the service. Handles also carry `service`. `GET /chat/count` gives a breakdown by `serviceName`. The same contact can have both an `iMessage;-;X` and an `SMS;-;X` chat. These are distinct, and the server never merges them.

### 4.4 Handle (`HandleSerializer.convert`)

```json
{ "originalROWID": 12, "address": "+15551234567", "service": "iMessage",
  "uncanonicalizedId": "(555) 123-4567", "country": "us" }
```

`uncanonicalizedId` and `country` are left out in slim form. Optional `chats` / `messages` appear only on handle query.

### 4.5 Attachment (`AttachmentSerializer.convert`)

| Field | Type | Notes |
|---|---|---|
| `originalROWID` | int | |
| `guid` | string | |
| `uti` | string | for example `public.jpeg`, `public.heic`, `com.apple.coreaudio-format` |
| `mimeType` | string \| null | after conversion (see 2.3) |
| `transferName` | string | file name (after conversion) |
| `totalBytes` | int | of the original |
| `transferState` | int | **5 = finished** (`AttachmentInterface.forceDownload`). Other values are Apple-defined. |
| `isOutgoing`, `hideAttachment`, `isSticker` | bool | slim form leaves these out |
| `originalGuid` | string | |
| `hasLivePhoto` | bool | a `.mov` sibling exists → `/attachment/:guid/live` |
| `height`, `width` | int | 0 when unknown. Present when `loadMetadata` (default true). |
| `metadata` | object \| null | image/audio/video metadata from the file |
| `data` | base64 string | only for the socket `get-attachment` |
| `messages` | string[] | only with `includeMessageGuids` |

### 4.6 attributedBody / messageSummaryInfo / payloadData
`S/databases/imessage/helpers/utils.ts#convertAttributedBody` (node-typedstream). The shape matches `third_party/bluebubbles-app/lib/database/global/attributed_body.dart`.

```json
"attributedBody": [{
  "string": "Hey @Bob look \ufffc",
  "runs": [
    { "range": [0, 4],  "attributes": { "__kIMMessagePartAttributeName": 0 } },
    { "range": [4, 4],  "attributes": { "__kIMMessagePartAttributeName": 1, "__kIMMentionConfirmedMention": "bob@example.com" } },
    { "range": [13, 1], "attributes": { "__kIMMessagePartAttributeName": 2, "__kIMFileTransferGUIDAttributeName": "<attachment guid>" } }
  ]
}]
```

- `range` is `[start, length]` in UTF-16 units **(inferred: NSString indexing)**.
- Each part (`__kIMMessagePartAttributeName`) is what `partIndex` refers to for tapbacks, replies, edits, and unsends.
- Audio messages may carry `IMAudioTranscription`.
- `messageSummaryInfo` and `payloadData` are decoded plists. They are opaque except for `retractedParts` (4.1).

### 4.7 Example: full `new-message` socket payload (incoming DM with an image)

```json
{
  "originalROWID": 401234,
  "guid": "6A1F2C3D-1111-2222-3333-444455556666",
  "text": "look at this",
  "attributedBody": [{ "string": "look at this\ufffc", "runs": [ ... ] }],
  "handle": { "originalROWID": 12, "address": "+15551234567", "service": "iMessage",
              "uncanonicalizedId": null, "country": "us" },
  "handleId": 12, "otherHandle": 0,
  "attachments": [{
    "originalROWID": 9001, "guid": "at_0_ABCDEF...", "uti": "public.heic",
    "mimeType": "image/jpeg", "transferName": "IMG_0001.jpeg", "totalBytes": 1832211,
    "transferState": 5, "isOutgoing": false, "hideAttachment": false, "isSticker": false,
    "originalGuid": "ABCDEF...", "hasLivePhoto": true, "height": 4032, "width": 3024,
    "metadata": { "...": "..." }
  }],
  "subject": null, "error": 0,
  "dateCreated": 1761600000000, "dateRead": null, "dateDelivered": null,
  "isDelivered": true, "isFromMe": false, "hasDdResults": false, "isArchived": false,
  "itemType": 0, "groupTitle": null, "groupActionType": 0,
  "balloonBundleId": null, "associatedMessageGuid": null, "associatedMessageType": null,
  "expressiveSendStyleId": null, "threadOriginatorGuid": null, "hasPayloadData": false,
  "country": null, "isDelayed": false, "isAutoReply": false, "isSystemMessage": false,
  "isServiceMessage": false, "isForward": false, "threadOriginatorPart": null,
  "isCorrupt": false, "datePlayed": null, "cacheRoomnames": null, "isSpam": false,
  "isExpired": false, "timeExpressiveSendPlayed": null, "isAudioMessage": false,
  "replyToGuid": null, "shareStatus": 0, "shareDirection": 0,
  "wasDeliveredQuietly": false, "didNotifyRecipient": false,
  "chats": [{ "originalROWID": 55, "guid": "iMessage;-;+15551234567", "style": 45,
              "chatIdentifier": "+15551234567", "isArchived": false, "displayName": "",
              "isFiltered": false, "groupId": "...", "properties": null, "lastAddressedHandle": "me@icloud.com" }],
  "messageSummaryInfo": null, "payloadData": null,
  "dateEdited": null, "dateRetracted": null, "partCount": 2
}
```

Tapback example (slim fields only): `{"guid":"...","text":"Loved “look at this”","associatedMessageGuid":"p:0/6A1F2C3D-...","associatedMessageType":"love","isFromMe":true, ...}`

---

## 5. Private API and AppleScript

`S/helpers/utils.ts#checkPrivateApiStatus`, `S/api/http/api/v1/middleware/privateApiMiddleware.ts`, interfaces.

The Private API is available when `enable_private_api` is true **and** the helper dylib is connected. A client detects this with `GET /server/info` → `private_api && helper_connected`. Capture `os_version` too, for OS-gated features.

| Feature | Requires PAPI | Extra OS gate |
|---|---|---|
| Send plain text | no (apple-script). Optional private-api. | |
| Send with effect / subject / reply (`selectedMessageGuid`) / `attributedBody` / ddScan | **yes** (method forced to private-api) | ddScan: Catalina+ |
| Send attachment | no (apple-script). private-api optional or forced as above. | |
| Multipart (mentions, multiple attachments), `/attachment/upload` | yes | |
| Tapbacks (`/message/react`) | yes | |
| Edit, unsend | yes | **Ventura (13)+** |
| Notify anyway | yes | Monterey+ |
| Typing indicators: send (`/chat/:guid/typing`) and receive (`typing-indicator`) | yes. Received only for DMs. | |
| Mark read / unread | yes. Unread acts on the Mac only on Ventura+; the event is emitted anyway. | |
| Read receipts and delivered (from the DB poller) | **no** | |
| Rename group, add/remove participant, leave, delete chat, delete message | yes | |
| Set/remove group icon | yes | Big Sur+ |
| Create chat (private-api method, groups) | yes. AppleScript on Big Sur+ is DM-only and needs a message. | |
| iMessage availability, FaceTime availability, focus status | yes | focus: Monterey+ |
| FaceTime session/answer/leave | yes | Monterey+ |
| Embedded media (Digital Touch / handwriting) | yes | |
| Force download of purged attachments | yes | |
| Contact-share status / share | yes | Big Sur+ |
| iCloud account / alias / contact card, Find My friends | yes | |
| Message text search on Ventura+ | uses PAPI if enabled, else DB (outgoing text is NULL on Ventura+, so a DB-only search misses your own messages) | |

Errors when the PAPI is missing:

- Routes with middleware → 500 `iMessage Error`, "Please make sure you have completed the setup for the Private API, and your helper is connected!".
- Interface checks → 500 with `error.message` "iMessage Private API is not enabled!" or "iMessage Private API Helper is not connected!".

---

## 6. Quirks a client must handle

1. **Sent-message reconciliation.** Generate a `tempGuid` (UUID) for every send. Insert an optimistic row keyed by it. Then expect, in any order:
   - (a) the HTTP response `data` with real `guid` + `tempGuid`
   - (b) socket `new-message` with `tempGuid` (slim)
   - (c) socket `new-message` without `tempGuid`, same `guid` (full)
   - (d) later, `updated-message` for delivered/read

   Match (a)/(b) by `tempGuid`, then merge everything by `guid`. Prefer full payloads over slim ones: slim lacks `attributedBody` and non-essential fields. (`S/managers/outgoingMessageManager/messagePromise.ts`, `S/databases/imessage/pollers/MessagePoller.ts`: `messageManager.resolve()` is not awaited before the poll results are emitted.)
   - Multipart and private-api chat creation do not produce (b).
   - Reactions sent through REST have no tempGuid.
2. **`new-message` can be a re-emit of a message you already have.** The poller's "seen" cache (`IMessageCache.events`) is trimmed after **1 hour**, and its state cache is cleared on restart. If a message changes later (read receipt, edit, unsend, delivery) more than an hour after the server first saw it, it comes as `new-message`, not `updated-message` (`S/databases/imessage/pollers/index.ts#getMessageEvent`). **Always upsert by `guid`.** Never append blindly.
3. **Missed events.** There is no replay. After a reconnect, resync with `POST /message/query` (`after = last seen dateCreated`, or a `where message.ROWID > :id` cursor, `sort: ASC`) and the update counter `GET /message/count/updated?after=`.
   - The poller looks back 1 week by `dateCreated`, from `lastCheck − 30 s`, capped at 24 h after downtime (`S/databases/imessage/listeners/IMessageListener.ts`).
   - The server does not buffer events for disconnected clients.
4. **Ordering.** Within one poll, results are sorted by `dateCreated` ascending and emitted 10 ms apart. Polls run on chat.db/WAL file changes, debounced 500 ms. Display by `dateCreated`, not arrival order. REST pages default to `DESC` (`/chat/:guid/message`, `/message/query`). Socket `get-messages` defaults to `ASC`.
5. **Outgoing messages may show up before they are sent.** The poller emits from-me rows that are still unsent (`isDelivered:false`, `dateDelivered:null`). Failure comes later as `message-send-error`, with `error != 0`.
6. **Attachments.** An incoming attachment message arrives as `new-message` with `attachments[]` already populated. The file may not be downloaded yet: `transferState != 5`, `height`/`width` 0. **No event fires when the download finishes**, because attachment state is not part of the update check. Retry `GET /attachment/:guid/download` with backoff, or call `/download/force` (PAPI). Download returns 500 "Attachment does not exist in disk!" while the file is missing.
   - Attachment JSON already describes the **converted** file (HEIC → JPEG, CAF → MP3).
   - Pass `original=true` for the raw file.
   - Use `width`/`height`/`quality` for thumbnails; the output is always PNG.
7. **Dates** are ms since Unix epoch everywhere in message/chat/attachment JSON and in query params. `null` means unset. Scheduled-message dates are ISO strings. FaceTime `incoming-facetime` is a JSON **string**, and its `timestamp` is ms.
8. **Pagination.** `limit` max is 1000 on chat query, chat messages, message query, and handle query. Defaults: chat query 1000, others 100. `metadata.total` is the total match count. Chat `sort=lastmessage` is per page only (2.1).
9. **Error-shaped success.** A send can return HTTP 500 with a full message in `data` (`error` code set). Treat that as "sent but failed", and use the returned `guid`.
10. **Non-JSON errors.** Unknown routes return a plain-text 404, and so does the `/message/query` nonexistent-chat case. Downloads and icons return raw bytes on success but JSON on error. Check `Content-Type`.
11. **Duplicate sends.** A `tempGuid` that is still in flight is rejected with 400. The send cache is purged every 6 h.
12. **SMS vs iMessage.** These are separate chats and separate handles for the same address (`iMessage;-;X` vs `SMS;-;X`). Choose the GUID by the user's intended service. The server's GUID normalisation never crosses services. On Tahoe, GUIDs use `any;` and you can still send with an `iMessage;`/`SMS;` GUID. Typing indicators never fire for group chats.
13. **Participants in message payloads.** Socket messages carry `chats[]` without participants. The serializer's participant-loading loop has an index bug (`for (let k...; i++)` in `MessageSerializer.serializeList`), so do not rely on `chats[].participants` inside messages. Fetch chats separately (`/chat/query` or `/chat/:guid?with=participants`). Group events (`participant-*`, `group-name-change`) are serialized with participants loaded, subject to the same bug.
14. **Chats without handles** (for example after leaving) are left out of `/chat/query`, because of the INNER JOIN on participants.
15. **Large payloads.** The socket max buffer is 100 MB. `get-attachment` over the socket base64-encodes the entire file, so use REST downloads. FCM payloads are capped at 4000 bytes (participants stripped), which does not matter for socket clients.
16. **Throttle.** Auth failures are throttled per IP (1.2). A client with a stale password retries without getting locked out, since repeats count once. After a 429, honour `Retry-After`.
17. **Version differences visible in this code.**
    - Fields depend on macOS: `dateEdited`/`dateRetracted`/`partCount` only on Ventura+. `wasDeliveredQuietly`/`didNotifyRecipient` only on Monterey+. `threadOriginatorPart` only on Big Sur+. `messageSummaryInfo`/`payloadData` only on High Sierra+.
    - Find My devices are `null` on Sequoia+.
    - Older servers (before about v1.9.x, unverifiable here) may lack Tahoe GUID resolution, the auth throttle, and the where-clause validator. Use `server_version` from `/server/info` to gate. Make every field optional or nullable in Rust structs (`#[serde(default)]`).
18. **Events with no data.** `hello-world` (only on server start) and `server-update-*` carry `null`. Prepare a `serde_json::Value` fallback for unknown events.
