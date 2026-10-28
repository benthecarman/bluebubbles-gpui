use crate::{APPLE_EPOCH_MS, Shared, full, now_ms, slim, socket};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use bb_api::{Message, SendText};
use serde_json::{Value, json};
use std::collections::HashMap;

type Q = Query<HashMap<String, String>>;

pub(crate) fn router(shared: Shared) -> Router {
    Router::new()
        .route("/socket.io/", any(socket::upgrade))
        .route("/api/v1/ping", get(ping))
        .route("/api/v1/server/info", get(info))
        .route("/api/v1/chat/query", post(chat_query))
        .route("/api/v1/chat/new", post(chat_new))
        .route("/api/v1/chat/{guid}", get(chat_get))
        .route("/api/v1/chat/{guid}/message", get(chat_messages))
        .route("/api/v1/chat/{guid}/read", post(chat_read))
        .route("/api/v1/chat/{guid}/unread", post(chat_unread))
        .route("/api/v1/chat/{guid}/typing", post(typing).delete(typing))
        .route("/api/v1/chat/{guid}/icon", get(chat_icon))
        .route("/api/v1/message/query", post(message_query))
        .route("/api/v1/message/text", post(send_text))
        .route("/api/v1/message/attachment", post(send_attachment))
        .route("/api/v1/message/react", post(react))
        .route("/api/v1/message/{guid}", get(message_get))
        .route("/api/v1/message/{guid}/edit", post(edit))
        .route("/api/v1/message/{guid}/unsend", post(unsend))
        .route("/api/v1/attachment/{guid}", get(attachment_get))
        .route("/api/v1/attachment/{guid}/download", get(download))
        .route("/api/v1/contact", get(contacts))
        .with_state(shared)
}

fn ok(data: Value) -> Response {
    axum::Json(json!({ "status": 200, "message": "Success", "data": data })).into_response()
}

fn ok_meta(data: Value, total: usize) -> Response {
    axum::Json(json!({
        "status": 200, "message": "Success", "data": data,
        "metadata": { "total": total, "count": data.as_array().map_or(0, Vec::len) }
    }))
    .into_response()
}

fn error(status: StatusCode, message: &str, data: Option<Value>) -> Response {
    let mut body = json!({
        "status": status.as_u16(),
        "message": message,
        "error": { "type": "Server Error", "message": message },
    });
    if let Some(data) = data {
        body["data"] = data;
    }
    (status, axum::Json(body)).into_response()
}

/// Checks the password and logs the request, as the server's auth
/// middleware runs first.
#[allow(clippy::result_large_err)]
fn auth(
    shared: &Shared,
    q: &HashMap<String, String>,
    method: &Method,
    uri: &Uri,
) -> Result<(), Response> {
    let mut s = shared.lock();
    let given = q.get("guid").or(q.get("password")).or(q.get("token"));
    if given != Some(&s.password) {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "You are not authorized to access this resource",
            None,
        ));
    }
    let path = crate::decode_uri(uri.path()).unwrap_or_else(|| uri.path().to_owned());
    s.requests.push(format!("{method} {path}"));
    Ok(())
}

macro_rules! authed {
    ($shared:expr, $q:expr, $method:expr, $uri:expr) => {
        if let Err(r) = auth(&$shared, &$q, &$method, &$uri) {
            return r;
        }
    };
}

async fn ping(State(s): State<Shared>, Query(q): Q, method: Method, uri: Uri) -> Response {
    authed!(s, q, method, uri);
    ok(json!("pong"))
}

async fn info(State(s): State<Shared>, Query(q): Q, method: Method, uri: Uri) -> Response {
    authed!(s, q, method, uri);
    let info = s.lock().info.clone();
    ok(serde_json::to_value(info).unwrap_or_default())
}

fn chat_json(s: &crate::State, guid: &str) -> Option<Value> {
    let chat = s.chat(guid)?.clone();
    let mut v = serde_json::to_value(&chat).ok()?;
    v["lastMessage"] = s
        .last_message(guid)
        .map(|m| full(&m))
        .unwrap_or(Value::Null);
    Some(v)
}

