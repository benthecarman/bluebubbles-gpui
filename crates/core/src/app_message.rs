//! Messages sent from iMessage apps: Apple Cash payments, GamePigeon,
//! polls and the like. Their text is only an object-replacement
//! character. What there is to show sits in `payloadData`, an
//! `NSKeyedArchiver` archive (see `rich_link`) of a dictionary: the app's
//! name under `an`, a one-line description under `ldtext`, and a
//! `userInfo` dictionary with `caption` and `subcaption` ("$20.00
//! Payment" for Apple Cash).

use crate::rich_link::resolve;
use serde_json::Value;

/// The `balloonBundleId` prefix of messages from iMessage apps; the
/// app's own id follows the last colon.
pub const APP_BALLOON: &str = "com.apple.messages.MSMessageExtensionBalloonPlugin";
const APPLE_CASH: &str = "com.apple.PassbookUIService.PeerPaymentMessagesExtension";

/// Apps without a payload to name them.
const KNOWN: &[(&str, &str)] = &[
    (APPLE_CASH, "Apple Cash"),
    ("com.gamerdelights.gamepigeon.ext", "GamePigeon"),
    (
        "com.nearfuturespecialists.imessagepoll.MessagesExtension",
        "Poll",
    ),
    ("com.apple.messages.Polls", "Poll"),
    ("com.google.ios.youtube.MessagesExtension", "YouTube"),
    ("com.shazam.Shazam.imessageextension", "Shazam"),
    ("com.apple.mobileslideshow.PhotosMessagesApp", "Photos"),
    ("com.google.Maps.MessagesExtension", "Google Maps"),
    ("com.apple.Handwriting.HandwritingProvider", "Handwriting"),
    ("com.apple.DigitalTouchBalloonProvider", "Digital Touch"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppMessage {
    /// "Apple Cash", "GamePigeon".
    pub app: String,
    pub kind: AppKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppKind {
    /// Money sent or asked for: the amount ("$20.00") and what it is
    /// ("Payment", "Request").
    Payment {
        amount: Option<String>,
        label: Option<String>,
    },
    /// Any other app: its caption and a second line, where it gave them.
    Other {
        caption: Option<String>,
        detail: Option<String>,
    },
}

impl AppMessage {
    /// One line for the chat list and notifications.
    pub fn summary(&self) -> String {
        let detail = match &self.kind {
            AppKind::Payment { amount, label } => match (amount, label) {
                (Some(a), Some(l)) => Some(format!("{a} {l}")),
                (a, l) => a.clone().or_else(|| l.clone()),
            },
            AppKind::Other { caption, detail } => caption.clone().or_else(|| detail.clone()),
        };
        match detail {
            Some(d) if d != self.app => format!("{}: {d}", self.app),
            _ => self.app.clone(),
        }
    }
}

/// The app message a message is, from its `balloonBundleId` and
/// `payloadData`; `None` for ordinary messages and link previews.
pub fn parse(balloon: Option<&str>, payload: Option<&Value>) -> Option<AppMessage> {
    let balloon = balloon?;
    let id = if balloon.starts_with(APP_BALLOON) {
        balloon.rsplit(':').next().unwrap_or(balloon)
    } else if KNOWN.iter().any(|(known, _)| *known == balloon) {
        balloon
    } else {
        return None;
    };
    let fields = payload.and_then(Fields::read);
    let field = |key: &str| fields.as_ref().and_then(|f| f.get(key));
    let app = KNOWN
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(_, name)| (*name).to_owned())
        .or_else(|| field("an"))
        .unwrap_or_else(|| "iMessage App".into());
    let caption = field("caption");
    let subcaption = field("subcaption");
    let kind = if id == APPLE_CASH {
        let (amount, label) = match subcaption.or_else(|| field("ldtext")) {
            Some(s) => split_amount(&s),
            None => (None, None),
        };
        AppKind::Payment { amount, label }
    } else {
        AppKind::Other {
            detail: subcaption
                .or_else(|| field("ldtext"))
                .filter(|d| Some(d) != caption.as_ref()),
            caption,
        }
    };
    Some(AppMessage { app, kind })
}

/// "$20.00 Payment" → ("$20.00", "Payment"), and "12,50 € Payment" →
/// ("12,50 €", "Payment"); a line that doesn't start with an amount is
/// all label.
fn split_amount(s: &str) -> (Option<String>, Option<String>) {
    let amountish = |t: &&str| {
        t.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',') || !c.is_alphanumeric())
    };
    let tokens: Vec<&str> = s.split_whitespace().collect();
    let n = tokens.iter().take_while(|t| amountish(t)).count();
    let (amount, label) = (tokens[..n].join(" "), tokens[n..].join(" "));
    let some = |t: String| (!t.is_empty()).then_some(t);
    if amount.chars().any(|c| c.is_ascii_digit()) {
        (some(amount), some(label))
    } else {
        (None, some(tokens.join(" ")))
    }
}

