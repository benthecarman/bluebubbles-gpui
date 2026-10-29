//! Conversations for demos and screenshots: direct and group chats,
//! iMessage and SMS, tapbacks, a reply, an edit, a group event and a
//! photo. All names and numbers are made up.

use crate::{MockServer, Stored, now_ms};

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;

pub const MOM: &str = "+15550101234";
pub const ALEX: &str = "alex.kim@example.com";
pub const SAM: &str = "+15550104567";
pub const JORDAN: &str = "+15550107890";
pub const PRIYA: &str = "priya@example.com";

const PHOTO: &[u8] = include_bytes!("../../../assets/icons/bluebubbles-512.png");
const PREVIEW: &[u8] = include_bytes!("../../../assets/icons/bluebubbles-256.png");
const PDF: &[u8] = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[]/Count 0>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";

/// A made-up profile picture: a head and shoulders on a soft background,
/// 600 pixels square as phone contact photos are.
pub fn portrait(background: [u8; 3], hair: [u8; 3]) -> Vec<u8> {
    const SIDE: u32 = 600;
    let skin = [224.0, 182.0, 150.0];
    let image = image::RgbImage::from_fn(SIDE, SIDE, |x, y| {
        let (x, y) = (x as f32 / SIDE as f32, y as f32 / SIDE as f32);
        let shade = 1.0 - 0.25 * y;
        let mut c = background.map(|v| v as f32 * shade);
        let head = ((x - 0.5) / 0.19).powi(2) + ((y - 0.42) / 0.23).powi(2);
        let hair_top = ((x - 0.5) / 0.21).powi(2) + ((y - 0.36) / 0.21).powi(2);
        let shoulders = ((x - 0.5) / 0.42).powi(2) + ((y - 1.02) / 0.3).powi(2);
        if shoulders < 1.0 {
            c = hair.map(|v| v as f32 * 0.6 + 60.0);
        }
        if hair_top < 1.0 && y < 0.38 {
            c = hair.map(|v| v as f32);
        }
        if head < 1.0 && !(hair_top < 1.0 && y < 0.3) {
            c = skin;
        }
        image::Rgb(c.map(|v| v.clamp(0.0, 255.0) as u8))
    });
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::Jpeg,
        )
        .expect("encode a JPEG");
    out
}

