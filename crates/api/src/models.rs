//! The server's JSON shapes (`packages/server/src/server/api/serializers`).
//!
//! Fields vary with the server and macOS version, and with the `with`
//! options of a request, so nearly everything is optional and unknown
//! fields are ignored. Dates are milliseconds since the Unix epoch.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// The envelope around every REST response.
#[derive(Debug, Clone, Deserialize)]
pub struct Envelope<T> {
    pub status: u16,
    #[serde(default)]
    pub message: Option<String>,
    pub data: Option<T>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Handle {
    #[serde(rename = "originalROWID", deserialize_with = "lenient_i64")]
    pub original_rowid: Option<i64>,
    pub address: String,
    pub service: Option<String>,
    pub uncanonicalized_id: Option<String>,
    pub country: Option<String>,
}

/// `style` values for chats.
pub const STYLE_GROUP: i64 = 43;
pub const STYLE_DIRECT: i64 = 45;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Chat {
    #[serde(rename = "originalROWID", deserialize_with = "lenient_i64")]
    pub original_rowid: Option<i64>,
    pub guid: String,
    #[serde(deserialize_with = "lenient_i64")]
    pub style: Option<i64>,
    pub chat_identifier: Option<String>,
    #[serde(deserialize_with = "lenient_bool")]
    pub is_archived: Option<bool>,
    pub display_name: Option<String>,
    pub participants: Option<Vec<Handle>>,
    pub last_message: Option<Box<Message>>,
    pub group_id: Option<String>,
    pub properties: Option<Value>,
}

impl Chat {
    pub fn is_group(&self) -> bool {
        self.style == Some(STYLE_GROUP)
            || self.participants.as_ref().is_some_and(|p| p.len() > 1)
            || self.guid.contains(";+;")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Attachment {
    #[serde(rename = "originalROWID", deserialize_with = "lenient_i64")]
    pub original_rowid: Option<i64>,
    pub guid: String,
    pub uti: Option<String>,
    pub mime_type: Option<String>,
    pub transfer_name: Option<String>,
    #[serde(deserialize_with = "lenient_i64")]
    pub total_bytes: Option<i64>,
    #[serde(deserialize_with = "lenient_i64")]
    pub transfer_state: Option<i64>,
    #[serde(deserialize_with = "lenient_bool")]
    pub is_outgoing: Option<bool>,
    #[serde(deserialize_with = "lenient_bool")]
    pub hide_attachment: Option<bool>,
    #[serde(deserialize_with = "lenient_bool")]
    pub is_sticker: Option<bool>,
    #[serde(deserialize_with = "lenient_bool")]
    pub has_live_photo: Option<bool>,
    #[serde(deserialize_with = "lenient_i64")]
    pub height: Option<i64>,
    #[serde(deserialize_with = "lenient_i64")]
    pub width: Option<i64>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Message {
    #[serde(rename = "originalROWID", deserialize_with = "lenient_i64")]
    pub original_rowid: Option<i64>,
    pub guid: String,
    /// Present on the server's echo of a message this client sent.
    pub temp_guid: Option<String>,
    pub text: Option<String>,
    pub subject: Option<String>,
    pub attributed_body: Option<Value>,
    pub handle: Option<Handle>,
    #[serde(deserialize_with = "lenient_i64")]
    pub handle_id: Option<i64>,
    #[serde(deserialize_with = "lenient_i64")]
    pub other_handle: Option<i64>,
    #[serde(deserialize_with = "null_as_default")]
    pub attachments: Vec<Attachment>,
    #[serde(deserialize_with = "lenient_i64")]
    pub error: Option<i64>,
    #[serde(deserialize_with = "lenient_i64")]
    pub date_created: Option<i64>,
    #[serde(deserialize_with = "lenient_i64")]
    pub date_read: Option<i64>,
    #[serde(deserialize_with = "lenient_i64")]
    pub date_delivered: Option<i64>,
    #[serde(deserialize_with = "lenient_i64")]
    pub date_edited: Option<i64>,
    #[serde(deserialize_with = "lenient_i64")]
    pub date_retracted: Option<i64>,
    #[serde(deserialize_with = "lenient_i64")]
    pub date_played: Option<i64>,
    #[serde(deserialize_with = "lenient_bool")]
    pub is_delivered: Option<bool>,
    #[serde(deserialize_with = "lenient_bool_default")]
    pub is_from_me: bool,
    #[serde(deserialize_with = "lenient_bool")]
    pub has_dd_results: Option<bool>,
    #[serde(deserialize_with = "lenient_i64")]
    pub item_type: Option<i64>,
    pub group_title: Option<String>,
    #[serde(deserialize_with = "lenient_i64")]
    pub group_action_type: Option<i64>,
    pub balloon_bundle_id: Option<String>,
    pub associated_message_guid: Option<String>,
    /// `love`, `-love`, … as the server maps Apple's 2000/3000 codes;
    /// other codes (such as 2006, an emoji tapback) arrive as numbers in a
    /// string.
    #[serde(deserialize_with = "lenient_string")]
    pub associated_message_type: Option<String>,
    #[serde(deserialize_with = "lenient_i64")]
    pub associated_message_part: Option<i64>,
    pub expressive_send_style_id: Option<String>,
    pub thread_originator_guid: Option<String>,
    #[serde(deserialize_with = "lenient_string")]
    pub thread_originator_part: Option<String>,
    #[serde(deserialize_with = "lenient_bool")]
    pub has_payload_data: Option<bool>,
    pub payload_data: Option<Value>,
    pub message_summary_info: Option<Value>,
    #[serde(deserialize_with = "lenient_i64")]
    pub part_count: Option<i64>,
    #[serde(deserialize_with = "lenient_bool")]
    pub was_delivered_quietly: Option<bool>,
    #[serde(deserialize_with = "lenient_bool")]
    pub did_notify_recipient: Option<bool>,
    #[serde(deserialize_with = "lenient_bool")]
    pub is_audio_message: Option<bool>,
    #[serde(deserialize_with = "null_as_default")]
    pub chats: Vec<Chat>,
}

/// `GET /server/info`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerInfo {
    pub computer_id: Option<String>,
    pub os_version: Option<String>,
    pub server_version: Option<String>,
    #[serde(deserialize_with = "lenient_bool")]
    pub private_api: Option<bool>,
    pub proxy_service: Option<String>,
    #[serde(deserialize_with = "lenient_bool")]
    pub helper_connected: Option<bool>,
    pub detected_icloud: Option<String>,
    pub detected_imessage: Option<String>,
    #[serde(deserialize_with = "null_as_default")]
    pub local_ipv4s: Vec<String>,
    #[serde(deserialize_with = "null_as_default")]
    pub local_ipv6s: Vec<String>,
}

impl ServerInfo {
    /// Whether features that need the Private API helper will work now.
    pub fn private_api_ready(&self) -> bool {
        self.private_api == Some(true) && self.helper_connected == Some(true)
    }

    /// macOS `(major, minor)`, e.g. `(14, 4)`.
    pub fn macos(&self) -> Option<(u32, u32)> {
        let v = self.os_version.as_deref()?;
        let mut parts = v.split('.').map(|p| p.parse::<u32>().ok());
        Some((parts.next()??, parts.next().flatten().unwrap_or(0)))
    }

    /// Messages on macOS 13+ records edit and retraction dates.
    pub fn supports_edits(&self) -> bool {
        self.macos().is_some_and(|(major, _)| major >= 13)
    }
}

/// One entry of `GET /contact`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Contact {
    #[serde(deserialize_with = "lenient_string")]
    pub id: Option<String>,
    pub display_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub nickname: Option<String>,
    #[serde(deserialize_with = "null_as_default")]
    pub phone_numbers: Vec<ContactAddress>,
    #[serde(deserialize_with = "null_as_default")]
    pub emails: Vec<ContactAddress>,
    /// Base64 image data, when asked for with `extraProperties=avatar`.
    pub avatar: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContactAddress {
    pub address: String,
}

// The server's types drift between versions (numbers as strings, 0/1 as
// booleans); one bad field must not drop a whole message.

fn lenient_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(b as i64),
        _ => None,
    })
}

