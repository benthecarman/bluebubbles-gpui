use crate::packet::{EnginePacket, OpenInfo, SocketPacket};
use futures::{SinkExt, StreamExt};
use rand::Rng;
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{Instant, sleep_until, timeout};
use tokio_tungstenite::tungstenite::{self, Message, client::IntoClientRequest, http};
use url::Url;

/// Where and how to connect.
#[derive(Debug, Clone)]
pub struct SocketConfig {
    /// The server's base URL, `http(s)://host[:port][/prefix]`.
    pub server: Url,
    /// Extra query parameters, such as the server password.
    pub query: Vec<(String, String)>,
    /// Extra request headers for the WebSocket upgrade.
    pub headers: Vec<(String, String)>,
    pub backoff: Backoff,
    /// Limit for the TCP/TLS connect plus both handshakes.
    pub connect_timeout: Duration,
    /// How long a request waits for the server's acknowledgement.
    pub ack_timeout: Duration,
}

impl SocketConfig {
    pub fn new(server: Url) -> Self {
        Self {
            server,
            query: Vec::new(),
            headers: Vec::new(),
            backoff: Backoff::default(),
            connect_timeout: Duration::from_secs(15),
            ack_timeout: Duration::from_secs(30),
        }
    }

    /// The WebSocket URL for this server's Engine.IO endpoint.
    pub fn websocket_url(&self) -> Result<Url, SocketError> {
        let mut url = self.server.clone();
        let scheme = match url.scheme() {
            "http" | "ws" => "ws",
            "https" | "wss" => "wss",
            other => return Err(SocketError::BadUrl(format!("unsupported scheme {other}"))),
        };
        url.set_scheme(scheme)
            .map_err(|()| SocketError::BadUrl("cannot set scheme".into()))?;
        let path = format!("{}/socket.io/", url.path().trim_end_matches('/'));
        url.set_path(&path);
        url.set_fragment(None);
        {
            let mut q = url.query_pairs_mut();
            q.clear();
            q.append_pair("EIO", "4");
            q.append_pair("transport", "websocket");
            for (k, v) in &self.query {
                q.append_pair(k, v);
            }
        }
        Ok(url)
    }
}

/// Reconnect delays: `min * 2^attempt`, capped at `max`, with full jitter
/// over the upper half so many clients don't retry in step.
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    pub min: Duration,
    pub max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            min: Duration::from_millis(500),
            max: Duration::from_secs(30),
        }
    }
}

