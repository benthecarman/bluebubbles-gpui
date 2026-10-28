use super::*;
use bb_api::{Attachment, Chat, Handle, Message};

fn alice() -> Handle {
    Handle {
        original_rowid: Some(3),
        address: "+15551234567".into(),
        service: Some("iMessage".into()),
        ..Default::default()
    }
}

const DM: &str = "iMessage;-;+15551234567";

fn dm_chat() -> Chat {
    Chat {
        guid: DM.into(),
        style: Some(45),
        chat_identifier: Some("+15551234567".into()),
        participants: Some(vec![alice()]),
        ..Default::default()
    }
}

fn incoming(guid: &str, text: &str, date: i64) -> Message {
    Message {
        original_rowid: Some(date / 1000),
        guid: guid.into(),
        text: Some(text.into()),
        handle: Some(alice()),
        handle_id: Some(3),
        date_created: Some(date),
        chats: vec![Chat {
            guid: DM.into(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn store_with_chat() -> (Store, i64) {
    let store = Store::open_in_memory().unwrap();
    let changes = store
        .apply(Batch {
            chats: vec![dm_chat()],
            ..Default::default()
        })
        .unwrap();
    let id = *changes.chats.iter().next().unwrap();
    (store, id)
}

#[test]
fn upserts_by_guid_and_moves_dates_forward() {
    let (store, chat) = store_with_chat();
    let mut m = incoming("g1", "hi", 1_000);
    let c = store
        .apply(Batch {
            messages: vec![(None, m.clone())],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(c.inserted.len(), 1);

    // A later update with a read date, then a stale copy without it.
    m.date_read = Some(5_000);
    store
        .apply(Batch {
            messages: vec![(None, m.clone())],
            ..Default::default()
        })
        .unwrap();
    m.date_read = None;
    m.text = None;
    let c = store
        .apply(Batch {
            messages: vec![(None, m)],
            ..Default::default()
        })
        .unwrap();
    assert!(c.inserted.is_empty(), "an update is not an insert");

    let rows = store.messages_before(chat, None, 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].date_read, Some(5_000));
    assert_eq!(rows[0].text.as_deref(), Some("hi"));
    assert_eq!(rows[0].sender.as_deref(), Some("+15551234567"));
}

#[test]
fn resolves_a_send_whichever_reply_comes_first() {
    for socket_first in [false, true] {
        let (store, chat) = store_with_chat();
        let (c, id) = store
            .insert_outgoing(&OutgoingMessage {
                temp_guid: "temp-1".into(),
                chat_guid: DM.into(),
                text: Some("on my way".into()),
                date_created: 10_000,
                ..Default::default()
            })
            .unwrap();
        assert!(c.messages[&chat].contains(&id));
        assert!(store.messages_before(chat, None, 10).unwrap()[0].is_pending());

        let full = Message {
            guid: "REAL".into(),
            text: Some("on my way".into()),
            is_from_me: true,
            date_created: Some(10_500),
            chats: vec![Chat {
                guid: DM.into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let slim = Message {
            temp_guid: Some("temp-1".into()),
            ..full.clone()
        };
        let order = if socket_first {
            vec![full.clone(), slim.clone()]
        } else {
            vec![slim.clone(), full.clone()]
        };
        let mut resolved = Vec::new();
        for m in order {
            let c = store
                .apply(Batch {
                    messages: vec![(None, m)],
                    ..Default::default()
                })
                .unwrap();
            resolved.extend(c.resolved);
        }
        // Either way: one row, the pending one renamed in place.
        let rows = store.messages_before(chat, None, 10).unwrap();
        assert_eq!(resolved, vec![("temp-1".to_string(), "REAL".to_string())]);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].id, id, "the row keeps its identity");
        assert!(!rows[0].is_pending());
    }
}

#[test]
fn derives_unread_from_messages_and_read_state() {
    let (store, chat) = store_with_chat();
    store
        .apply(Batch {
            messages: vec![(None, incoming("g1", "hey", 1_000))],
            ..Default::default()
        })
        .unwrap();
    assert!(store.chat(chat).unwrap().unwrap().unread);

    store.mark_read_local(chat).unwrap();
    assert!(!store.chat(chat).unwrap().unwrap().unread);

    // A newer message makes it unread again; the server marking it read
    // clears it.
    store
        .apply(Batch {
            messages: vec![(None, incoming("g2", "you there?", 2_000))],
            ..Default::default()
        })
        .unwrap();
    assert!(store.chat(chat).unwrap().unwrap().unread);
    store
        .apply(Batch {
            read_states: vec![(DM.into(), true)],
            ..Default::default()
        })
        .unwrap();
    let row = store.chat(chat).unwrap().unwrap();
    assert!(!row.unread);
    assert_eq!(row.last_message.unwrap().guid, "g2");

    // Read on the Mac: the message itself carries dateRead.
    let mut m = incoming("g3", "later", 3_000);
    m.date_read = Some(3_100);
    store
        .apply(Batch {
            messages: vec![(None, m)],
            ..Default::default()
        })
        .unwrap();
    assert!(!store.chat(chat).unwrap().unwrap().unread);

    store
        .apply(Batch {
            read_states: vec![(DM.into(), false)],
            ..Default::default()
        })
        .unwrap();
    assert!(store.chat(chat).unwrap().unwrap().unread);
}

#[test]
fn counts_unread_chats_that_arent_muted() {
    let (store, chat) = store_with_chat();
    assert_eq!(store.unread_chat_count().unwrap(), 0);
    store
        .apply(Batch {
            messages: vec![(None, incoming("g1", "hey", 1_000))],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(store.unread_chat_count().unwrap(), 1);
    store.set_muted(chat, true).unwrap();
    assert_eq!(store.unread_chat_count().unwrap(), 0);
    store.set_muted(chat, false).unwrap();
    store.mark_read_local(chat).unwrap();
    assert_eq!(store.unread_chat_count().unwrap(), 0);
}

#[test]
fn pages_history_by_date_and_hides_tapbacks() {
    let (store, chat) = store_with_chat();
    let mut batch = Batch::default();
    for i in 0..10 {
        batch
            .messages
            .push((None, incoming(&format!("g{i}"), "x", 1_000 + i * 10)));
    }
    let mut tapback = incoming("t1", "Loved “x”", 2_000);
    tapback.associated_message_guid = Some("p:0/g9".into());
    tapback.associated_message_type = Some("love".into());
    batch.messages.push((None, tapback));
    store.apply(batch).unwrap();

    let newest = store.messages_before(chat, None, 4).unwrap();
    assert_eq!(
        newest.iter().map(|m| m.guid.as_str()).collect::<Vec<_>>(),
        ["g6", "g7", "g8", "g9"]
    );
    let older = store
        .messages_before(chat, Some(newest[0].cursor()), 4)
        .unwrap();
    assert_eq!(older.first().unwrap().guid, "g2");
    let after = store.messages_after(chat, older[1].cursor(), 2).unwrap();
    assert_eq!(
        after.iter().map(|m| m.guid.as_str()).collect::<Vec<_>>(),
        ["g4", "g5"]
    );

    let reactions = store.associated(&["g9".into()]).unwrap();
    assert_eq!(reactions.len(), 1);
    assert_eq!(reactions[0].associated_guid.as_deref(), Some("g9"));
    assert_eq!(reactions[0].associated_part, Some(0));
    // The tapback is the chat's latest message, as the sidebar shows it.
    assert_eq!(
        store
            .chat(chat)
            .unwrap()
            .unwrap()
            .last_message
            .unwrap()
            .guid,
        "t1"
    );
}

#[test]
fn makes_stub_chats_for_unknown_chats_and_maps_any_guids() {
    let store = Store::open_in_memory().unwrap();
    let mut m = incoming("g1", "hello", 1_000);
    m.chats[0].guid = "iMessage;+;chat99".into();
    let c = store
        .apply(Batch {
            messages: vec![(None, m)],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(c.incomplete_chats, vec!["iMessage;+;chat99".to_string()]);

    // macOS 26 names the same chat `any;+;chat99`.
    let mut m2 = incoming("g2", "again", 2_000);
    m2.chats[0].guid = "any;+;chat99".into();
    let c2 = store
        .apply(Batch {
            messages: vec![(None, m2)],
            ..Default::default()
        })
        .unwrap();
    assert!(c2.incomplete_chats.is_empty());
    assert_eq!(c.chats, c2.chats);
    assert!(store.chat_by_guid("any;+;chat99").unwrap().is_some());
}

#[test]
fn hands_local_files_to_server_attachments() {
    let (store, chat) = store_with_chat();
    store
        .insert_outgoing(&OutgoingMessage {
            temp_guid: "temp-2".into(),
            chat_guid: DM.into(),
            date_created: 5_000,
            attachments: vec![OutgoingAttachment {
                guid: "temp-2-0".into(),
                path: "/tmp/cat.png".into(),
                name: "cat.png".into(),
                mime_type: "image/png".into(),
                total_bytes: 10,
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
    let echo = Message {
        guid: "REAL2".into(),
        temp_guid: Some("temp-2".into()),
        is_from_me: true,
        date_created: Some(5_100),
        attachments: vec![Attachment {
            guid: "at_0_REAL2".into(),
            mime_type: Some("image/png".into()),
            transfer_name: Some("cat.png".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    store
        .apply(Batch {
            messages: vec![(None, echo)],
            ..Default::default()
        })
        .unwrap();
    let rows = store.messages_before(chat, None, 10).unwrap();
    assert_eq!(rows[0].attachments.len(), 1);
    assert_eq!(rows[0].attachments[0].guid, "at_0_REAL2");
    assert_eq!(
        rows[0].attachments[0].local_path.as_deref(),
        Some("/tmp/cat.png")
    );
}

#[test]
fn notifies_each_message_once() {
    let (store, _) = store_with_chat();
    let c = store
        .apply(Batch {
            messages: vec![
                (None, incoming("old", "before we connected", 1_000)),
                (None, incoming("new", "while offline", 9_000)),
            ],
            ..Default::default()
        })
        .unwrap();
    let first = store.take_notifiable(&c.inserted, 5_000).unwrap();
    assert_eq!(first.len(), 1);
    assert!(
        store
            .take_notifiable(&c.inserted, 5_000)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn outbox_survives_and_counts_attempts() {
    let store = Store::open_in_memory().unwrap();
    store.outbox_push("temp-9", DM, "text", "{}", 1).unwrap();
    store
        .outbox_set_state("temp-9", OUTBOX_IN_FLIGHT, None)
        .unwrap();
    store
        .outbox_set_state("temp-9", OUTBOX_FAILED, Some("timeout"))
        .unwrap();
    let items = store.outbox().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!((items[0].state, items[0].attempts), (OUTBOX_FAILED, 1));
    store.outbox_remove("temp-9").unwrap();
    assert!(store.outbox().unwrap().is_empty());
}

#[test]
fn reopens_a_file_without_redoing_migrations() {
    let dir = std::env::temp_dir().join(format!("bb-store-test-{}", std::process::id()));
    let path = dir.join("cache.sqlite3");
    {
        let store = Store::open(&path).unwrap();
        store.set("cursor", "42").unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.get_i64("cursor").unwrap(), Some(42));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn folds_a_late_reply_into_the_server_row() {
    let (store, chat) = store_with_chat();
    // Two identical texts pending; the server copy of the first arrives
    // with different text (say, Messages trimmed it), so it can't adopt.
    for (temp, date) in [("temp-a", 1_000), ("temp-b", 2_000)] {
        store
            .insert_outgoing(&OutgoingMessage {
                temp_guid: temp.into(),
                chat_guid: DM.into(),
                text: Some("ok ".into()),
                date_created: date,
                ..Default::default()
            })
            .unwrap();
    }
    let full = Message {
        guid: "R1".into(),
        text: Some("different".into()),
        is_from_me: true,
        date_created: Some(1_100),
        chats: vec![Chat {
            guid: DM.into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    store
        .apply(Batch {
            messages: vec![(None, full.clone())],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(store.messages_before(chat, None, 10).unwrap().len(), 3);
    let c = store
        .apply(Batch {
            messages: vec![(
                None,
                Message {
                    temp_guid: Some("temp-a".into()),
                    ..full
                },
            )],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(c.resolved, vec![("temp-a".to_string(), "R1".to_string())]);
    let rows = store.messages_before(chat, None, 10).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.guid.as_str()).collect::<Vec<_>>(),
        ["R1", "temp-b"]
    );
}

#[test]
fn two_connections_write_without_locking_each_other_out() {
    // The app has one connection for the engine and one for the views
    // (drafts); a write on one mustn't fail while the other writes.
    let dir = std::env::temp_dir().join(format!("bb-store-lock-{}", std::process::id()));
    let path = dir.join("cache.sqlite3");
    let engine = Store::open(&path).unwrap();
    let views = Store::open(&path).unwrap();
    let (changes, _) = engine
        .insert_outgoing(&OutgoingMessage {
            temp_guid: "temp-first".into(),
            chat_guid: DM.into(),
            text: Some("first".into()),
            date_created: 1,
            ..Default::default()
        })
        .unwrap();
    let chat = *changes.chats.iter().next().unwrap();
    let drafts = std::thread::spawn(move || {
        for i in 0..200 {
            views.set_draft(chat, Some(&format!("draft {i}"))).unwrap();
        }
    });
    for i in 0..200 {
        engine
            .insert_outgoing(&OutgoingMessage {
                temp_guid: format!("temp-{i}"),
                chat_guid: DM.into(),
                text: Some(format!("message {i}")),
                date_created: 10 + i,
                ..Default::default()
            })
            .unwrap();
    }
    drafts.join().unwrap();
    assert_eq!(engine.messages_before(chat, None, 500).unwrap().len(), 201);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn only_rows_that_keep_history_unbroken_fit() {
    let (store, chat) = store_with_chat();
    // Nothing cached and no known last message: nothing can be placed.
    assert!(!store.fits_history("g0", DM, 1_000).unwrap());
    store
        .apply(Batch {
            messages: vec![(None, incoming("g2", "b", 2_000))],
            ..Default::default()
        })
        .unwrap();
    assert!(store.fits_history("g2", DM, 2_000).unwrap());
    assert!(store.fits_history("g3", DM, 3_000).unwrap());
    assert!(!store.fits_history("g1", DM, 1_000).unwrap());
    assert!(
        !store
            .fits_history("g1", "iMessage;-;nobody", 3_000)
            .unwrap()
    );
    store
        .apply(Batch {
            history_done: vec![DM.into()],
            ..Default::default()
        })
        .unwrap();
    assert!(store.chat(chat).unwrap().unwrap().history_done);
    assert!(store.fits_history("g1", DM, 1_000).unwrap());
}