async fn chat_query(
    State(s): State<Shared>,
    Query(q): Q,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    authed!(s, q, method, uri);
    let body: Value = serde_json::from_slice(&body).unwrap_or_default();
    let offset = body["offset"].as_u64().unwrap_or(0) as usize;
    let limit = body["limit"].as_u64().unwrap_or(1000) as usize;
    let st = s.lock();
    let mut chats: Vec<Value> = st
        .chats
        .iter()
        .filter_map(|c| chat_json(&st, &c.guid))
        .collect();
    let total = chats.len();
    chats.sort_by_key(|c| std::cmp::Reverse(c["lastMessage"]["dateCreated"].as_i64().unwrap_or(0)));
    let page: Vec<Value> = chats.into_iter().skip(offset).take(limit).collect();
    ok_meta(Value::Array(page), total)
}

async fn chat_get(
    State(s): State<Shared>,
    Path(guid): Path<String>,
    Query(q): Q,
    method: Method,
    uri: Uri,
) -> Response {
    authed!(s, q, method, uri);
    match chat_json(&s.lock(), &guid) {
        Some(v) => ok(v),
        None => error(StatusCode::NOT_FOUND, "Chat does not exist", None),
    }
}

async fn chat_new(
    State(s): State<Shared>,
    Query(q): Q,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    authed!(s, q, method, uri);
    let body: Value = serde_json::from_slice(&body).unwrap_or_default();
    let addresses: Vec<String> = body["addresses"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let service = body["service"].as_str().unwrap_or("iMessage").to_owned();
    let guid = if addresses.len() == 1 {
        format!("{service};-;{}", addresses[0])
    } else {
        format!("{service};+;chat{}", now_ms())
    };
    let text = body["message"].as_str().unwrap_or_default().to_owned();
    let temp = body["tempGuid"].as_str().unwrap_or_default().to_owned();
    {
        let mut st = s.lock();
        if st.chat(&guid).is_none() {
            let handles = addresses.iter().map(|a| st.handle(a, &service)).collect();
            let n = st.chats.len() as i64 + 1;
            st.chats.push(bb_api::Chat {
                original_rowid: Some(n),
                guid: guid.clone(),
                style: Some(if addresses.len() > 1 { 43 } else { 45 }),
                chat_identifier: addresses.first().cloned(),
                participants: Some(handles),
                display_name: Some(String::new()),
                ..Default::default()
            });
        }
    }
    let m = store_sent(&s, &guid, &text, None);
    s.emit("new-message", slim(&m, &temp));
    s.emit("new-message", full(&m));
    let st = s.lock();
    let mut chat = chat_json(&st, &guid).unwrap_or_default();
    let mut mv = full(&m);
    mv["tempGuid"] = json!(temp);
    chat["messages"] = json!([mv]);
    ok(chat)
}

fn in_chat(m: &Message, guid: &str) -> bool {
    m.chats.first().is_some_and(|c| c.guid == guid)
}

async fn chat_messages(
    State(s): State<Shared>,
    Path(guid): Path<String>,
    Query(q): Q,
    method: Method,
    uri: Uri,
) -> Response {
    authed!(s, q, method, uri);
    let st = s.lock();
    if st.chat(&guid).is_none() {
        return (StatusCode::NOT_FOUND, "Not Found").into_response();
    }
    let before = q
        .get("before")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(i64::MAX);
    let after = q
        .get("after")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let limit = q
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100);
    let mut found: Vec<Message> = st
        .messages
        .iter()
        .map(|m| &m.message)
        .filter(|m| in_chat(m, &guid))
        .filter(|m| {
            let d = m.date_created.unwrap_or(0);
            d < before && d >= after
        })
        .cloned()
        .collect();
    found.sort_by_key(|m| std::cmp::Reverse((m.date_created, m.original_rowid)));
    if q.get("sort").map(String::as_str) == Some("ASC") {
        found.reverse();
    }
    let total = found.len();
    let page: Vec<Value> = found
        .into_iter()
        .take(limit)
        .map(|mut m| {
            m.chats = Vec::new();
            full(&m)
        })
        .collect();
    ok_meta(Value::Array(page), total)
}