impl Backoff {
    pub fn delay(&self, attempt: u32) -> Duration {
        let factor = 1u32.checked_shl(attempt.min(16)).unwrap_or(u32::MAX);
        let ceiling = self.min.saturating_mul(factor).min(self.max);
        let half = ceiling / 2;
        half + rand::rng().random_range(Duration::ZERO..=half)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Connected,
    /// Waiting to retry.
    Disconnected,
    /// The server refused the password; no retries until
    /// [`Socket::reconnect_now`].
    Unauthorized,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectReason {
    ConnectFailed(String),
    PingTimeout,
    ServerClosed,
    TransportError(String),
    /// The wall clock jumped ahead of the monotonic clock: the machine slept.
    Resumed,
    Requested,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SocketEvent {
    Connecting {
        attempt: u32,
    },
    /// The namespace handshake finished. `reconnect` is false only for the
    /// first connection of this [`Socket`].
    Connected {
        reconnect: bool,
    },
    Disconnected {
        reason: DisconnectReason,
        retry_in: Option<Duration>,
    },
    Unauthorized {
        message: String,
    },
    /// A server-sent event and its first argument (`Null` if none).
    Event {
        name: String,
        data: Value,
    },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SocketError {
    #[error("invalid server URL: {0}")]
    BadUrl(String),
    #[error("not connected")]
    NotConnected,
    #[error("timed out waiting for the server")]
    Timeout,
    #[error("connection closed before the server answered")]
    Closed,
}

enum Command {
    Emit {
        name: String,
        args: Vec<Value>,
        ack: Option<oneshot::Sender<Result<Vec<Value>, SocketError>>>,
    },
    Reconnect,
    Shutdown,
}

/// Handle to the connection task. Dropping every handle stops the task.
#[derive(Clone)]
pub struct Socket {
    commands: mpsc::UnboundedSender<Command>,
    state: watch::Receiver<ConnectionState>,
    ack_timeout: Duration,
}

impl Socket {
    /// Starts the connection task on the current Tokio runtime.
    pub fn spawn(
        config: SocketConfig,
    ) -> Result<(Self, mpsc::UnboundedReceiver<SocketEvent>), SocketError> {
        let url = config.websocket_url()?;
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (events, event_rx) = mpsc::unbounded_channel();
        let (state_tx, state) = watch::channel(ConnectionState::Connecting);
        let ack_timeout = config.ack_timeout;
        tokio::spawn(
            Task {
                config,
                url,
                commands: command_rx,
                events,
                state: state_tx,
                next_ack: 0,
            }
            .run(),
        );
        Ok((
            Self {
                commands,
                state,
                ack_timeout,
            },
            event_rx,
        ))
    }

    pub fn state(&self) -> ConnectionState {
        *self.state.borrow()
    }

    pub fn watch_state(&self) -> watch::Receiver<ConnectionState> {
        self.state.clone()
    }

    /// Sends an event without waiting for an acknowledgement. Fails when
    /// not connected rather than queueing: stale typing indicators are
    /// worse than none.
    pub fn emit(&self, name: &str, data: Value) -> Result<(), SocketError> {
        if self.state() != ConnectionState::Connected {
            return Err(SocketError::NotConnected);
        }
        self.commands
            .send(Command::Emit {
                name: name.to_owned(),
                args: vec![data],
                ack: None,
            })
            .map_err(|_| SocketError::Closed)
    }

    /// Sends an event and waits for the server's acknowledgement.
    pub async fn request(&self, name: &str, data: Value) -> Result<Value, SocketError> {
        if self.state() != ConnectionState::Connected {
            return Err(SocketError::NotConnected);
        }
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(Command::Emit {
                name: name.to_owned(),
                args: vec![data],
                ack: Some(tx),
            })
            .map_err(|_| SocketError::Closed)?;
        match timeout(self.ack_timeout, rx).await {
            Ok(Ok(result)) => result.map(|args| args.into_iter().next().unwrap_or(Value::Null)),
            Ok(Err(_)) => Err(SocketError::Closed),
            Err(_) => Err(SocketError::Timeout),
        }
    }

    /// Drops the current connection (if any) and connects again at once,
    /// skipping any backoff. For network changes and changed credentials.
    pub fn reconnect_now(&self) {
        let _ = self.commands.send(Command::Reconnect);
    }

    pub fn shutdown(&self) {
        let _ = self.commands.send(Command::Shutdown);
    }
}

struct Task {
    config: SocketConfig,
    url: Url,
    commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<SocketEvent>,
    state: watch::Sender<ConnectionState>,
    next_ack: u64,
}

enum SessionEnd {
    Shutdown,
    Unauthorized(String),
    Lost {
        reason: DisconnectReason,
        /// The connection was up for at least [`HEALTHY_AFTER`].
        healthy: bool,
    },
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// How far the wall clock may run ahead of the monotonic clock between two
/// checks before we assume the machine slept.
const SUSPEND_SLACK: Duration = Duration::from_secs(10);
const CLOCK_CHECK: Duration = Duration::from_secs(3);
const HEALTHY_AFTER: Duration = Duration::from_secs(10);

impl Task {
    async fn run(mut self) {
        let mut attempt = 0u32;
        let mut connected_once = false;
        loop {
            self.set_state(ConnectionState::Connecting);
            self.send(SocketEvent::Connecting { attempt });
            let end = self.session(&mut connected_once).await;
            match end {
                SessionEnd::Shutdown => break,
                SessionEnd::Unauthorized(message) => {
                    self.set_state(ConnectionState::Unauthorized);
                    self.send(SocketEvent::Unauthorized { message });
                    attempt = 0;
                    if !self.wait_for_reconnect(None).await {
                        break;
                    }
                }
                SessionEnd::Lost { reason, healthy } => {
                    // A connection that held for a while, or a requested
                    // reconnect, retries at once; repeated failures (and a
                    // server that accepts then drops us) back off.
                    let delay = if healthy || reason == DisconnectReason::Requested {
                        attempt = 0;
                        Duration::ZERO
                    } else {
                        self.config.backoff.delay(attempt)
                    };
                    attempt = attempt.saturating_add(1);
                    self.set_state(ConnectionState::Disconnected);
                    self.send(SocketEvent::Disconnected {
                        reason,
                        retry_in: Some(delay),
                    });
                    if !delay.is_zero() && !self.wait_for_reconnect(Some(delay)).await {
                        break;
                    }
                }
            }
        }
        self.set_state(ConnectionState::Stopped);
    }

    fn send(&self, event: SocketEvent) {
        let _ = self.events.send(event);
    }

    fn set_state(&self, state: ConnectionState) {
        self.state.send_replace(state);
    }

    /// Waits out a delay (or forever), returning early on a reconnect
    /// request. Returns false on shutdown. Emits while disconnected fail.
    async fn wait_for_reconnect(&mut self, delay: Option<Duration>) -> bool {
        let deadline = delay.map(|d| Instant::now() + d);
        loop {
            let command = match deadline {
                Some(at) => tokio::select! {
                    c = self.commands.recv() => c,
                    () = sleep_until(at) => return true,
                },
                None => self.commands.recv().await,
            };
            match command {
                None | Some(Command::Shutdown) => return false,
                Some(Command::Reconnect) => return true,
                Some(Command::Emit { ack, .. }) => {
                    if let Some(ack) = ack {
                        let _ = ack.send(Err(SocketError::NotConnected));
                    }
                }
            }
        }
    }

    async fn connect(config: &SocketConfig, url: &Url) -> Result<(Ws, OpenInfo), SessionEnd> {
        let lost = |reason: String| SessionEnd::Lost {
            reason: DisconnectReason::ConnectFailed(reason),
            healthy: false,
        };
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| lost(e.to_string()))?;
        for (k, v) in &config.headers {
            if let (Ok(name), Ok(value)) = (
                http::HeaderName::from_bytes(k.as_bytes()),
                http::HeaderValue::from_str(v),
            ) {
                request.headers_mut().insert(name, value);
            }
        }
        let (mut ws, _) = match tokio_tungstenite::connect_async(request).await {
            Ok(ok) => ok,
            Err(tungstenite::Error::Http(response))
                if matches!(response.status().as_u16(), 401 | 403) =>
            {
                return Err(SessionEnd::Unauthorized(format!(
                    "server answered {}",
                    response.status()
                )));
            }
            Err(e) => return Err(lost(e.to_string())),
        };
        // Engine.IO handshake.
        let open = loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => match EnginePacket::decode(&text) {
                    Ok(EnginePacket::Open(info)) => break info,
                    Ok(_) => continue,
                    Err(e) => return Err(lost(e.to_string())),
                },
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(lost(e.to_string())),
                None => return Err(lost("closed during handshake".into())),
            }
        };
        // Socket.IO namespace handshake.
        let connect = EnginePacket::Message(SocketPacket::Connect(None).encode()).encode();
        ws.send(Message::text(connect))
            .await
            .map_err(|e| lost(e.to_string()))?;
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => match EnginePacket::decode(&text) {
                    Ok(EnginePacket::Message(payload)) => match SocketPacket::decode(&payload) {
                        Ok(a) if a.namespace == "/" => match a.packet {
                            SocketPacket::Connect(_) => return Ok((ws, open)),
                            SocketPacket::ConnectError(detail) => {
                                return Err(SessionEnd::Unauthorized(error_message(detail)));
                            }
                            _ => continue,
                        },
                        _ => continue,
                    },
                    Ok(EnginePacket::Ping(p)) => {
                        let _ = ws.send(Message::text(EnginePacket::Pong(p).encode())).await;
                    }
                    Ok(EnginePacket::Close) => return Err(lost("closed during handshake".into())),
                    _ => continue,
                },
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(lost(e.to_string())),
                None => return Err(lost("closed during handshake".into())),
            }
        }
    }

