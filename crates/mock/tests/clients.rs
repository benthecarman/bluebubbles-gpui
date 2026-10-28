//! The REST and socket clients against the mock server.

use bb_api::{
    ApiError, Client, Download, MessageQuery, SendText, ServerConfig, WhereClause, socket_password,
};
use bb_mock::{MockServer, SendBehaviour, seed_demo};
use bb_socket::{Socket, SocketConfig, SocketEvent};
use serde_json::json;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use url::Url;

const PASSWORD: &str = "p@ss%20word#1";

fn client(server: &MockServer, password: &str) -> Client {
    let url = Url::parse(&server.url()).unwrap();
    Client::new(&ServerConfig::new(url, password)).unwrap()
}

fn socket(server: &MockServer, password: &str) -> (Socket, UnboundedReceiver<SocketEvent>) {
    let mut config = SocketConfig::new(Url::parse(&server.url()).unwrap());
    config
        .query
        .push(("password".into(), socket_password(password)));
    config.backoff.min = Duration::from_millis(20);
    config.backoff.max = Duration::from_millis(100);
    Socket::spawn(config).unwrap()
}

/// The next event that isn't a connection-state change.
async fn next_message(events: &mut UnboundedReceiver<SocketEvent>) -> (String, serde_json::Value) {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Some(SocketEvent::Event { name, data })) => return (name, data),
            Ok(Some(_)) => continue,
            other => panic!("no event: {other:?}"),
        }
    }
}

async fn wait_connected(events: &mut UnboundedReceiver<SocketEvent>) -> bool {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Some(SocketEvent::Connected { reconnect })) => return reconnect,
            Ok(Some(SocketEvent::Unauthorized { message })) => panic!("unauthorized: {message}"),
            Ok(Some(_)) => continue,
            other => panic!("never connected: {other:?}"),
        }
    }
}

#[tokio::test]
async fn demo_data_over_rest() {
    let server = MockServer::start(PASSWORD).await;
    seed_demo(&server);
    let api = client(&server, PASSWORD);

    api.ping().await.unwrap();
    assert!(api.server_info().await.unwrap().private_api_ready());

    let chats = api.chats(0, 100).await.unwrap();
    assert_eq!(chats.total, Some(6));
    assert_eq!(
        chats.items[0].guid,
        format!("iMessage;-;{}", bb_mock::demo::MOM)
    );
    let trip = chats
        .items
        .iter()
        .find(|c| c.guid == "iMessage;+;chat100200300")
        .unwrap();
    assert!(trip.is_group());
    assert_eq!(trip.participants.as_ref().unwrap().len(), 3);

    let page = api.chat_messages(&trip.guid, None, 50).await.unwrap();
    let dates: Vec<_> = page.iter().map(|m| m.date_created.unwrap()).collect();
    assert!(dates.windows(2).all(|w| w[0] >= w[1]), "newest first");
    assert!(
        page.iter()
            .any(|m| m.associated_message_type.as_deref() == Some("love"))
    );
    assert!(page.iter().any(|m| m.item_type == Some(2)));

    let photo = page.iter().flat_map(|m| &m.attachments).next().unwrap();
    let bytes = api
        .download_attachment(&photo.guid, Download::default())
        .await
        .unwrap();
    assert_eq!(bytes.len() as i64, photo.total_bytes.unwrap());

    assert_eq!(api.contacts(false).await.unwrap().len(), 5);
    assert_eq!(api.chat_icon(&trip.guid).await.unwrap(), None);
}

