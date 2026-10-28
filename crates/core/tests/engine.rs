//! The engine against the mock server.

use bb_api::ServerConfig;
use bb_core::{
    Connection, Engine, EngineConfig, EngineEvent, Names, Region, Tapback, TextMessage, Timings,
};
use bb_mock::{MockServer, SendBehaviour, demo, seed_demo};
use bb_socket::Backoff;
use bb_store::Store;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use url::Url;

const PASSWORD: &str = "test%pass";
const WAIT: Duration = Duration::from_secs(10);

fn timings() -> Timings {
    Timings {
        tick: Duration::from_millis(20),
        typing_timeout: Duration::from_millis(400),
        composing_idle: Duration::from_millis(200),
        read_debounce: Duration::from_millis(50),
        catch_up_every: Duration::from_secs(600),
        offline_poll: Duration::from_millis(200),
        focus_debounce: Duration::ZERO,
        send_give_up: Duration::from_secs(600),
        contacts_every: Duration::from_secs(600),
        notify_window: Duration::from_secs(900),
        socket_backoff: Backoff {
            min: Duration::from_millis(20),
            max: Duration::from_millis(100),
        },
    }
}

fn start(
    server: &MockServer,
    store: &Store,
    password: &str,
) -> (Engine, UnboundedReceiver<EngineEvent>) {
    let url = Url::parse(&server.url()).unwrap();
    let mut config = EngineConfig::new(ServerConfig::new(url, password), store.clone());
    config.region = Region::new("US");
    config.timings = timings();
    Engine::spawn(config).unwrap()
}

async fn synced(server: &MockServer) -> (Engine, UnboundedReceiver<EngineEvent>, Store) {
    let store = Store::open_in_memory().unwrap();
    let (engine, mut events) = start(server, &store, PASSWORD);
    wait_event(&mut events, |e| {
        matches!(e, EngineEvent::Status(s) if s.synced && !s.syncing && s.connection == Connection::Online)
    })
    .await;
    (engine, events, store)
}

async fn wait_event(
    events: &mut UnboundedReceiver<EngineEvent>,
    mut f: impl FnMut(&EngineEvent) -> bool,
) -> EngineEvent {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(e)) if f(&e) => return e,
            Ok(Some(_)) => continue,
            Ok(None) => panic!("engine stopped"),
            Err(_) => panic!("timed out waiting for an event"),
        }
    }
}

async fn wait_until(mut f: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !f() {
        assert!(tokio::time::Instant::now() < deadline, "timed out");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Events that arrive within `d`.
async fn drain(events: &mut UnboundedReceiver<EngineEvent>, d: Duration) -> Vec<EngineEvent> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + d;
    while let Ok(Some(e)) = tokio::time::timeout_at(deadline, events.recv()).await {
        out.push(e);
    }
    out
}

fn dm(address: &str) -> String {
    format!("iMessage;-;{address}")
}

#[tokio::test]
async fn first_sync_fills_the_store_quietly() {
    let server = MockServer::start(PASSWORD).await;
    seed_demo(&server);
    let store = Store::open_in_memory().unwrap();
    let (engine, mut events) = start(&server, &store, PASSWORD);
    let mut notified = false;
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        match tokio::time::timeout_at(deadline, events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            EngineEvent::Notify(_) => notified = true,
            EngineEvent::Status(s) if s.synced && !s.syncing => break,
            _ => {}
        }
    }
    assert!(!notified, "the first sync must not notify");
    assert!(engine.status().features.private_api);

    let chats = store.chats(false).unwrap();
    assert_eq!(chats.len(), 6);
    assert_eq!(chats[0].guid, dm(demo::MOM));
    let names = Names::for_chats(&store, Region::new("US"), &chats).unwrap();
    let titles: Vec<String> = chats.iter().map(|c| names.chat_title(c)).collect();
    assert!(titles.contains(&"Mom".to_owned()), "{titles:?}");
    assert!(titles.contains(&"Weekend Trip 🏕️".to_owned()));
    assert!(titles.contains(&"Priya & Sam".to_owned()));
    assert!(titles.contains(&"(555) 010-9999".to_owned()));

    // The top chats' recent history was prefetched, tapbacks included.
    let trip = store
        .chat_by_guid("iMessage;+;chat100200300")
        .unwrap()
        .unwrap();
    let messages = store.messages_before(trip.id, None, 50).unwrap();
    assert_eq!(messages.len(), 9, "8 messages and a rename");
    let target = messages
        .iter()
        .find(|m| m.text.as_deref() == Some("That's amazing, can't wait"))
        .unwrap();
    let reactions = bb_core::tapback::reactions(
        &store
            .associated(std::slice::from_ref(&target.guid))
            .unwrap(),
        0,
    );
    assert_eq!(reactions.len(), 2);
    assert!(messages.iter().any(|m| !m.attachments.is_empty()));
    assert_eq!(store.contact_count().unwrap(), 5);
}