    async fn session(&mut self, connected_once: &mut bool) -> SessionEnd {
        // Commands that arrive while connecting wait in the channel, except
        // shutdown and reconnect, which `select!` sees below.
        let connect = timeout(
            self.config.connect_timeout,
            Self::connect(&self.config, &self.url),
        );
        let (mut ws, open) = tokio::select! {
            r = connect => match r {
                Ok(Ok(ok)) => ok,
                Ok(Err(end)) => return end,
                Err(_) => return SessionEnd::Lost {
                    reason: DisconnectReason::ConnectFailed("timed out".into()),
                    healthy: false,
                },
            },
            c = peek_control(&mut self.commands) => return c,
        };
        let reconnect = *connected_once;
        *connected_once = true;
        self.set_state(ConnectionState::Connected);
        self.send(SocketEvent::Connected { reconnect });

        let liveness = open.ping_interval + open.ping_timeout;
        let connected_at = Instant::now();
        let mut last_seen = connected_at;
        let mut pending: HashMap<u64, oneshot::Sender<Result<Vec<Value>, SocketError>>> =
            HashMap::new();
        let mut clock = ClockWatch::new();
        let mut clock_tick = tokio::time::interval(CLOCK_CHECK);
        clock_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        let end = loop {
            tokio::select! {
                frame = ws.next() => {
                    last_seen = Instant::now();
                    match frame {
                        Some(Ok(Message::Text(text))) => {
                            match EnginePacket::decode(&text) {
                                Ok(EnginePacket::Ping(p)) => {
                                    if let Err(e) = ws.send(Message::text(EnginePacket::Pong(p).encode())).await {
                                        break lost(DisconnectReason::TransportError(e.to_string()));
                                    }
                                }
                                Ok(EnginePacket::Message(payload)) => {
                                    if let Some(end) = self.on_message(&payload, &mut pending) {
                                        break end;
                                    }
                                }
                                Ok(EnginePacket::Close) => break lost(DisconnectReason::ServerClosed),
                                Ok(_) => {}
                                Err(e) => tracing::debug!(error = %e, "ignoring engine.io frame"),
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break lost(DisconnectReason::ServerClosed),
                        Some(Ok(_)) => {}
                        Some(Err(e)) => break lost(DisconnectReason::TransportError(e.to_string())),
                    }
                }
                command = self.commands.recv() => match command {
                    None | Some(Command::Shutdown) => {
                        let bye = EnginePacket::Message(SocketPacket::Disconnect.encode()).encode();
                        let _ = ws.send(Message::text(bye)).await;
                        let _ = ws.close(None).await;
                        break SessionEnd::Shutdown;
                    }
                    Some(Command::Reconnect) => break lost(DisconnectReason::Requested),
                    Some(Command::Emit { name, args, ack }) => {
                        let id = ack.map(|ack| {
                            let id = self.next_ack;
                            self.next_ack += 1;
                            pending.insert(id, ack);
                            id
                        });
                        let packet = SocketPacket::Event { id, name, args };
                        let frame = EnginePacket::Message(packet.encode()).encode();
                        if let Err(e) = ws.send(Message::text(frame)).await {
                            break lost(DisconnectReason::TransportError(e.to_string()));
                        }
                    }
                },
                () = sleep_until(last_seen + liveness) => break lost(DisconnectReason::PingTimeout),
                _ = clock_tick.tick() => {
                    if clock.slept() {
                        break lost(DisconnectReason::Resumed);
                    }
                }
            }
        };
        for (_, ack) in pending.drain() {
            let _ = ack.send(Err(SocketError::Closed));
        }
        match end {
            SessionEnd::Lost { reason, .. } => SessionEnd::Lost {
                reason,
                healthy: connected_at.elapsed() >= HEALTHY_AFTER,
            },
            other => other,
        }
    }

    fn on_message(
        &self,
        payload: &str,
        pending: &mut HashMap<u64, oneshot::Sender<Result<Vec<Value>, SocketError>>>,
    ) -> Option<SessionEnd> {
        let addressed = match SocketPacket::decode(payload) {
            Ok(a) => a,
            Err(e) => {
                tracing::debug!(error = %e, "ignoring socket.io packet");
                return None;
            }
        };
        if addressed.namespace != "/" {
            return None;
        }
        match addressed.packet {
            SocketPacket::Event { name, args, .. } => {
                let data = args.into_iter().next().unwrap_or(Value::Null);
                self.send(SocketEvent::Event { name, data });
            }
            SocketPacket::Ack { id, args } => {
                if let Some(ack) = pending.remove(&id) {
                    let _ = ack.send(Ok(args));
                }
            }
            SocketPacket::Disconnect => return Some(lost(DisconnectReason::ServerClosed)),
            SocketPacket::ConnectError(detail) => {
                return Some(SessionEnd::Unauthorized(error_message(detail)));
            }
            SocketPacket::Connect(_) => {}
        }
        None
    }
}

fn lost(reason: DisconnectReason) -> SessionEnd {
    SessionEnd::Lost {
        reason,
        healthy: false,
    }
}

fn error_message(detail: Option<Value>) -> String {
    detail
        .as_ref()
        .and_then(|d| d.get("message").and_then(Value::as_str).map(str::to_owned))
        .or_else(|| detail.map(|d| d.to_string()))
        .unwrap_or_else(|| "connection refused".into())
}

/// Resolves when a shutdown or reconnect command arrives while connecting.
/// Emits that arrive meanwhile are refused.
async fn peek_control(commands: &mut mpsc::UnboundedReceiver<Command>) -> SessionEnd {
    loop {
        match commands.recv().await {
            None | Some(Command::Shutdown) => return SessionEnd::Shutdown,
            Some(Command::Reconnect) => {
                return SessionEnd::Lost {
                    reason: DisconnectReason::Requested,
                    healthy: false,
                };
            }
            Some(Command::Emit { ack, .. }) => {
                if let Some(ack) = ack {
                    let _ = ack.send(Err(SocketError::NotConnected));
                }
            }
        }
    }
}

/// Detects suspend: Linux's monotonic clock stops while asleep and the
/// wall clock doesn't.
struct ClockWatch {
    wall: SystemTime,
    mono: std::time::Instant,
}

impl ClockWatch {
    fn new() -> Self {
        Self {
            wall: SystemTime::now(),
            mono: std::time::Instant::now(),
        }
    }

    fn slept(&mut self) -> bool {
        let (wall, mono) = (SystemTime::now(), std::time::Instant::now());
        let wall_delta = wall.duration_since(self.wall).unwrap_or_default();
        let mono_delta = mono.duration_since(self.mono);
        self.wall = wall;
        self.mono = mono;
        wall_delta > mono_delta + SUSPEND_SLACK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_websocket_url() {
        let mut c = SocketConfig::new(Url::parse("https://example.com:1234/base/").unwrap());
        c.query.push(("guid".into(), "p&ss".into()));
        assert_eq!(
            c.websocket_url().unwrap().as_str(),
            "wss://example.com:1234/base/socket.io/?EIO=4&transport=websocket&guid=p%26ss"
        );
        let c = SocketConfig::new(Url::parse("http://10.0.0.2:1234").unwrap());
        assert_eq!(
            c.websocket_url().unwrap().as_str(),
            "ws://10.0.0.2:1234/socket.io/?EIO=4&transport=websocket"
        );
    }

    #[test]
    fn backoff_grows_and_caps() {
        let b = Backoff {
            min: Duration::from_millis(100),
            max: Duration::from_secs(2),
        };
        for attempt in 0..40 {
            let d = b.delay(attempt);
            let ceiling = (Duration::from_millis(100) * 2u32.pow(attempt.min(16))).min(b.max);
            assert!(d >= ceiling / 2 && d <= ceiling, "attempt {attempt}: {d:?}");
        }
    }
}