/// Supports the statements this client sends: `message.ROWID > :x` and
/// the reconcile query over date columns (nanoseconds since 2001).
async fn message_query(
    State(s): State<Shared>,
    Query(q): Q,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    authed!(s, q, method, uri);
    let body: Value = serde_json::from_slice(&body).unwrap_or_default();
    let limit = body["limit"].as_u64().unwrap_or(100) as usize;
    let offset = body["offset"].as_u64().unwrap_or(0) as usize;
    let after = body["after"].as_i64();
    let before = body["before"].as_i64();
    let chat = body["chatGuid"].as_str().map(str::to_owned);
    let mut min_rowid: Option<i64> = None;
    let mut updated_since: Option<i64> = None;
    for clause in body["where"].as_array().cloned().unwrap_or_default() {
        let statement = clause["statement"].as_str().unwrap_or_default();
        let arg = clause["args"]
            .as_object()
            .and_then(|a| a.values().next())
            .and_then(Value::as_f64);
        if statement.contains("ROWID >") {
            min_rowid = arg.map(|a| a as i64);
        } else if statement.contains("date_") {
            updated_since = arg.map(|ns| (ns / 1_000_000.0) as i64 + APPLE_EPOCH_MS);
        } else {
            return error(
                StatusCode::BAD_REQUEST,
                &format!("mock: unsupported where {statement}"),
                None,
            );
        }
    }
    let st = s.lock();
    let mut found: Vec<Message> = st
        .messages
        .iter()
        .filter(|m| chat.as_ref().is_none_or(|c| in_chat(&m.message, c)))
        .filter(|m| after.is_none_or(|a| m.message.date_created.unwrap_or(0) >= a))
        .filter(|m| before.is_none_or(|b| m.message.date_created.unwrap_or(0) <= b))
        .filter(|m| min_rowid.is_none_or(|r| m.message.original_rowid.unwrap_or(0) > r))
        .filter(|m| {
            updated_since.is_none_or(|t| {
                m.updated_at >= t && m.message.date_created.unwrap_or(0) < m.updated_at
            })
        })
        .map(|m| m.message.clone())
        .collect();
    found.sort_by_key(|m| (m.date_created, m.original_rowid));
    if body["sort"].as_str() != Some("ASC") {
        found.reverse();
    }
    let total = found.len();
    let page: Vec<Value> = found
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|m| full(&m))
        .collect();
    ok_meta(Value::Array(page), total)
}

async fn message_get(
    State(s): State<Shared>,
    Path(guid): Path<String>,
    Query(q): Q,
    method: Method,
    uri: Uri,
) -> Response {
    authed!(s, q, method, uri);
    let st = s.lock();
    match st.messages.iter().find(|m| m.message.guid == guid) {
        Some(m) => ok(full(&m.message)),
        None => error(StatusCode::NOT_FOUND, "Message does not exist", None),
    }
}

fn store_sent(s: &Shared, chat: &str, text: &str, edit: Option<&dyn Fn(&mut Message)>) -> Message {
    let mut st = s.lock();
    let mut m = st.new_message(chat, None, text, now_ms());
    if let Some(edit) = edit {
        edit(&mut m);
    }
    st.messages.push(crate::Stored {
        message: m.clone(),
        updated_at: now_ms(),
    });
    m
}

/// The server's three answers to a send, with this mock's behaviour.
async fn finish_send(s: &Shared, mut m: Message, temp: &str) -> Response {
    let behaviour = std::mem::take(&mut s.lock().send);
    if !behaviour.delay.is_zero() {
        tokio::time::sleep(behaviour.delay).await;
    }
    if let Some(code) = behaviour.fail_with {
        let guid = m.guid.clone();
        s.lock()
            .messages
            .iter_mut()
            .filter(|x| x.message.guid == guid)
            .for_each(|x| x.message.error = Some(code));
        m.error = Some(code);
        let mut payload = slim(&m, temp);
        payload["error"] = json!(code);
        s.emit("message-send-error", payload);
        let mut data = full(&m);
        data["tempGuid"] = json!(temp);
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "iMessage has encountered an error",
            Some(data),
        );
    }
    if behaviour.full_echo_first {
        s.emit("new-message", full(&m));
        s.emit("new-message", slim(&m, temp));
    } else {
        s.emit("new-message", slim(&m, temp));
        s.emit("new-message", full(&m));
    }
    if behaviour.lose_reply {
        return error(StatusCode::GATEWAY_TIMEOUT, "The request timed-out", None);
    }
    let mut data = full(&m);
    data["tempGuid"] = json!(temp);
    ok(data)
}

