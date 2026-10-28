//! Sending one outbox item, and telling a lost reply from a failed send.

use crate::now_ms;
use bb_api::{ApiError, Client, Message, MessageQuery, SendAttachment, SendText};
use serde::{Deserialize, Serialize};

/// An outbox payload, stored as JSON so a send survives a restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Outbound {
    Text {
        text: String,
        subject: Option<String>,
        effect: Option<String>,
        reply_to: Option<(String, i64)>,
    },
    Attachment {
        path: String,
        name: String,
        mime: String,
        reply_to: Option<(String, i64)>,
    },
    React {
        target: String,
        part: i64,
        /// `love`, `-love`, …: the server's `reaction` value.
        reaction: String,
        target_text: String,
    },
}

impl Outbound {
    pub fn kind(&self) -> &'static str {
        match self {
            Outbound::Text { .. } => "text",
            Outbound::Attachment { .. } => "attachment",
            Outbound::React { .. } => "react",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Job {
    pub temp_guid: String,
    pub chat_guid: String,
    pub outbound: Outbound,
    pub created_at: i64,
    /// Earlier attempts; the server may already have the message.
    pub attempts: i64,
}

#[derive(Debug)]
pub(crate) enum Outcome {
    /// The server's message, with our temp GUID set.
    Sent(Box<Message>),
    Failed(String),
    /// The server is unreachable; try again when it is back.
    Offline,
}

pub(crate) async fn send(api: &Client, job: &Job, private_api: bool, give_up_at: i64) -> Outcome {
    if job.attempts > 0
        && let Some(m) = find_echo(api, job).await
    {
        return Outcome::Sent(Box::new(m));
    }
    let method = if private_api {
        "private-api"
    } else {
        "apple-script"
    };
    let result = match &job.outbound {
        Outbound::Text {
            text,
            subject,
            effect,
            reply_to,
        } => {
            // Subjects, effects and replies need the Private API.
            let extras = private_api;
            api.send_text(&SendText {
                chat_guid: job.chat_guid.clone(),
                temp_guid: job.temp_guid.clone(),
                message: text.clone(),
                method: method.into(),
                subject: subject.clone().filter(|_| extras),
                effect_id: effect.clone().filter(|_| extras),
                selected_message_guid: reply_to.as_ref().filter(|_| extras).map(|r| r.0.clone()),
                part_index: reply_to.as_ref().filter(|_| extras).map(|r| r.1),
            })
            .await
        }
        Outbound::Attachment {
            path,
            name,
            mime,
            reply_to,
        } => {
            let data = match tokio::fs::read(path).await {
                Ok(data) => data,
                Err(e) => return Outcome::Failed(format!("could not read {name}: {e}")),
            };
            api.send_attachment(SendAttachment {
                chat_guid: job.chat_guid.clone(),
                temp_guid: job.temp_guid.clone(),
                file_name: name.clone(),
                mime_type: mime.clone(),
                data,
                method: method.into(),
                selected_message_guid: reply_to
                    .as_ref()
                    .filter(|_| private_api)
                    .map(|r| r.0.clone()),
                part_index: reply_to.as_ref().filter(|_| private_api).map(|r| r.1),
                is_audio_message: false,
            })
            .await
        }
        Outbound::React {
            target,
            part,
            reaction,
            target_text,
        } => {
            api.react(&job.chat_guid, target, target_text, reaction, *part)
                .await
        }
    };
    match result {
        Ok(mut m) => {
            m.temp_guid.get_or_insert_with(|| job.temp_guid.clone());
            Outcome::Sent(Box::new(m))
        }
        Err(e) if e.is_transient() => {
            // The reply was lost, but the message may have gone out.
            if let Some(m) = find_echo(api, job).await {
                return Outcome::Sent(Box::new(m));
            }
            if matches!(e, ApiError::Network(_)) && now_ms() < give_up_at {
                Outcome::Offline
            } else {
                Outcome::Failed(e.to_string())
            }
        }
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

/// Our message on the server, if the send went through without a reply.
async fn find_echo(api: &Client, job: &Job) -> Option<Message> {
    let mut query = MessageQuery::new(25);
    query.chat_guid = Some(job.chat_guid.clone());
    query.after = Some(job.created_at - 60_000);
    query.sort = "DESC".into();
    let page = api.query_messages(&query).await.ok()?;
    let mut found = page
        .items
        .into_iter()
        .find(|m| m.is_from_me && matches(job, m))?;
    found.temp_guid = Some(job.temp_guid.clone());
    Some(found)
}

fn matches(job: &Job, m: &Message) -> bool {
    match &job.outbound {
        Outbound::Text { text, .. } => {
            m.associated_message_guid.is_none()
                && m.text.as_deref().map(str::trim) == Some(text.trim())
        }
        Outbound::Attachment { name, .. } => m
            .attachments
            .iter()
            .any(|a| a.transfer_name.as_deref() == Some(name)),
        Outbound::React {
            target, reaction, ..
        } => {
            m.associated_message_guid
                .as_deref()
                .is_some_and(|g| bb_store::split_part_guid(g).0 == *target)
                && m.associated_message_type.as_deref() == Some(reaction)
        }
    }
}

/// A file's MIME type from its extension.
pub(crate) fn mime_for(name: &str) -> &'static str {
    let ext = name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "webp" => "image/webp",
        "tif" | "tiff" => "image/tiff",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "flac" => "audio/flac",
        "caf" => "audio/x-caf",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "txt" => "text/plain",
        "vcf" => "text/vcard",
        "json" => "application/json",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "pages" => "application/vnd.apple.pages",
        "key" => "application/vnd.apple.keynote",
        "numbers" => "application/vnd.apple.numbers",
        _ => "application/octet-stream",
    }
}
