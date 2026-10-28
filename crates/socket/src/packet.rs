//! Engine.IO 4 and Socket.IO 5 text packets, the protocol pair spoken by
//! socket.io 4 servers such as the BlueBubbles Server. Binary attachments
//! are not supported; the server never sends them to clients.

use serde_json::Value;
use std::time::Duration;

/// An Engine.IO packet carried in one WebSocket text frame.
#[derive(Debug, Clone, PartialEq)]
pub enum EnginePacket {
    Open(OpenInfo),
    Close,
    Ping(String),
    Pong(String),
    Message(String),
    Upgrade,
    Noop,
}

/// The handshake the server sends first.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenInfo {
    pub sid: String,
    pub ping_interval: Duration,
    pub ping_timeout: Duration,
}

impl EnginePacket {
    pub fn decode(frame: &str) -> Result<Self, PacketError> {
        let mut chars = frame.chars();
        let kind = chars.next().ok_or(PacketError::Empty)?;
        let rest = chars.as_str();
        Ok(match kind {
            '0' => {
                let v: Value = serde_json::from_str(rest).map_err(|_| PacketError::BadOpen)?;
                let ms = |key: &str, default: u64| {
                    Duration::from_millis(v.get(key).and_then(Value::as_u64).unwrap_or(default))
                };
                EnginePacket::Open(OpenInfo {
                    sid: v
                        .get("sid")
                        .and_then(Value::as_str)
                        .ok_or(PacketError::BadOpen)?
                        .to_owned(),
                    ping_interval: ms("pingInterval", 25_000),
                    ping_timeout: ms("pingTimeout", 20_000),
                })
            }
            '1' => EnginePacket::Close,
            '2' => EnginePacket::Ping(rest.to_owned()),
            '3' => EnginePacket::Pong(rest.to_owned()),
            '4' => EnginePacket::Message(rest.to_owned()),
            '5' => EnginePacket::Upgrade,
            '6' => EnginePacket::Noop,
            other => return Err(PacketError::UnknownType(other)),
        })
    }

    pub fn encode(&self) -> String {
        match self {
            EnginePacket::Open(_) => "0".into(),
            EnginePacket::Close => "1".into(),
            EnginePacket::Ping(p) => format!("2{p}"),
            EnginePacket::Pong(p) => format!("3{p}"),
            EnginePacket::Message(m) => format!("4{m}"),
            EnginePacket::Upgrade => "5".into(),
            EnginePacket::Noop => "6".into(),
        }
    }
}

/// A Socket.IO packet on the default namespace (other namespaces are
/// parsed and reported, so they can be ignored).
#[derive(Debug, Clone, PartialEq)]
pub enum SocketPacket {
    Connect(Option<Value>),
    Disconnect,
    Event {
        id: Option<u64>,
        name: String,
        args: Vec<Value>,
    },
    Ack {
        id: u64,
        args: Vec<Value>,
    },
    ConnectError(Option<Value>),
}

/// A decoded Socket.IO packet and the namespace it was addressed to.
#[derive(Debug, Clone, PartialEq)]
pub struct Addressed {
    pub namespace: String,
    pub packet: SocketPacket,
}

impl SocketPacket {
    /// Decodes the payload of an Engine.IO message packet.
    pub fn decode(payload: &str) -> Result<Addressed, PacketError> {
        let bytes = payload.as_bytes();
        let kind = *bytes.first().ok_or(PacketError::Empty)?;
        let mut i = 1;
        if kind == b'5' || kind == b'6' {
            return Err(PacketError::Binary);
        }
        let mut namespace = "/".to_owned();
        if bytes.get(i) == Some(&b'/') {
            let end = payload[i..].find(',').map_or(payload.len(), |n| i + n);
            namespace = payload[i..end].to_owned();
            i = (end + 1).min(payload.len());
        }
        let digits = payload[i..].bytes().take_while(u8::is_ascii_digit).count();
        let id = if digits > 0 {
            Some(
                payload[i..i + digits]
                    .parse()
                    .map_err(|_| PacketError::BadId)?,
            )
        } else {
            None
        };
        i += digits;
        let body = &payload[i..];
        let json = || -> Result<Option<Value>, PacketError> {
            if body.is_empty() {
                Ok(None)
            } else {
                serde_json::from_str(body)
                    .map(Some)
                    .map_err(|_| PacketError::BadJson)
            }
        };
        let args = || -> Result<Vec<Value>, PacketError> {
            match json()? {
                Some(Value::Array(items)) => Ok(items),
                Some(_) => Err(PacketError::BadJson),
                None => Ok(Vec::new()),
            }
        };
        let packet = match kind {
            b'0' => SocketPacket::Connect(json()?),
            b'1' => SocketPacket::Disconnect,
            b'2' => {
                let mut items = args()?.into_iter();
                let name = match items.next() {
                    Some(Value::String(name)) => name,
                    _ => return Err(PacketError::BadJson),
                };
                SocketPacket::Event {
                    id,
                    name,
                    args: items.collect(),
                }
            }
            b'3' => SocketPacket::Ack {
                id: id.ok_or(PacketError::BadId)?,
                args: args()?,
            },
            b'4' => SocketPacket::ConnectError(json()?),
            other => return Err(PacketError::UnknownType(other as char)),
        };
        Ok(Addressed { namespace, packet })
    }