async fn send_text(
    State(s): State<Shared>,
    Query(q): Q,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    authed!(s, q, method, uri);
    let Ok(send) = serde_json::from_slice::<Value>(&body) else {
        return error(StatusCode::BAD_REQUEST, "bad body", None);
    };
    let send = SendText {
        chat_guid: send["chatGuid"].as_str().unwrap_or_default().into(),
        temp_guid: send["tempGuid"].as_str().unwrap_or_default().into(),
        message: send["message"].as_str().unwrap_or_default().into(),
        method: send["method"].as_str().unwrap_or("apple-script").into(),
        selected_message_guid: send["selectedMessageGuid"].as_str().map(str::to_owned),
        part_index: send["partIndex"].as_i64(),
        effect_id: send["effectId"].as_str().map(str::to_owned),
        subject: send["subject"].as_str().map(str::to_owned),
    };
    if s.lock().chat(&send.chat_guid).is_none() {
        return error(StatusCode::BAD_REQUEST, "Chat does not exist", None);
    }
    let reply = send.selected_message_guid.clone();
    let part = send.part_index;
    let effect = send.effect_id.clone();
    let m = store_sent(
        &s,
        &send.chat_guid,
        &send.message,
        Some(&move |m: &mut Message| {
            m.thread_originator_guid = reply.clone();
            m.thread_originator_part = reply.as_ref().map(|_| format!("{}:0:0", part.unwrap_or(0)));
            m.expressive_send_style_id = effect.clone();
        }),
    );
    finish_send(&s, m, &send.temp_guid).await
}

async fn send_attachment(
    State(s): State<Shared>,
    Query(q): Q,
    method: Method,
    uri: Uri,
    mut form: Multipart,
) -> Response {
    authed!(s, q, method, uri);
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut file: Option<(String, String, Vec<u8>)> = None;
    while let Ok(Some(field)) = form.next_field().await {
        let name = field.name().unwrap_or_default().to_owned();
        if name == "attachment" {
            let file_name = field.file_name().unwrap_or("file").to_owned();
            let mime = field
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_owned();
            let bytes = field.bytes().await.map(|b| b.to_vec()).unwrap_or_default();
            file = Some((file_name, mime, bytes));
        } else {
            fields.insert(name, field.text().await.unwrap_or_default());
        }
    }
    let (Some(chat), Some((name, mime, bytes))) = (fields.get("chatGuid").cloned(), file) else {
        return error(
            StatusCode::BAD_REQUEST,
            "missing attachment or chatGuid",
            None,
        );
    };
    let attachment = s.lock().attachment(&name, &mime, bytes);
    let m = store_sent(
        &s,
        &chat,
        "",
        Some(&move |m: &mut Message| {
            m.text = None;
            m.attachments = vec![attachment.clone()];
        }),
    );
    let temp = fields.get("tempGuid").cloned().unwrap_or_default();
    finish_send(&s, m, &temp).await
}

