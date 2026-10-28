use crate::models::{Attachment, Chat, Contact, Envelope, Message, ServerInfo};
use reqwest::{Method, StatusCode, multipart};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::fmt;
use std::time::Duration;
use url::Url;

/// How to reach a server.
#[derive(Clone)]
pub struct ServerConfig {
    /// `http(s)://host[:port][/prefix]`, as the server shows it.
    pub url: Url,
    pub password: String,
    /// Extra headers some tunnels need.
    pub headers: Vec<(String, String)>,
    /// Accept a self-signed or otherwise invalid certificate. Off unless
    /// the user turns it on for a server on their own network.
    pub accept_invalid_certs: bool,
    pub timeout: Duration,
}

impl fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerConfig")
            .field("url", &self.url.as_str())
            .field("password", &"<redacted>")
            .field("headers", &self.headers.len())
            .field("accept_invalid_certs", &self.accept_invalid_certs)
            .finish()
    }
}

impl ServerConfig {
    pub fn new(url: Url, password: impl Into<String>) -> Self {
        Self {
            url,
            password: password.into(),
            headers: Vec::new(),
            accept_invalid_certs: false,
            timeout: Duration::from_secs(30),
        }
    }

    /// Headers the client adds for known tunnel services, plus the user's.
    pub fn effective_headers(&self) -> Vec<(String, String)> {
        let mut headers = self.headers.clone();
        let host = self.url.host_str().unwrap_or_default();
        if host.contains("ngrok") {
            headers.push(("ngrok-skip-browser-warning".into(), "true".into()));
        }
        if host.contains("zrok") {
            headers.push(("skip_zrok_interstitial".into(), "true".into()));
        }
        headers
    }
}

/// Tunnel services, which serve https.
const TUNNELS: &[&str] = &[
    "ngrok.io",
    "ngrok-free.app",
    "ngrok.app",
    "trycloudflare.com",
    "zrok.io",
];

/// The BlueBubbles server's listening port unless changed.
pub const DEFAULT_PORT: u16 = 1234;

/// Parses what a user types as a server address and drops a trailing
/// slash. Without a scheme, known tunnels get https and anything else (a
/// LAN or Tailscale address) gets http and, unless given, the server's
/// default port.
pub fn parse_server_url(input: &str) -> Result<Url, ApiError> {
    let trimmed = input.trim().trim_matches('"').trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(ApiError::BadUrl("enter the server's address".into()));
    }
    let url = if trimmed.contains("://") {
        Url::parse(trimmed)
    } else if TUNNELS.iter().any(|t| trimmed.contains(t)) {
        Url::parse(&format!("https://{trimmed}"))
    } else {
        // A bare IPv6 address, such as a Tailscale one, needs brackets.
        let host = if trimmed.parse::<std::net::Ipv6Addr>().is_ok() {
            format!("[{trimmed}]")
        } else {
            trimmed.to_owned()
        };
        Url::parse(&format!("http://{host}")).map(|mut url| {
            if url.port().is_none() {
                let _ = url.set_port(Some(DEFAULT_PORT));
            }
            url
        })
    };
    let url = url.map_err(|e| ApiError::BadUrl(e.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
        return Err(ApiError::BadUrl(
            "use an http:// or https:// address".into(),
        ));
    }
    Ok(url)
}

/// Errors carry no URLs: the password is a query parameter.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ApiError {
    #[error("invalid server address: {0}")]
    BadUrl(String),
    #[error("the server rejected the password")]
    Unauthorized,
    #[error("not found")]
    NotFound,
    #[error("the server answered {status}: {message}")]
    Server { status: u16, message: String },
    #[error("timed out")]
    Timeout,
    #[error("could not reach the server: {0}")]
    Network(String),
    #[error("unexpected response: {0}")]
    Decode(String),
}

impl ApiError {
    fn from_reqwest(e: reqwest::Error) -> Self {
        let e = e.without_url();
        if e.is_timeout() {
            ApiError::Timeout
        } else if e.is_decode() {
            ApiError::Decode(e.to_string())
        } else {
            ApiError::Network(e.to_string())
        }
    }

    /// Whether retrying the same request later could succeed.
    pub fn is_transient(&self) -> bool {
        match self {
            ApiError::Timeout | ApiError::Network(_) => true,
            ApiError::Server { status, .. } => *status >= 500,
            _ => false,
        }
    }
}

/// A page of results and the server's total, when it sends one.
#[derive(Debug, Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: Option<u64>,
}