#[tokio::test]
async fn live_messages_notify_once() {
    let server = MockServer::start(PASSWORD).await;
    server.add_chat(&dm(demo::ALEX), None, &[demo::ALEX]);
    server.add_contact("Alex Kim", &[], &[demo::ALEX]);
    let (_engine, mut events, store) = synced(&server).await;

    let m = server.receive(&dm(demo::ALEX), demo::ALEX, "are you around?");
    let EngineEvent::Notify(n) =
        wait_event(&mut events, |e| matches!(e, EngineEvent::Notify(_))).await
    else {
        unreachable!()
    };
    assert_eq!(n.title, "Alex Kim");
    assert_eq!(n.body, "are you around?");
    assert_eq!(n.subtitle, None);
    let chat = store.chat_by_guid(&dm(demo::ALEX)).unwrap().unwrap();
    assert!(chat.unread);

    // The server re-sends it (as it does when a message changes).
    server.emit("new-message", serde_json::to_value(&m).unwrap());
    let later = drain(&mut events, Duration::from_millis(300)).await;
    assert!(!later.iter().any(|e| matches!(e, EngineEvent::Notify(_))));
}

#[tokio::test]
async fn the_open_chat_is_read_and_quiet() {
    let server = MockServer::start(PASSWORD).await;
    server.add_chat(&dm(demo::ALEX), None, &[demo::ALEX]);
    server.receive(&dm(demo::ALEX), demo::ALEX, "first");
    let (engine, mut events, store) = synced(&server).await;
    let chat = store.chat_by_guid(&dm(demo::ALEX)).unwrap().unwrap();
    assert!(chat.unread);

    engine.view(Some(chat.id), true);
    wait_event(
        &mut events,
        |e| matches!(e, EngineEvent::ChatRead { chat_id } if *chat_id == chat.id),
    )
    .await;
    assert!(!store.chat(chat.id).unwrap().unwrap().unread);

    server.receive(&dm(demo::ALEX), demo::ALEX, "second");
    let later = drain(&mut events, Duration::from_millis(300)).await;
    assert!(!later.iter().any(|e| matches!(e, EngineEvent::Notify(_))));
    assert!(!store.chat(chat.id).unwrap().unwrap().unread);
    let read_path = format!("POST /api/v1/chat/{}/read", dm(demo::ALEX));
    wait_until(|| server.requests().contains(&read_path)).await;
}