pub fn seed_demo(server: &MockServer) {
    let now = now_ms();
    server.add_contact("Mom", &[MOM], &[]);
    server.add_contact("Alex Kim", &[], &[ALEX]);
    server.add_contact("Sam Rivera", &[SAM], &[]);
    server.add_contact("Jordan Lee", &[JORDAN], &[]);
    server.add_contact("Priya Shah", &[], &[PRIYA]);
    server.set_contact_photo("Mom", &portrait([236, 160, 120], [120, 70, 50]));
    server.set_contact_photo("Alex Kim", &portrait([110, 170, 230], [40, 50, 70]));
    server.set_contact_photo("Sam Rivera", &portrait([150, 210, 140], [90, 60, 40]));
    server.set_contact_photo("Priya Shah", &portrait([230, 140, 190], [30, 25, 25]));
    // Jordan has no contact photo but shares one through iMessage, so the
    // list shows it with the Private API and initials without.
    server.set_shared_photo(JORDAN, &portrait([250, 200, 90], [60, 40, 30]));

    let mom = format!("iMessage;-;{MOM}");
    let alex = format!("iMessage;-;{ALEX}");
    let trip = "iMessage;+;chat100200300";
    let jordan = format!("SMS;-;{JORDAN}");
    let dentist = "SMS;-;+15550109999";
    let book = "iMessage;+;chat400500600";

    server.add_chat(&mom, None, &[MOM]);
    server.add_chat(&alex, None, &[ALEX]);
    server.add_chat(trip, Some("Weekend Trip 🏕️"), &[ALEX, SAM, JORDAN]);
    server.add_chat(&jordan, None, &[JORDAN]);
    server.add_chat(dentist, None, &["+15550109999"]);
    server.add_chat(book, None, &[PRIYA, SAM]);

    let at = |offset: i64| now - offset;
    let me: Option<&str> = None;

    // Weekend trip: a busy group with a rename, a photo and a tapback.
    let mut seq = vec![
        (
            Some(SAM),
            "Okay who's driving Saturday?",
            at(DAY + 3 * HOUR),
        ),
        (
            Some(ALEX),
            "I can take 4 people",
            at(DAY + 3 * HOUR - 2 * MIN),
        ),
        (
            me,
            "I'll bring the tent and the stove",
            at(DAY + 3 * HOUR - 4 * MIN),
        ),
        (Some(JORDAN), "I'll handle snacks 🍫", at(DAY + 2 * HOUR)),
        (
            Some(SAM),
            "Campsite confirmed for two nights!",
            at(3 * HOUR),
        ),
        (
            Some(ALEX),
            "Here's the view from last year",
            at(3 * HOUR - MIN),
        ),
        (me, "That's amazing, can't wait", at(2 * HOUR)),
        (
            Some(JORDAN),
            "Leaving at 8 sharp, don't be late",
            at(40 * MIN),
        ),
    ];
    let trip_ids = insert_all(server, trip, &mut seq);
    let mut photo = server
        .shared
        .lock()
        .attachment("campsite.png", "image/png", PHOTO.to_vec());
    photo.width = Some(512);
    photo.height = Some(512);
    server.update(&trip_ids[5], true, |m| m.attachments = vec![photo]);
    let itinerary =
        server
            .shared
            .lock()
            .attachment("Itinerary.pdf", "application/pdf", PDF.to_vec());
    server.update(&trip_ids[4], true, |m| m.attachments = vec![itinerary]);
    group_event(server, trip, SAM, at(DAY + 4 * HOUR), "Weekend Trip 🏕️");
    tapback(
        server,
        trip,
        Some(SAM),
        &trip_ids[6],
        "love",
        at(2 * HOUR - MIN),
    );
    tapback(
        server,
        trip,
        Some(ALEX),
        &trip_ids[6],
        "like",
        at(2 * HOUR - 2 * MIN),
    );

    // Mom: a direct chat with history over several days, read receipt.
    let mut seq = vec![
        (Some(MOM), "Did you get home safe?", at(3 * DAY)),
        (me, "Yes! Thanks for dinner ❤️", at(3 * DAY - 10 * MIN)),
        (Some(MOM), "Anytime sweetie", at(3 * DAY - 12 * MIN)),
        (me, "\u{fffc}", at(2 * DAY)),
        (me, "happy birthday", at(2 * DAY - 20_000)),
        (Some(MOM), "\u{fffc}", at(2 * DAY - 30 * MIN)),
        (Some(MOM), "Are you coming for Sunday lunch?", at(26 * MIN)),
        (me, "Wouldn't miss it", at(25 * MIN)),
        (me, "Should I bring anything?", at(25 * MIN - 20_000)),
        (Some(MOM), "Just yourself 😊", at(9 * MIN)),
    ];
    let mom_ids = insert_all(server, &mom, &mut seq);
    // Apple Cash: a payment from me, and Mom asking for one back.
    apple_cash(server, &mom_ids[3], "$20.00 Payment");
    apple_cash(server, &mom_ids[5], "$12.50 Request");
    for guid in &mom_ids[7..9] {
        server.update(guid, true, |m| {
            m.date_delivered = Some(at(24 * MIN));
            m.date_read = Some(at(10 * MIN));
        });
    }
    tapback(
        server,
        &mom,
        Some(MOM),
        &mom_ids[1],
        "love",
        at(3 * DAY - 11 * MIN),
    );

    // Alex: a reply thread and an edited message, last message from me.
    let mut seq = vec![
        (Some(ALEX), "Did you see the new trailer?", at(5 * HOUR)),
        (Some(ALEX), "It looks so good", at(5 * HOUR - 20_000)),
        (me, "Not yet, sending it to me?", at(4 * HOUR)),
        (
            Some(ALEX),
            "https://example.com/trailer",
            at(4 * HOUR - MIN),
        ),
        (me, "Watching it tonight", at(4 * HOUR - 3 * MIN)),
    ];
    let alex_ids = insert_all(server, &alex, &mut seq);
    // The link as Messages sends it: a rich-link balloon, with the page's
    // picture attached by the sender's phone.
    let preview = server.shared.lock().attachment(
        "4B2A.pluginPayloadAttachment",
        "application/octet-stream",
        PREVIEW.to_vec(),
    );
    server.update(&alex_ids[3], true, |m| {
        m.balloon_bundle_id = Some("com.apple.messages.URLBalloonProvider".into());
        m.payload_data = Some(serde_json::json!([{
            "$archiver": "NSKeyedArchiver",
            "$top": {"root": {"UID": 1}},
            "$objects": [
                "$null",
                {"richLinkMetadata": {"UID": 2}, "richLinkIsPlaceholder": false},
                {"title": {"UID": 3}, "siteName": {"UID": 4}, "URL": {"UID": 5}},
                "Starfall — Official Trailer",
                "Example Films",
                {"NS.relative": "https://example.com/trailer"}
            ]
        }]));
        m.attachments = vec![preview];
    });
    server.update(&alex_ids[4], true, |m| {
        m.thread_originator_guid = Some(alex_ids[0].clone());
        m.thread_originator_part = Some("0:0:27".into());
        m.date_edited = Some(at(4 * HOUR - 2 * MIN));
        m.date_delivered = Some(at(4 * HOUR - 3 * MIN));
    });

    // Jordan over SMS: green bubbles.
    let mut seq = vec![
        (Some(JORDAN), "Hey, it's Jordan. New number!", at(2 * DAY)),
        (me, "Got it, saved 👍", at(2 * DAY - 5 * MIN)),
        (
            Some(JORDAN),
            "Can you send me the address for Saturday?",
            at(70 * MIN),
        ),
    ];
    insert_all(server, &jordan, &mut seq);

    // An unknown number.
    let mut seq = vec![(
        Some("+15550109999"),
        "Reminder: your dental cleaning is Tue at 9:30 AM. Reply C to confirm.",
        at(6 * DAY),
    )];
    insert_all(server, dentist, &mut seq);

    // Unnamed group: the title comes from its members. A long backlog
    // there pages in from the server as the transcript scrolls up.
    let backlog: Vec<String> = (1..=180).map(|n| format!("Chapter {n} notes")).collect();
    let mut seq: Vec<_> = backlog
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let from = [Some(PRIYA), Some(SAM), me][i % 3];
            (
                from,
                text.as_str(),
                at(9 * DAY + (180 - i as i64) * 3 * HOUR),
            )
        })
        .collect();
    insert_all(server, book, &mut seq);
    let mut seq = vec![
        (
            Some(PRIYA),
            "Next book: The Left Hand of Darkness?",
            at(9 * DAY),
        ),
        (Some(SAM), "Yes please", at(9 * DAY - HOUR)),
        (me, "I'm in 📚", at(8 * DAY)),
    ];
    insert_all(server, book, &mut seq);

    // The server's ROWIDs follow arrival order; catch-up relies on it.
    let mut s = server.shared.lock();
    s.messages.sort_by_key(|m| m.message.date_created);
    for (i, m) in s.messages.iter_mut().enumerate() {
        m.message.original_rowid = Some(i as i64 + 1);
    }
}