#[tokio::test]
async fn wrong_password_is_unauthorized() {
    let server = MockServer::start(PASSWORD).await;
    let api = client(&server, "nope");
    assert!(matches!(api.ping().await, Err(ApiError::Unauthorized)));
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn catch_up_by_rowid_and_reconcile_by_date() {
    let server = MockServer::start(PASSWORD).await;
    server.add_chat("iMessage;-;a@example.com", None, &["a@example.com"]);
    let first = server.receive("iMessage;-;a@example.com", "a@example.com", "one");
    let cursor = first.original_rowid.unwrap();
    let offline = server.insert_silently(
        "iMessage;-;a@example.com",
        Some("a@example.com"),
        "two",
        bb_mock::now_ms(),
    );
    let api = client(&server, PASSWORD);

    let mut query = MessageQuery::new(100);
    query.r#where.push(WhereClause {
        statement: "message.ROWID > :rowid".into(),
        args: json!({ "rowid": cursor }),
    });
    let page = api.query_messages(&query).await.unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].guid, offline.guid);

    // The first message is read later, without an event reaching us.
    let since = bb_mock::now_ms() - 1;
    std::thread::sleep(Duration::from_millis(5));
    server.update(&first.guid, true, |m| m.date_read = Some(bb_mock::now_ms()));
    let apple_ns = (since - bb_mock::APPLE_EPOCH_MS) * 1_000_000;
    let mut query = MessageQuery::new(100);
    query.r#where.push(WhereClause {
        statement: "message.date_read >= :since".into(),
        args: json!({ "since": apple_ns }),
    });
    let page = api.query_messages(&query).await.unwrap();
    assert_eq!(
        page.items.iter().map(|m| &m.guid).collect::<Vec<_>>(),
        [&first.guid]
    );
}

#[tokio::test]
async fn send_echoes_twice_on_the_socket() {
    let server = MockServer::start(PASSWORD).await;
    server.add_chat("iMessage;-;a@example.com", None, &["a@example.com"]);
    let (_socket, mut events) = socket(&server, PASSWORD);
    assert!(!wait_connected(&mut events).await);
    let api = client(&server, PASSWORD);

    let sent = api
        .send_text(&SendText {
            chat_guid: "iMessage;-;a@example.com".into(),
            temp_guid: "temp-1".into(),
            message: "hello".into(),
            method: "private-api".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(sent.is_from_me);
    assert_eq!(sent.temp_guid.as_deref(), Some("temp-1"));

    let (name, slim) = next_message(&mut events).await;
    assert_eq!(name, "new-message");
    assert_eq!(slim["tempGuid"], "temp-1");
    let (name, full) = next_message(&mut events).await;
    assert_eq!(name, "new-message");
    assert_eq!(full["guid"], sent.guid.as_str());
    assert!(full.get("tempGuid").is_none());
}

#[tokio::test]
async fn failed_send_returns_the_message() {
    let server = MockServer::start(PASSWORD).await;
    server.add_chat("SMS;-;+15550100000", None, &["+15550100000"]);
    server.set_send_behaviour(SendBehaviour {
        fail_with: Some(22),
        ..Default::default()
    });
    let api = client(&server, PASSWORD);
    let sent = api
        .send_text(&SendText {
            chat_guid: "SMS;-;+15550100000".into(),
            temp_guid: "temp-2".into(),
            message: "will fail".into(),
            method: "apple-script".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(sent.error, Some(22));
}

#[tokio::test]
async fn socket_password_with_percent_signs() {
    let server = MockServer::start(PASSWORD).await;
    let (_socket, mut events) = socket(&server, PASSWORD);
    wait_connected(&mut events).await;
    assert_eq!(server.connected_sockets(), 1);

    server.typing("iMessage;-;a@example.com", true);
    let (name, data) = next_message(&mut events).await;
    assert_eq!(name, "typing-indicator");
    assert_eq!(data["display"], true);
}

#[tokio::test]
async fn socket_wrong_password_stops() {
    let server = MockServer::start(PASSWORD).await;
    let (socket, mut events) = socket(&server, "p@ss word#1");
    loop {
        match tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
        {
            Some(SocketEvent::Unauthorized { .. }) => break,
            Some(SocketEvent::Connected { .. }) => panic!("connected with a bad password"),
            Some(_) => continue,
            None => panic!("socket task ended"),
        }
    }
    assert_eq!(socket.state(), bb_socket::ConnectionState::Unauthorized);
}

#[tokio::test]
async fn socket_reconnects_after_a_drop() {
    let server = MockServer::start(PASSWORD).await;
    let (_socket, mut events) = socket(&server, PASSWORD);
    assert!(!wait_connected(&mut events).await);
    server.drop_sockets();
    assert!(wait_connected(&mut events).await);
}