/// The strings of the archived dictionary, with `userInfo`'s merged in.
struct Fields(Vec<(String, String)>);

impl Fields {
    fn read(payload: &Value) -> Option<Self> {
        let archive = match payload {
            Value::Array(items) => items.first()?,
            other => other,
        };
        let objects = archive.get("$objects")?.as_array()?;
        let root = resolve(objects, archive.get("$top")?.get("root")?)?;
        let mut out = Vec::new();
        collect(objects, root, &mut out, 0);
        Some(Self(out))
    }

    fn get(&self, key: &str) -> Option<String> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }
}

/// Reads an `NSDictionary` (`NS.keys` and `NS.objects`), going one level
/// into dictionaries it holds.
fn collect(objects: &[Value], dict: &Value, out: &mut Vec<(String, String)>, depth: usize) {
    let (Some(keys), Some(values)) = (
        dict.get("NS.keys").and_then(Value::as_array),
        dict.get("NS.objects").and_then(Value::as_array),
    ) else {
        return;
    };
    for (k, v) in keys.iter().zip(values) {
        let (Some(k), Some(v)) = (
            resolve(objects, k).and_then(Value::as_str),
            resolve(objects, v),
        ) else {
            continue;
        };
        match v {
            Value::String(s) if !s.trim().is_empty() => {
                out.push((k.to_owned(), s.trim().to_owned()))
            }
            Value::Object(_) if depth == 0 => collect(objects, v, out, depth + 1),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CASH: &str = "com.apple.messages.MSMessageExtensionBalloonPlugin:0000000000:\
                        com.apple.PassbookUIService.PeerPaymentMessagesExtension";

    /// A dictionary archived the way the server sends it.
    fn archive(fields: &[(&str, &str)], user_info: &[(&str, &str)]) -> Value {
        let mut objects = vec![json!("$null"), json!(null), json!(null)];
        let mut add = |v: Value| {
            objects.push(v);
            json!({"UID": objects.len() - 1})
        };
        let dict = |pairs: &[(&str, &str)], add: &mut dyn FnMut(Value) -> Value| {
            let (mut keys, mut values) = (vec![], vec![]);
            for (k, v) in pairs {
                keys.push(add(json!(k)));
                values.push(add(json!(v)));
            }
            (keys, values)
        };
        let (uk, uv) = dict(user_info, &mut add);
        let (mut keys, mut values) = dict(fields, &mut add);
        keys.push(add(json!("userInfo")));
        objects[2] = json!({"NS.keys": uk, "NS.objects": uv});
        values.push(json!({"UID": 2}));
        objects[1] = json!({"NS.keys": keys, "NS.objects": values});
        json!([{"$top": {"root": {"UID": 1}}, "$objects": objects}])
    }

    #[test]
    fn apple_cash_shows_the_amount() {
        let payload = archive(
            &[("an", "Apple Pay"), ("ldtext", "Sent $20 with Apple Cash.")],
            &[("caption", "Apple Cash"), ("subcaption", "$20.00 Payment")],
        );
        let app = parse(Some(CASH), Some(&payload)).unwrap();
        assert_eq!(app.app, "Apple Cash");
        assert_eq!(
            app.kind,
            AppKind::Payment {
                amount: Some("$20.00".into()),
                label: Some("Payment".into())
            }
        );
        assert_eq!(app.summary(), "Apple Cash: $20.00 Payment");
    }

    #[test]
    fn without_a_payload_it_is_still_named() {
        // The chat list's last message comes without payloadData.
        let app = parse(Some(CASH), None).unwrap();
        assert_eq!(app.summary(), "Apple Cash");
        let pigeon = "com.apple.messages.MSMessageExtensionBalloonPlugin:QQ:\
                      com.gamerdelights.gamepigeon.ext";
        assert_eq!(parse(Some(pigeon), None).unwrap().app, "GamePigeon");
    }

    #[test]
    fn other_apps_use_their_caption() {
        let payload = archive(&[("an", "Shuffleboard")], &[("caption", "Your turn!")]);
        let id = "com.apple.messages.MSMessageExtensionBalloonPlugin:QQ:com.example.shuffle";
        let app = parse(Some(id), Some(&payload)).unwrap();
        assert_eq!(app.app, "Shuffleboard");
        assert_eq!(app.summary(), "Shuffleboard: Your turn!");
    }

    #[test]
    fn requests_and_plain_messages() {
        assert_eq!(
            split_amount("$5.50 Request"),
            (Some("$5.50".into()), Some("Request".into()))
        );
        assert_eq!(
            split_amount("Request Pending"),
            (None, Some("Request Pending".into()))
        );
        assert_eq!(
            split_amount("12,50 € Payment"),
            (Some("12,50 €".into()), Some("Payment".into()))
        );
        assert!(parse(Some("com.apple.messages.URLBalloonProvider"), None).is_none());
        assert!(parse(None, None).is_none());
    }
}