fn insert_all(
    server: &MockServer,
    chat: &str,
    seq: &mut [(Option<&str>, &str, i64)],
) -> Vec<String> {
    seq.iter()
        .map(|(from, text, date)| {
            let m = server.insert_silently(chat, *from, text, *date);
            if from.is_none() {
                server.update(&m.guid, true, |m| m.date_delivered = Some(date + 2_000));
            }
            m.guid
        })
        .collect()
}

fn tapback(
    server: &MockServer,
    chat: &str,
    from: Option<&str>,
    target: &str,
    kind: &str,
    date: i64,
) {
    let text = server
        .messages()
        .into_iter()
        .find(|m| m.guid == target)
        .and_then(|m| m.text)
        .unwrap_or_default();
    let verb = match kind {
        "love" => "Loved",
        "like" => "Liked",
        "laugh" => "Laughed at",
        _ => "Reacted to",
    };
    let m = server.insert_silently(chat, from, &format!("{verb} “{text}”"), date);
    server.update(&m.guid, true, |m| {
        m.associated_message_guid = Some(format!("p:0/{target}"));
        m.associated_message_type = Some(kind.to_owned());
    });
}

/// Makes `guid` an Apple Cash message, archived as Messages sends it.
fn apple_cash(server: &MockServer, guid: &str, subcaption: &str) {
    server.update(guid, true, |m| {
        m.balloon_bundle_id = Some(
            "com.apple.messages.MSMessageExtensionBalloonPlugin:0000000000:\
             com.apple.PassbookUIService.PeerPaymentMessagesExtension"
                .into(),
        );
        m.payload_data = Some(serde_json::json!([{
            "$archiver": "NSKeyedArchiver",
            "$top": {"root": {"UID": 1}},
            "$objects": [
                "$null",
                {
                    "NS.keys": [{"UID": 3}, {"UID": 4}, {"UID": 5}],
                    "NS.objects": [{"UID": 6}, {"UID": 7}, {"UID": 2}]
                },
                {
                    "NS.keys": [{"UID": 8}, {"UID": 9}],
                    "NS.objects": [{"UID": 10}, {"UID": 11}]
                },
                "an", "ldtext", "userInfo",
                "Apple Pay", "Apple Cash",
                "caption", "subcaption",
                "Apple Cash", subcaption
            ]
        }]));
    });
}

fn group_event(server: &MockServer, chat: &str, from: &str, date: i64, title: &str) {
    let mut s = server.shared.lock();
    let mut m = s.new_message(chat, Some(from), "", date);
    m.text = None;
    m.item_type = Some(2);
    m.group_title = Some(title.to_owned());
    s.messages.push(Stored {
        message: m,
        updated_at: date,
    });
}
