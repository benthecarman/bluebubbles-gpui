//! Engine.IO 4 over WebSocket, server side: enough of the protocol for
//! one namespace, pings and acks.

use crate::{Outgoing, Shared};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::time::Duration;

pub(crate) async fn upgrade(
    State(shared): State<Shared>,
    Query(q): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    // The server runs `decodeURI` on the already-decoded query value.
    let given = q
        .get("password")
        .or(q.get("guid"))
        .and_then(|p| crate::decode_uri(p));
    if given.as_ref() != Some(&shared.lock().password) {
        // The real server refuses the Engine.IO handshake itself.
        return (StatusCode::FORBIDDEN, "Authentication failed").into_response();
    }
    ws.on_upgrade(move |socket| session(shared, socket))
}

async fn session(shared: Shared, socket: WebSocket) {
    let (interval, timeout) = {
        let s = shared.lock();
        (s.ping_interval_ms, s.ping_timeout_ms)
    };
    let mut events = shared.events.subscribe();
    let (mut tx, mut rx) = socket.split();
    let open = format!(
        r#"0{{"sid":"mock","upgrades":[],"pingInterval":{interval},"pingTimeout":{timeout},"maxPayload":100000000}}"#
    );
    if tx.send(Message::text(open)).await.is_err() {
        return;
    }
    // The namespace connect.
    loop {
        match rx.next().await {
            Some(Ok(Message::Text(t))) if t.as_str().starts_with("40") => break,
            Some(Ok(_)) => continue,
            _ => return,
        }
    }
    if tx
        .send(Message::text(r#"40{"sid":"mock-ns"}"#))
        .await
        .is_err()
    {
        return;
    }
    shared.lock().sockets += 1;
    let mut ping = tokio::time::interval(Duration::from_millis(interval.max(1)));
    ping.tick().await;
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(Outgoing::Event(name, data)) => {
                    let frame = format!("42{}", serde_json::json!([name, data]));
                    if tx.send(Message::text(frame)).await.is_err() {
                        break;
                    }
                }
                Ok(Outgoing::Drop) | Err(_) => break,
            },
            frame = rx.next() => match frame {
                Some(Ok(Message::Text(t))) => {
                    let t = t.as_str();
                    // Acknowledge requests: `42<id>[...]` → `43<id>[envelope]`.
                    if let Some(rest) = t.strip_prefix("42") {
                        let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
                        if !id.is_empty() {
                            let reply = format!(r#"43{id}[{{"status":200,"message":"Success","data":null,"encrypted":false}}]"#);
                            if tx.send(Message::text(reply)).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            _ = ping.tick() => {
                if tx.send(Message::text("2")).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = tx.close().await;
    let mut s = shared.lock();
    s.sockets = s.sockets.saturating_sub(1);
}