/// The `with` options that give a message everything this client draws.
pub const MESSAGE_WITH: &[&str] = &[
    "chats",
    "attachment",
    "attributedBody",
    "messageSummaryInfo",
    "payloadData",
];

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_guid: Option<String>,
    pub with: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub r#where: Vec<WhereClause>,
    /// `ASC` or `DESC` by date.
    pub sort: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<i64>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhereClause {
    pub statement: String,
    pub args: Value,
}

impl MessageQuery {
    pub fn new(limit: u32) -> Self {
        Self {
            with: MESSAGE_WITH.iter().map(|s| (*s).to_owned()).collect(),
            sort: "ASC".into(),
            limit,
            ..Default::default()
        }
    }
}

/// `POST /message/text`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendText {
    pub chat_guid: String,
    pub temp_guid: String,
    pub message: String,
    /// `apple-script` or `private-api`.
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_message_guid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_index: Option<i64>,
}

/// `POST /message/attachment`.
#[derive(Debug, Clone, Default)]
pub struct SendAttachment {
    pub chat_guid: String,
    pub temp_guid: String,
    pub file_name: String,
    pub mime_type: String,
    pub data: Vec<u8>,
    pub method: String,
    pub selected_message_guid: Option<String>,
    pub part_index: Option<i64>,
    pub is_audio_message: bool,
}

/// `GET /attachment/:guid/download` options.
#[derive(Debug, Clone, Copy, Default)]
pub struct Download {
    /// The file as stored on the Mac; otherwise the server converts HEIC
    /// to JPEG and CAF to MP3.
    pub original: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub quality: Option<u32>,
}

/// REST client for one server. Cheap to clone.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: Url,
    password: String,
    timeout: Duration,
}

/// Uploads and downloads get this many times the normal timeout.
const TRANSFER_TIMEOUT_FACTOR: u32 = 12;