async fn send_and_settle(full_echo_first: bool, lose_reply: bool) {
    let server = MockServer::start(PASSWORD).await;
    server.add_chat(&dm(demo::ALEX), None, &[demo::ALEX]);
    let (engine, mut events, store) = synced(&server).await;
    server.set_send_behaviour(SendBehaviour {
        full_echo_first,
        lose_reply,
        ..Default::default()
    });
    engine.send_text(TextMessage {
        chat_guid: dm(demo::ALEX),
        text: "on my way".into(),
        ..Default::default()
    });
    let chat = loop {
        if let Some(c) = store.chat_by_guid(&dm(demo::ALEX)).unwrap() {
            break c;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    wait_until(|| {
        let rows = store.messages_before(chat.id, None, 10).unwrap();
        rows.len() == 1 && !rows[0].is_pending() && !rows[0].guid.starts_with("temp-")
    })
    .await;
    // Let every echo land, then check nothing doubled.
    drain(&mut events, Duration::from_millis(300)).await;
    let rows = store.messages_before(chat.id, None, 10).unwrap();
    assert_eq!(rows.len(), 1, "{rows:#?}");
    assert!(!rows[0].is_failed());
    assert_eq!(rows[0].text.as_deref(), Some("on my way"));
    assert!(store.outbox().unwrap().is_empty());
    assert_eq!(server.messages().len(), 1);
}

#[tokio::test]
async fn send_resolves_with_slim_echo_first() {
    send_and_settle(false, false).await;
}

#[tokio::test]
async fn send_resolves_with_full_echo_first() {
    send_and_settle(true, false).await;
}

#[tokio::test]
async fn send_with_a_lost_reply_is_not_failed() {
    send_and_settle(false, true).await;
}

#[tokio::test]
async fn refused_send_is_failed_and_retry_sends_a_copy() {
    let server = MockServer::start(PASSWORD).await;
    server.add_chat("SMS;-;+15550100000", None, &["+15550100000"]);
    let (engine, _events, store) = synced(&server).await;
    server.set_send_behaviour(SendBehaviour {
        fail_with: Some(22),
        ..Default::default()
    });
    engine.send_text(TextMessage {
        chat_guid: "SMS;-;+15550100000".into(),
        text: "hello?".into(),
        ..Default::default()
    });
    wait_until(|| {
        store
            .chat_by_guid("SMS;-;+15550100000")
            .unwrap()
            .and_then(|c| c.last_message)
            .is_some_and(|m| m.is_failed())
    })
    .await;
    let failed = store
        .chat_by_guid("SMS;-;+15550100000")
        .unwrap()
        .unwrap()
        .last_message
        .unwrap();
    engine.retry(failed.id);
    wait_until(|| server.messages().len() == 2 && server.messages()[1].error == Some(0)).await;
}

#[tokio::test]
async fn catch_up_after_a_dropped_socket() {
    let server = MockServer::start(PASSWORD).await;
    server.add_chat(&dm(demo::ALEX), None, &[demo::ALEX]);
    let (_engine, mut events, store) = synced(&server).await;

    // Offline: the socket drops and a message and a receipt arrive with no
    // event to carry them.
    server.drop_sockets();
    let sent = server.insert_silently(
        &dm(demo::ALEX),
        None,
        "did you get this?",
        bb_mock::now_ms() - 5_000,
    );
    let missed = server.insert_silently(
        &dm(demo::ALEX),
        Some(demo::ALEX),
        "missed you",
        bb_mock::now_ms(),
    );
    let n = wait_event(&mut events, |e| matches!(e, EngineEvent::Notify(_))).await;
    let EngineEvent::Notify(n) = n else {
        unreachable!()
    };
    assert_eq!(n.message_guid, missed.guid);
    assert!(store.message_by_guid(&sent.guid).unwrap().is_some());

    server.update(&sent.guid, true, |m| m.date_read = Some(bb_mock::now_ms()));
    _engine.sync_now();
    wait_until(|| {
        store
            .message_by_guid(&sent.guid)
            .unwrap()
            .is_some_and(|m| m.date_read.is_some())
    })
    .await;
}

#[tokio::test]
async fn typing_bubbles_come_and_go() {
    let server = MockServer::start(PASSWORD).await;
    server.add_chat(&dm(demo::ALEX), None, &[demo::ALEX]);
    let (_engine, mut events, store) = synced(&server).await;
    let chat = store.chat_by_guid(&dm(demo::ALEX)).unwrap().unwrap();

    server.typing(&dm(demo::ALEX), true);
    wait_event(
        &mut events,
        |e| matches!(e, EngineEvent::Typing { chat_id, typing: true } if *chat_id == chat.id),
    )
    .await;
    // A message ends it.
    server.receive(&dm(demo::ALEX), demo::ALEX, "hi");
    wait_event(&mut events, |e| {
        matches!(e, EngineEvent::Typing { typing: false, .. })
    })
    .await;

    // So does silence.
    server.typing(&dm(demo::ALEX), true);
    wait_event(&mut events, |e| {
        matches!(e, EngineEvent::Typing { typing: true, .. })
    })
    .await;
    let started = tokio::time::Instant::now();
    wait_event(&mut events, |e| {
        matches!(e, EngineEvent::Typing { typing: false, .. })
    })
    .await;
    assert!(started.elapsed() >= Duration::from_millis(300));
}

#[tokio::test]
async fn composing_sends_typing_and_stops_when_idle() {
    let server = MockServer::start(PASSWORD).await;
    server.add_chat(&dm(demo::ALEX), None, &[demo::ALEX]);
    let (engine, _events, _store) = synced(&server).await;
    let path = format!("/api/v1/chat/{}/typing", dm(demo::ALEX));
    engine.composing(&dm(demo::ALEX), true);
    engine.composing(&dm(demo::ALEX), true);
    wait_until(|| server.requests().contains(&format!("DELETE {path}"))).await;
    let posts = server
        .requests()
        .iter()
        .filter(|r| **r == format!("POST {path}"))
        .count();
    assert_eq!(posts, 1);
}

#[tokio::test]
async fn wrong_password_stops_at_unauthorized() {
    let server = MockServer::start(PASSWORD).await;
    let store = Store::open_in_memory().unwrap();
    let (engine, mut events) = start(&server, &store, "wrong");
    wait_event(
        &mut events,
        |e| matches!(e, EngineEvent::Status(s) if s.connection == Connection::Unauthorized),
    )
    .await;
    assert!(!engine.status().synced);
}

#[tokio::test]
async fn tapbacks_resolve_and_aggregate() {
    let server = MockServer::start(PASSWORD).await;
    server.add_chat(&dm(demo::ALEX), None, &[demo::ALEX]);
    let target = server.receive(&dm(demo::ALEX), demo::ALEX, "pizza tonight?");
    let (engine, _events, store) = synced(&server).await;
    engine.react(&dm(demo::ALEX), &target.guid, 0, Tapback::Love, false);
    wait_until(|| {
        let rows = store
            .associated(std::slice::from_ref(&target.guid))
            .unwrap();
        rows.len() == 1 && !rows[0].guid.starts_with("temp-")
    })
    .await;
    let rows = store
        .associated(std::slice::from_ref(&target.guid))
        .unwrap();
    let reactions = bb_core::tapback::reactions(&rows, 0);
    assert_eq!(bb_core::tapback::mine(&reactions), Some(&Tapback::Love));
}

#[tokio::test]
async fn interrupted_send_is_not_sent_twice() {
    let server = MockServer::start(PASSWORD).await;
    server.add_chat(&dm(demo::ALEX), None, &[demo::ALEX]);
    let (engine, _events, store) = synced(&server).await;
    server.set_send_behaviour(SendBehaviour {
        delay: Duration::from_secs(30),
        ..Default::default()
    });
    engine.send_text(TextMessage {
        chat_guid: dm(demo::ALEX),
        text: "sent before the crash".into(),
        ..Default::default()
    });
    // The server has it, but the app quits before the reply.
    wait_until(|| server.messages().len() == 1).await;
    engine.shutdown();
    drop(engine);
    tokio::time::sleep(Duration::from_millis(100)).await;
    server.set_send_behaviour(SendBehaviour::default());

    let (_engine, _events) = start(&server, &store, PASSWORD);
    wait_until(|| {
        store.outbox().unwrap().is_empty()
            && store
                .chat_by_guid(&dm(demo::ALEX))
                .unwrap()
                .and_then(|c| c.last_message)
                .is_some_and(|m| !m.is_pending())
    })
    .await;
    assert_eq!(server.messages().len(), 1);
}

#[tokio::test]
async fn history_pages_until_done() {
    let server = MockServer::start(PASSWORD).await;
    server.add_chat(&dm(demo::ALEX), None, &[demo::ALEX]);
    let now = bb_mock::now_ms();
    for i in 0..120 {
        server.insert_silently(
            &dm(demo::ALEX),
            Some(demo::ALEX),
            &format!("m{i}"),
            now - (120 - i) * 60_000,
        );
    }
    let (engine, mut events, store) = synced(&server).await;
    let chat = store.chat_by_guid(&dm(demo::ALEX)).unwrap().unwrap();
    assert_eq!(store.messages_before(chat.id, None, 500).unwrap().len(), 25);
    loop {
        engine.load_older(chat.id);
        wait_event(
            &mut events,
            |e| matches!(e, EngineEvent::Changed(c) if c.chats.contains(&chat.id)),
        )
        .await;
        if store.chat(chat.id).unwrap().unwrap().history_done {
            break;
        }
    }
    assert_eq!(
        store.messages_before(chat.id, None, 500).unwrap().len(),
        120
    );
}

#[tokio::test]
async fn an_old_receipt_does_not_leave_a_gap_in_history() {
    let server = MockServer::start(PASSWORD).await;
    server.add_chat(&dm(demo::ALEX), None, &[demo::ALEX]);
    let now = bb_mock::now_ms();
    let guids: Vec<String> = (0..120)
        .map(|i| {
            server
                .insert_silently(
                    &dm(demo::ALEX),
                    None,
                    &format!("m{i}"),
                    now - (120 - i) * 60_000,
                )
                .guid
        })
        .collect();
    let (engine, mut events, store) = synced(&server).await;
    let chat = store.chat_by_guid(&dm(demo::ALEX)).unwrap().unwrap();

    // A receipt on a message older than anything cached comes back from
    // reconcile; history must still page in without skipping past it.
    server.update(&guids[3], true, |m| m.date_read = Some(bb_mock::now_ms()));
    engine.sync_now();
    wait_event(
        &mut events,
        |e| matches!(e, EngineEvent::Status(s) if s.syncing),
    )
    .await;
    wait_event(
        &mut events,
        |e| matches!(e, EngineEvent::Status(s) if !s.syncing),
    )
    .await;
    assert!(store.message_by_guid(&guids[3]).unwrap().is_none());

    while !store.chat(chat.id).unwrap().unwrap().history_done {
        engine.load_older(chat.id);
        wait_event(
            &mut events,
            |e| matches!(e, EngineEvent::Changed(c) if c.chats.contains(&chat.id)),
        )
        .await;
    }
    assert_eq!(
        store.messages_before(chat.id, None, 500).unwrap().len(),
        120
    );
    assert!(
        store
            .message_by_guid(&guids[3])
            .unwrap()
            .is_some_and(|m| m.date_read.is_some())
    );
}

#[tokio::test]
async fn without_the_private_api_only_safe_requests_go_out() {
    let server = MockServer::start(PASSWORD).await;
    server.set_info(|i| {
        i.private_api = Some(false);
        i.helper_connected = Some(false);
    });
    server.add_chat(&dm(demo::ALEX), None, &[demo::ALEX]);
    let target = server.receive(&dm(demo::ALEX), demo::ALEX, "hello");
    let (engine, _events, store) = synced(&server).await;
    wait_until(|| {
        !engine.status().features.private_api && engine.status().features.macos.is_some()
    })
    .await;
    let chat = store.chat_by_guid(&dm(demo::ALEX)).unwrap().unwrap();

    engine.react(&dm(demo::ALEX), &target.guid, 0, Tapback::Love, false);
    engine.composing(&dm(demo::ALEX), true);
    engine.view(Some(chat.id), true);
    engine.send_text(TextMessage {
        chat_guid: dm(demo::ALEX),
        text: "plain text still works".into(),
        reply_to: Some((target.guid.clone(), 0)),
        ..Default::default()
    });
    wait_until(|| server.messages().len() == 2).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Read here, but the Mac isn't told; no tapback, no typing.
    assert!(!store.chat(chat.id).unwrap().unwrap().unread);
    let requests = server.requests();
    assert!(
        !requests
            .iter()
            .any(|r| r.contains("/react") || r.contains("/typing") || r.ends_with("/read")),
        "{requests:#?}"
    );
    assert!(
        store
            .associated(std::slice::from_ref(&target.guid))
            .unwrap()
            .is_empty()
    );
    // The reply was sent as a plain message.
    assert!(server.messages()[1].thread_originator_guid.is_none());
}