fn lenient_bool<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Bool(b) => Some(b),
        Value::Number(n) => n.as_i64().map(|n| n != 0),
        Value::String(s) => match s.as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

fn lenient_bool_default<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(lenient_bool(d)?.unwrap_or(false))
}

fn lenient_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn null_as_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(
    d: D,
) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_a_server_message() {
        let m: Message = serde_json::from_value(json!({
            "originalROWID": 812,
            "guid": "p:0/AAAA-1111",
            "text": "Hello",
            "handle": {"originalROWID": 3, "address": "+15551234567", "service": "iMessage", "country": "us"},
            "handleId": 3,
            "otherHandle": 0,
            "attachments": null,
            "subject": null,
            "error": 0,
            "dateCreated": 1_700_000_000_000i64,
            "dateRead": null,
            "dateDelivered": 1_700_000_001_000i64,
            "isDelivered": true,
            "isFromMe": false,
            "hasDdResults": false,
            "itemType": 0,
            "groupTitle": null,
            "groupActionType": 0,
            "balloonBundleId": null,
            "associatedMessageGuid": null,
            "associatedMessageType": null,
            "expressiveSendStyleId": null,
            "threadOriginatorGuid": null,
            "threadOriginatorPart": "0:0:5",
            "hasPayloadData": false,
            "chats": [{"originalROWID": 1, "guid": "iMessage;-;+15551234567", "style": 45, "chatIdentifier": "+15551234567", "isArchived": false, "displayName": ""}],
            "partCount": 1,
            "someFutureField": {"x": 1}
        }))
        .unwrap();
        assert_eq!(m.original_rowid, Some(812));
        assert_eq!(m.handle.as_ref().unwrap().address, "+15551234567");
        assert!(m.attachments.is_empty());
        assert_eq!(m.date_delivered, Some(1_700_000_001_000));
        assert_eq!(m.chats[0].style, Some(STYLE_DIRECT));
        assert!(!m.chats[0].is_group());
    }

    #[test]
    fn tolerates_drifting_types() {
        let m: Message = serde_json::from_value(json!({
            "guid": "g",
            "isFromMe": 1,
            "error": "22",
            "associatedMessageType": 2006,
            "dateCreated": 1.7e12,
            "chats": null
        }))
        .unwrap();
        assert!(m.is_from_me);
        assert_eq!(m.error, Some(22));
        assert_eq!(m.associated_message_type.as_deref(), Some("2006"));
        assert_eq!(m.date_created, Some(1_700_000_000_000));
    }

    #[test]
    fn reads_server_info() {
        let info: ServerInfo = serde_json::from_value(json!({
            "computer_id": "abc",
            "os_version": "14.4.1",
            "server_version": "1.9.9",
            "private_api": true,
            "helper_connected": true,
            "local_ipv4s": ["192.168.1.2"]
        }))
        .unwrap();
        assert!(info.private_api_ready());
        assert_eq!(info.macos(), Some((14, 4)));
        assert!(info.supports_edits());
    }
}