    /// Encodes the packet for the default namespace.
    pub fn encode(&self) -> String {
        match self {
            SocketPacket::Connect(None) => "0".into(),
            SocketPacket::Connect(Some(auth)) => format!("0{auth}"),
            SocketPacket::Disconnect => "1".into(),
            SocketPacket::Event { id, name, args } => {
                let mut items = Vec::with_capacity(args.len() + 1);
                items.push(Value::String(name.clone()));
                items.extend(args.iter().cloned());
                let id = id.map(|id| id.to_string()).unwrap_or_default();
                format!("2{id}{}", Value::Array(items))
            }
            SocketPacket::Ack { id, args } => format!("3{id}{}", Value::Array(args.clone())),
            SocketPacket::ConnectError(None) => "4".into(),
            SocketPacket::ConnectError(Some(v)) => format!("4{v}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PacketError {
    #[error("empty packet")]
    Empty,
    #[error("unknown packet type {0:?}")]
    UnknownType(char),
    #[error("malformed open packet")]
    BadOpen,
    #[error("malformed packet id")]
    BadId,
    #[error("malformed packet JSON")]
    BadJson,
    #[error("binary packets are not supported")]
    Binary,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_open() {
        let p = EnginePacket::decode(
            r#"0{"sid":"abc","upgrades":[],"pingInterval":25000,"pingTimeout":5000,"maxPayload":1000000}"#,
        )
        .unwrap();
        assert_eq!(
            p,
            EnginePacket::Open(OpenInfo {
                sid: "abc".into(),
                ping_interval: Duration::from_secs(25),
                ping_timeout: Duration::from_secs(5),
            })
        );
    }

    #[test]
    fn round_trips_engine_packets() {
        for p in [
            EnginePacket::Ping(String::new()),
            EnginePacket::Pong("probe".into()),
            EnginePacket::Message("2[\"x\"]".into()),
            EnginePacket::Close,
        ] {
            assert_eq!(EnginePacket::decode(&p.encode()).unwrap(), p);
        }
    }

    #[test]
    fn decodes_event_with_payload() {
        let a = SocketPacket::decode(r#"2["new-message",{"guid":"g1","text":"hi"}]"#).unwrap();
        assert_eq!(a.namespace, "/");
        assert_eq!(
            a.packet,
            SocketPacket::Event {
                id: None,
                name: "new-message".into(),
                args: vec![json!({"guid":"g1","text":"hi"})],
            }
        );
    }

    #[test]
    fn decodes_ack_and_ids() {
        let a = SocketPacket::decode(r#"312[{"status":200}]"#).unwrap();
        assert_eq!(
            a.packet,
            SocketPacket::Ack {
                id: 12,
                args: vec![json!({"status":200})]
            }
        );
        let e = SocketPacket::decode(r#"27["ping"]"#).unwrap();
        assert!(matches!(e.packet, SocketPacket::Event { id: Some(7), .. }));
    }

    #[test]
    fn decodes_namespaces_and_connect() {
        let a = SocketPacket::decode(r#"0/admin,{"sid":"x"}"#).unwrap();
        assert_eq!(a.namespace, "/admin");
        assert_eq!(a.packet, SocketPacket::Connect(Some(json!({"sid":"x"}))));
        let c = SocketPacket::decode(r#"0{"sid":"y"}"#).unwrap();
        assert_eq!(c.namespace, "/");
        let err = SocketPacket::decode(r#"4{"message":"Not authorized"}"#).unwrap();
        assert_eq!(
            err.packet,
            SocketPacket::ConnectError(Some(json!({"message":"Not authorized"})))
        );
    }

    #[test]
    fn encodes_event_with_ack_id() {
        let p = SocketPacket::Event {
            id: Some(3),
            name: "started-typing".into(),
            args: vec![json!({"chatGuid":"c"})],
        };
        assert_eq!(p.encode(), r#"23["started-typing",{"chatGuid":"c"}]"#);
        assert_eq!(SocketPacket::decode(&p.encode()).unwrap().packet, p);
    }

    #[test]
    fn rejects_garbage() {
        assert!(SocketPacket::decode("").is_err());
        assert!(SocketPacket::decode("2not json").is_err());
        assert!(SocketPacket::decode("51-[\"x\",{}]").is_err());
        assert!(EnginePacket::decode("9").is_err());
    }
}