async fn react(
    State(s): State<Shared>,
    Query(q): Q,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    authed!(s, q, method, uri);
    let body: Value = serde_json::from_slice(&body).unwrap_or_default();
    let chat = body["chatGuid"].as_str().unwrap_or_default().to_owned();
    let target = body["selectedMessageGuid"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let reaction = body["reaction"].as_str().unwrap_or_default().to_owned();
    let part = body["partIndex"].as_i64().unwrap_or(0);
    let text = format!(
        "{} “{}”",
        reaction,
        body["selectedMessageText"].as_str().unwrap_or_default()
    );
    let m = store_sent(
        &s,
        &chat,
        &text,
        Some(&move |m: &mut Message| {
            m.associated_message_guid = Some(format!("p:{part}/{target}"));
            m.associated_message_type = Some(reaction.clone());
        }),
    );
    s.emit("new-message", full(&m));
    ok(full(&m))
}

async fn edit(
    State(s): State<Shared>,
    Path(guid): Path<String>,
    Query(q): Q,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    authed!(s, q, method, uri);
    let body: Value = serde_json::from_slice(&body).unwrap_or_default();
    let text = body["editedMessage"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    update_and_emit(&s, &guid, |m| {
        m.text = Some(text.clone());
        m.date_edited = Some(now_ms());
    })
}

async fn unsend(
    State(s): State<Shared>,
    Path(guid): Path<String>,
    Query(q): Q,
    method: Method,
    uri: Uri,
) -> Response {
    authed!(s, q, method, uri);
    update_and_emit(&s, &guid, |m| {
        m.text = None;
        m.date_edited = Some(now_ms());
        m.part_count = Some(0);
        m.message_summary_info = Some(json!([{ "retractedParts": [0] }]));
    })
}

fn update_and_emit(s: &Shared, guid: &str, f: impl FnOnce(&mut Message)) -> Response {
    let m = {
        let mut st = s.lock();
        let Some(stored) = st.messages.iter_mut().find(|m| m.message.guid == guid) else {
            return error(StatusCode::NOT_FOUND, "Message does not exist", None);
        };
        f(&mut stored.message);
        stored.updated_at = now_ms();
        stored.message.clone()
    };
    s.emit("updated-message", full(&m));
    ok(full(&m))
}

async fn chat_read(
    State(s): State<Shared>,
    Path(guid): Path<String>,
    Query(q): Q,
    method: Method,
    uri: Uri,
) -> Response {
    authed!(s, q, method, uri);
    s.emit(
        "chat-read-status-changed",
        json!({ "chatGuid": guid, "read": true }),
    );
    ok(Value::Null)
}

async fn chat_unread(
    State(s): State<Shared>,
    Path(guid): Path<String>,
    Query(q): Q,
    method: Method,
    uri: Uri,
) -> Response {
    authed!(s, q, method, uri);
    s.emit(
        "chat-read-status-changed",
        json!({ "chatGuid": guid, "read": false }),
    );
    ok(Value::Null)
}

async fn typing(
    State(s): State<Shared>,
    Path(_guid): Path<String>,
    Query(q): Q,
    method: Method,
    uri: Uri,
) -> Response {
    authed!(s, q, method, uri);
    ok(Value::Null)
}

async fn chat_icon(
    State(s): State<Shared>,
    Path(_guid): Path<String>,
    Query(q): Q,
    method: Method,
    uri: Uri,
) -> Response {
    authed!(s, q, method, uri);
    error(StatusCode::NOT_FOUND, "no icon", None)
}

async fn attachment_get(
    State(s): State<Shared>,
    Path(guid): Path<String>,
    Query(q): Q,
    method: Method,
    uri: Uri,
) -> Response {
    authed!(s, q, method, uri);
    let st = s.lock();
    let found = st
        .messages
        .iter()
        .flat_map(|m| &m.message.attachments)
        .find(|a| a.guid == guid);
    match found {
        Some(a) => ok(serde_json::to_value(a).unwrap_or_default()),
        None => error(StatusCode::NOT_FOUND, "Attachment does not exist!", None),
    }
}

async fn download(
    State(s): State<Shared>,
    Path(guid): Path<String>,
    Query(q): Q,
    method: Method,
    uri: Uri,
) -> Response {
    authed!(s, q, method, uri);
    match s.lock().attachment_data.get(&guid).cloned() {
        Some((mime, bytes)) => ([(header::CONTENT_TYPE, mime)], bytes).into_response(),
        None => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Attachment does not exist in disk!",
            None,
        ),
    }
}

async fn contacts(State(s): State<Shared>, Query(q): Q, method: Method, uri: Uri) -> Response {
    authed!(s, q, method, uri);
    let contacts = s.lock().contacts.clone();
    ok(serde_json::to_value(contacts).unwrap_or_default())
}
