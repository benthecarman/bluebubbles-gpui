//! The client against a scripted Engine.IO server.

use bb_socket::{ConnectionState, DisconnectReason, Socket, SocketConfig, SocketEvent};
use futures::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

#[derive(Clone, Copy)]
enum Behaviour {
    /// Handshake, push one event, answer acks, then drop the connection
    /// after the first ack on the first connection only.
    Normal,
    /// Handshake with a short ping interval, then never ping.
    Silent,
    /// Refuse the namespace connect.
    Refuse,
}

struct Server {
    url: url::Url,
    connections: Arc<AtomicUsize>,
    queries: mpsc::UnboundedReceiver<String>,
}

async fn serve(behaviour: Behaviour) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let (query_tx, queries) = mpsc::unbounded_channel();
    let count = connections.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let n = count.fetch_add(1, Ordering::SeqCst);
            let query_tx = query_tx.clone();
            tokio::spawn(async move {
                let mut ws =
                    tokio_tungstenite::accept_hdr_async(stream, |req: &Request, resp: Response| {
                        let _ = query_tx.send(req.uri().query().unwrap_or_default().to_owned());
                        Ok(resp)
                    })
                    .await
                    .unwrap();
                let ping = match behaviour {
                    Behaviour::Silent => 100,
                    _ => 25_000,
                };
                ws.send(Message::text(format!(
                    r#"0{{"sid":"s{n}","upgrades":[],"pingInterval":{ping},"pingTimeout":100,"maxPayload":1000000}}"#
                )))
                .await
                .unwrap();
                // Wait for the namespace connect.
                while let Some(Ok(Message::Text(t))) = ws.next().await {
                    if t.as_str() == "40" {
                        break;
                    }
                }
                match behaviour {
                    Behaviour::Refuse => {
                        let _ = ws
                            .send(Message::text(r#"44{"message":"Unauthorized"}"#))
                            .await;
                        return;
                    }
                    Behaviour::Silent => {
                        ws.send(Message::text(r#"40{"sid":"x"}"#)).await.unwrap();
                        // Hold the socket open without pinging.
                        while ws.next().await.is_some() {}
                        return;
                    }
                    Behaviour::Normal => {}
                }
                ws.send(Message::text(r#"40{"sid":"x"}"#)).await.unwrap();
                ws.send(Message::text(format!(
                    r#"42["new-message",{{"guid":"m{n}"}}]"#
                )))
                .await
                .unwrap();
                ws.send(Message::text("2")).await.unwrap();
                while let Some(Ok(msg)) = ws.next().await {
                    let Message::Text(t) = msg else { continue };
                    let t = t.as_str();
                    if t == "3" {
                        continue;
                    }
                    // `42<id>["name",data]` → `43<id>[{"echo":data}]`
                    if let Some(rest) = t.strip_prefix("42") {
                        let digits: String =
                            rest.chars().take_while(char::is_ascii_digit).collect();
                        if digits.is_empty() {
                            continue;
                        }
                        let args: serde_json::Value =
                            serde_json::from_str(&rest[digits.len()..]).unwrap();
                        let reply = json!([{ "echo": args[1] }]);
                        ws.send(Message::text(format!("43{digits}{reply}")))
                            .await
                            .unwrap();
                        if n == 0 {
                            let _ = ws.close(None).await;
                            return;
                        }
                    }
                }
            });
        }
    });
    let mut url = url::Url::parse(&format!("http://{addr}")).unwrap();
    url.set_path("/");
    Server {
        url,
        connections,
        queries,
    }
}

async fn next(events: &mut mpsc::UnboundedReceiver<SocketEvent>) -> SocketEvent {
    timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("an event in time")
        .expect("the task is running")
}

async fn until<F: Fn(&SocketEvent) -> bool>(
    events: &mut mpsc::UnboundedReceiver<SocketEvent>,
    want: F,
) -> SocketEvent {
    loop {
        let e = next(events).await;
        if want(&e) {
            return e;
        }
    }
}

fn config(server: &Server) -> SocketConfig {
    let mut c = SocketConfig::new(server.url.clone());
    c.query.push(("guid".into(), "secret".into()));
    c.backoff.min = Duration::from_millis(20);
    c.backoff.max = Duration::from_millis(100);
    c
}

#[tokio::test]
async fn receives_events_answers_acks_and_reconnects() {
    let mut server = serve(Behaviour::Normal).await;
    let (socket, mut events) = Socket::spawn(config(&server)).unwrap();

    assert_eq!(
        next(&mut events).await,
        SocketEvent::Connecting { attempt: 0 }
    );
    assert_eq!(
        next(&mut events).await,
        SocketEvent::Connected { reconnect: false }
    );
    let q = server.queries.recv().await.unwrap();
    assert!(q.contains("EIO=4") && q.contains("transport=websocket") && q.contains("guid=secret"));
    assert_eq!(
        next(&mut events).await,
        SocketEvent::Event {
            name: "new-message".into(),
            data: json!({"guid":"m0"})
        }
    );
    assert_eq!(socket.state(), ConnectionState::Connected);

    // The server answers, then drops this first connection.
    let answer = socket.request("get-thing", json!({"a":1})).await.unwrap();
    assert_eq!(answer, json!({"echo":{"a":1}}));
    let e = until(&mut events, |e| {
        matches!(e, SocketEvent::Disconnected { .. })
    })
    .await;
    assert!(matches!(
        e,
        SocketEvent::Disconnected {
            reason: DisconnectReason::ServerClosed,
            ..
        }
    ));
    assert_eq!(
        until(&mut events, |e| matches!(e, SocketEvent::Connected { .. })).await,
        SocketEvent::Connected { reconnect: true }
    );
    assert_eq!(
        next(&mut events).await,
        SocketEvent::Event {
            name: "new-message".into(),
            data: json!({"guid":"m1"})
        }
    );
    assert_eq!(server.connections.load(Ordering::SeqCst), 2);

    socket.shutdown();
    let mut state = socket.watch_state();
    timeout(
        Duration::from_secs(5),
        state.wait_for(|s| *s == ConnectionState::Stopped),
    )
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn silent_server_times_out_and_is_retried() {
    let server = serve(Behaviour::Silent).await;
    let (_socket, mut events) = Socket::spawn(config(&server)).unwrap();
    until(&mut events, |e| matches!(e, SocketEvent::Connected { .. })).await;
    let e = until(&mut events, |e| {
        matches!(e, SocketEvent::Disconnected { .. })
    })
    .await;
    assert!(matches!(
        e,
        SocketEvent::Disconnected {
            reason: DisconnectReason::PingTimeout,
            ..
        }
    ));
    assert_eq!(
        until(&mut events, |e| matches!(e, SocketEvent::Connected { .. })).await,
        SocketEvent::Connected { reconnect: true }
    );
}

#[tokio::test]
async fn refused_password_stops_until_asked() {
    let server = serve(Behaviour::Refuse).await;
    let (socket, mut events) = Socket::spawn(config(&server)).unwrap();
    let e = until(&mut events, |e| {
        matches!(e, SocketEvent::Unauthorized { .. })
    })
    .await;
    assert_eq!(
        e,
        SocketEvent::Unauthorized {
            message: "Unauthorized".into()
        }
    );
    assert_eq!(socket.state(), ConnectionState::Unauthorized);
    // No retries on its own.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(server.connections.load(Ordering::SeqCst), 1);
    assert!(socket.request("x", json!(null)).await.is_err());

    socket.reconnect_now();
    until(&mut events, |e| {
        matches!(e, SocketEvent::Unauthorized { .. })
    })
    .await;
    assert_eq!(server.connections.load(Ordering::SeqCst), 2);
}