impl Client {
    pub fn new(config: &ServerConfig) -> Result<Self, ApiError> {
        let mut headers = reqwest::header::HeaderMap::new();
        for (k, v) in config.effective_headers() {
            let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                .map_err(|e| ApiError::BadUrl(format!("header {k}: {e}")))?;
            let value = reqwest::header::HeaderValue::from_str(&v)
                .map_err(|e| ApiError::BadUrl(format!("header {k}: {e}")))?;
            headers.insert(name, value);
        }
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(15))
            .danger_accept_invalid_certs(config.accept_invalid_certs)
            .user_agent(concat!("bluebubbles-gpui/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(ApiError::from_reqwest)?;
        let mut base = config.url.clone();
        let path = format!("{}/api/v1/", base.path().trim_end_matches('/'));
        base.set_path(&path);
        base.set_query(None);
        Ok(Self {
            http,
            base,
            password: config.password.clone(),
            timeout: config.timeout,
        })
    }

    /// The full URL for `path` with the password. Never log it.
    pub fn url(&self, path: &str, query: &[(&str, String)]) -> Url {
        let mut url = self
            .base
            .join(path.trim_start_matches('/'))
            .expect("API paths are relative");
        {
            let mut q = url.query_pairs_mut();
            for (k, v) in query {
                q.append_pair(k, v);
            }
            q.append_pair("guid", &self.password);
        }
        url
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response, ApiError> {
        let response = request.send().await.map_err(ApiError::from_reqwest)?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        // Error bodies are usually JSON envelopes, sometimes plain text.
        let envelope = response.json::<Envelope<Value>>().await.ok();
        Err(error_for(status, envelope.map(|e| (e.message, e.error))))
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Envelope<T>, ApiError> {
        let mut request = self
            .http
            .request(method, self.url(path, query))
            .timeout(self.timeout);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = self.send(request).await?;
        let bytes = response.bytes().await.map_err(ApiError::from_reqwest)?;
        serde_json::from_slice(&bytes).map_err(|e| ApiError::Decode(e.to_string()))
    }

    async fn data<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<T, ApiError> {
        self.call::<T>(method, path, query, body)
            .await?
            .data
            .ok_or_else(|| ApiError::Decode("response has no data".into()))
    }

    pub async fn ping(&self) -> Result<(), ApiError> {
        self.call::<Value>(Method::GET, "ping", &[], None)
            .await
            .map(drop)
    }

    pub async fn server_info(&self) -> Result<ServerInfo, ApiError> {
        self.data(Method::GET, "server/info", &[], None).await
    }

    /// Chats, most recently active first, with participants and last
    /// message.
    pub async fn chats(&self, offset: u32, limit: u32) -> Result<Page<Chat>, ApiError> {
        let envelope = self
            .call::<Vec<Chat>>(
                Method::POST,
                "chat/query",
                &[],
                Some(json!({
                    "limit": limit,
                    "offset": offset,
                    "with": ["participants", "lastMessage"],
                    "sort": "lastmessage",
                })),
            )
            .await?;
        Ok(page(envelope))
    }

    pub async fn chat(&self, guid: &str) -> Result<Chat, ApiError> {
        self.data(
            Method::GET,
            &format!("chat/{}", encode(guid)),
            &[("with", "participants,lastmessage".into())],
            None,
        )
        .await
    }

    /// A chat's messages, newest first, older than `before` (ms) if given.
    pub async fn chat_messages(
        &self,
        guid: &str,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<Message>, ApiError> {
        let mut query = vec![
            (
                "with",
                "attachment,message.attributedBody,message.messageSummaryInfo,message.payloadData"
                    .to_owned(),
            ),
            ("sort", "DESC".into()),
            ("limit", limit.to_string()),
        ];
        if let Some(before) = before {
            query.push(("before", before.to_string()));
        }
        self.data(
            Method::GET,
            &format!("chat/{}/message", encode(guid)),
            &query,
            None,
        )
        .await
    }

    pub async fn query_messages(&self, query: &MessageQuery) -> Result<Page<Message>, ApiError> {
        let body = serde_json::to_value(query).map_err(|e| ApiError::Decode(e.to_string()))?;
        let envelope = self
            .call::<Vec<Message>>(Method::POST, "message/query", &[], Some(body))
            .await?;
        Ok(page(envelope))
    }

    pub async fn message(&self, guid: &str) -> Result<Message, ApiError> {
        self.data(
            Method::GET,
            &format!("message/{}", encode(guid)),
            &[(
                "with",
                "chats,attachment,handle,attributedBody,messageSummaryInfo,payloadData".into(),
            )],
            None,
        )
        .await
    }

    pub async fn send_text(&self, send: &SendText) -> Result<Message, ApiError> {
        let body = serde_json::to_value(send).map_err(|e| ApiError::Decode(e.to_string()))?;
        // Sending waits for Messages; give it the transfer budget.
        let request = self
            .http
            .post(self.url("message/text", &[]))
            .timeout(self.timeout * 4)
            .json(&body);
        self.send_message(request).await
    }

    pub async fn send_attachment(&self, send: SendAttachment) -> Result<Message, ApiError> {
        let part = multipart::Part::bytes(send.data)
            .file_name(send.file_name.clone())
            .mime_str(&send.mime_type)
            .map_err(|e| ApiError::Decode(e.to_string()))?;
        let mut form = multipart::Form::new()
            .part("attachment", part)
            .text("chatGuid", send.chat_guid)
            .text("tempGuid", send.temp_guid)
            .text("name", send.file_name)
            .text("method", send.method);
        if let Some(guid) = send.selected_message_guid {
            form = form.text("selectedMessageGuid", guid);
        }
        if let Some(part) = send.part_index {
            form = form.text("partIndex", part.to_string());
        }
        if send.is_audio_message {
            form = form.text("isAudioMessage", "true");
        }
        let request = self
            .http
            .post(self.url("message/attachment", &[]))
            .timeout(self.timeout * TRANSFER_TIMEOUT_FACTOR)
            .multipart(form);
        self.send_message(request).await
    }

    /// Sends return the message even when Messages failed to send it
    /// (HTTP 500 with the message, its `error` set); that is still a
    /// message to show, marked failed.
    async fn send_message(&self, request: reqwest::RequestBuilder) -> Result<Message, ApiError> {
        let response = request.send().await.map_err(ApiError::from_reqwest)?;
        let status = response.status();
        if status.is_success() {
            return envelope_data(response).await;
        }
        let bytes = response.bytes().await.map_err(ApiError::from_reqwest)?;
        let envelope = serde_json::from_slice::<Envelope<Message>>(&bytes).ok();
        if let Some(message) = envelope.as_ref().and_then(|e| e.data.clone())
            && !message.guid.is_empty()
        {
            return Ok(message);
        }
        Err(error_for(status, envelope.map(|e| (e.message, e.error))))
    }

    /// Adds a tapback (`love`, `like`, …) or removes one (`-love`, …).
    pub async fn react(
        &self,
        chat_guid: &str,
        message_guid: &str,
        message_text: &str,
        reaction: &str,
        part_index: i64,
    ) -> Result<Message, ApiError> {
        self.data(
            Method::POST,
            "message/react",
            &[],
            Some(json!({
                "chatGuid": chat_guid,
                "selectedMessageGuid": message_guid,
                "selectedMessageText": message_text,
                "reaction": reaction,
                "partIndex": part_index,
            })),
        )
        .await
    }

    pub async fn edit(&self, guid: &str, text: &str, part_index: i64) -> Result<Message, ApiError> {
        self.data(
            Method::POST,
            &format!("message/{}/edit", encode(guid)),
            &[],
            Some(json!({
                "editedMessage": text,
                "backwardsCompatibilityMessage": format!("Edited to “{text}”"),
                "partIndex": part_index,
            })),
        )
        .await
    }

    pub async fn unsend(&self, guid: &str, part_index: i64) -> Result<Message, ApiError> {
        self.data(
            Method::POST,
            &format!("message/{}/unsend", encode(guid)),
            &[],
            Some(json!({ "partIndex": part_index })),
        )
        .await
    }

    pub async fn mark_read(&self, chat_guid: &str) -> Result<(), ApiError> {
        self.call::<Value>(
            Method::POST,
            &format!("chat/{}/read", encode(chat_guid)),
            &[],
            None,
        )
        .await
        .map(drop)
    }

    pub async fn mark_unread(&self, chat_guid: &str) -> Result<(), ApiError> {
        self.call::<Value>(
            Method::POST,
            &format!("chat/{}/unread", encode(chat_guid)),
            &[],
            None,
        )
        .await
        .map(drop)
    }

    pub async fn set_typing(&self, chat_guid: &str, typing: bool) -> Result<(), ApiError> {
        let method = if typing { Method::POST } else { Method::DELETE };
        self.call::<Value>(
            method,
            &format!("chat/{}/typing", encode(chat_guid)),
            &[],
            None,
        )
        .await
        .map(drop)
    }

    /// Starts a conversation with `addresses` by sending `message`.
    pub async fn new_chat(
        &self,
        addresses: &[String],
        message: &str,
        service: &str,
        temp_guid: &str,
        method: &str,
    ) -> Result<Chat, ApiError> {
        self.data(
            Method::POST,
            "chat/new",
            &[],
            Some(json!({
                "addresses": addresses,
                "message": message,
                "service": service,
                "tempGuid": temp_guid,
                "method": method,
            })),
        )
        .await
    }

    /// An attachment's metadata; `transfer_state` 5 means the Mac has
    /// the whole file.
    pub async fn attachment(&self, guid: &str) -> Result<Attachment, ApiError> {
        self.data(
            Method::GET,
            &format!("attachment/{}", encode(guid)),
            &[],
            None,
        )
        .await
    }

    pub async fn download_attachment(
        &self,
        guid: &str,
        options: Download,
    ) -> Result<Vec<u8>, ApiError> {
        let mut query = vec![("original", options.original.to_string())];
        if let Some(w) = options.width {
            query.push(("width", w.to_string()));
        }
        if let Some(h) = options.height {
            query.push(("height", h.to_string()));
        }
        if let Some(q) = options.quality {
            query.push(("quality", q.to_string()));
        }
        let request = self
            .http
            .get(self.url(&format!("attachment/{}/download", encode(guid)), &query))
            .timeout(self.timeout * TRANSFER_TIMEOUT_FACTOR);
        let response = self.send(request).await?;
        Ok(response
            .bytes()
            .await
            .map_err(ApiError::from_reqwest)?
            .to_vec())
    }

    /// The group photo, or `None` when the chat has none.
    pub async fn chat_icon(&self, chat_guid: &str) -> Result<Option<Vec<u8>>, ApiError> {
        let request = self
            .http
            .get(self.url(&format!("chat/{}/icon", encode(chat_guid)), &[]))
            .timeout(self.timeout);
        match self.send(request).await {
            Ok(response) => Ok(Some(
                response
                    .bytes()
                    .await
                    .map_err(ApiError::from_reqwest)?
                    .to_vec(),
            )),
            Err(ApiError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn contacts(&self, avatars: bool) -> Result<Vec<Contact>, ApiError> {
        let query: Vec<(&str, String)> = if avatars {
            vec![("extraProperties", "avatar".into())]
        } else {
            Vec::new()
        };
        self.data(Method::GET, "contact", &query, None).await
    }
}

fn error_for(status: StatusCode, body: Option<(Option<String>, Option<Value>)>) -> ApiError {
    let message = body
        .and_then(|(message, error)| {
            error
                .as_ref()
                .and_then(|err| err.get("message").or(err.get("error")))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or(message)
        })
        .unwrap_or_else(|| status.canonical_reason().unwrap_or("error").to_owned());
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ApiError::Unauthorized,
        StatusCode::NOT_FOUND => ApiError::NotFound,
        _ => ApiError::Server {
            status: status.as_u16(),
            message,
        },
    }
}

/// The socket handshake's `password` value. The server URL-decodes it a
/// second time (`decodeURI`), so a literal `%` must arrive as `%25`.
pub fn socket_password(password: &str) -> String {
    password.replace('%', "%25")
}

fn page<T>(envelope: Envelope<Vec<T>>) -> Page<T> {
    let total = envelope
        .metadata
        .as_ref()
        .and_then(|m| m.get("total"))
        .and_then(Value::as_u64);
    Page {
        items: envelope.data.unwrap_or_default(),
        total,
    }
}

async fn envelope_data<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, ApiError> {
    let bytes = response.bytes().await.map_err(ApiError::from_reqwest)?;
    serde_json::from_slice::<Envelope<T>>(&bytes)
        .map_err(|e| ApiError::Decode(e.to_string()))?
        .data
        .ok_or_else(|| ApiError::Decode("response has no data".into()))
}

/// Chat and message GUIDs contain `;`, `+` and `/`; they go in one path
/// segment.
fn encode(segment: &str) -> String {
    url::form_urlencoded::byte_serialize(segment.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typed_addresses() {
        assert_eq!(
            parse_server_url("abc.ngrok-free.app/").unwrap().as_str(),
            "https://abc.ngrok-free.app/"
        );
        assert_eq!(
            parse_server_url(" 192.168.1.5:1234 ").unwrap().as_str(),
            "http://192.168.1.5:1234/"
        );
        assert!(parse_server_url("ftp://x").is_err());
        // Tailscale: IPs, MagicDNS names and bare IPv6, over plain http.
        let parsed = |s| parse_server_url(s).unwrap().to_string();
        assert_eq!(parsed("100.101.102.103"), "http://100.101.102.103:1234/");
        assert_eq!(
            parsed("100.101.102.103:1234"),
            "http://100.101.102.103:1234/"
        );
        assert_eq!(parsed("mac-mini"), "http://mac-mini:1234/");
        assert_eq!(
            parsed("mac-mini.tail1a2b3c.ts.net:1234"),
            "http://mac-mini.tail1a2b3c.ts.net:1234/"
        );
        assert_eq!(
            parsed("fd7a:115c:a1e0::1"),
            "http://[fd7a:115c:a1e0::1]:1234/"
        );
        assert_eq!(
            parsed("[fd7a:115c:a1e0::1]:1234"),
            "http://[fd7a:115c:a1e0::1]:1234/"
        );
        // A typed scheme keeps its own port.
        assert_eq!(parsed("http://mac-mini"), "http://mac-mini/");
        assert_eq!(
            parsed("https://mac-mini.tail1a2b3c.ts.net"),
            "https://mac-mini.tail1a2b3c.ts.net/"
        );
        assert!(parse_server_url("").is_err());
    }

    #[test]
    fn builds_urls_under_the_prefix_and_hides_the_password() {
        let mut config = ServerConfig::new(Url::parse("https://example.com/bb/").unwrap(), "p w");
        config.headers.push(("X-Test".into(), "1".into()));
        let client = Client::new(&config).unwrap();
        let url = client.url(
            &format!("chat/{}/message", encode("iMessage;+;chat123")),
            &[("limit", "5".into())],
        );
        assert_eq!(
            url.as_str(),
            "https://example.com/bb/api/v1/chat/iMessage%3B%2B%3Bchat123/message?limit=5&guid=p+w"
        );
        assert!(!format!("{config:?}").contains("p w"));
    }

    #[test]
    fn adds_tunnel_headers() {
        let config = ServerConfig::new(Url::parse("https://x.ngrok.io").unwrap(), "p");
        assert!(
            config
                .effective_headers()
                .iter()
                .any(|(k, _)| k == "ngrok-skip-browser-warning")
        );
    }
}
