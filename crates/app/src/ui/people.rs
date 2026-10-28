//! Faces for people and chats: contact photos, the photo someone shares
//! through iMessage, initials, or the person glyph.

use bb_core::{Names, Photo};
use bb_store::{ChatRow, Store};
use bb_ui::Face;
use gpui_kit::{Image, ImageFormat};
use std::collections::HashMap;
use std::sync::Arc;

/// Photos by where they come from.
#[derive(Default, Clone)]
pub struct Photos(HashMap<Photo, Arc<Image>>);

impl Photos {
    /// Loads the photos of `addresses` not loaded yet. Runs off the UI
    /// thread.
    pub fn load_more<'a>(
        &self,
        store: &Store,
        names: &Names,
        addresses: impl IntoIterator<Item = &'a str>,
    ) -> Photos {
        let mut out = self.clone();
        for address in addresses {
            let Some(photo) = names.photo(address) else {
                continue;
            };
            if out.0.contains_key(&photo) {
                continue;
            }
            if let Some(bytes) = photo.load(store)
                && let Some(format) = format_of(&bytes)
            {
                out.0
                    .insert(photo, Arc::new(Image::from_bytes(format, bytes)));
            }
        }
        out
    }
}

pub fn format_of(bytes: &[u8]) -> Option<ImageFormat> {
    match bytes {
        [0xFF, 0xD8, ..] => Some(ImageFormat::Jpeg),
        [0x89, b'P', b'N', b'G', ..] => Some(ImageFormat::Png),
        [b'G', b'I', b'F', ..] => Some(ImageFormat::Gif),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some(ImageFormat::Webp),
        _ => None,
    }
}

pub fn face(names: &Names, photos: &Photos, address: &str) -> Face {
    if let Some(photo) = names.photo(address)
        && let Some(image) = photos.0.get(&photo)
    {
        return Face::Image(image.clone());
    }
    match names.initials(address) {
        Some(initials) => Face::Initials(initials.into()),
        None => Face::Glyph,
    }
}

/// One face for a direct chat, the first two members' for a group.
pub fn chat_faces(chat: &ChatRow, names: &Names, photos: &Photos) -> Vec<Face> {
    if chat.is_group() {
        let faces: Vec<Face> = chat
            .participants
            .iter()
            .take(2)
            .map(|a| face(names, photos, a))
            .collect();
        if faces.len() >= 2 {
            return faces;
        }
    }
    vec![
        chat.participants
            .first()
            .map(|a| face(names, photos, a))
            .unwrap_or(Face::Glyph),
    ]
}

/// The service a chat sends with: from its GUID, or (for macOS 26's
/// `any;` GUIDs) from its latest message.
pub fn chat_service(chat: &ChatRow) -> String {
    let from_guid = chat.service();
    if !from_guid.eq_ignore_ascii_case("any") {
        return from_guid.to_owned();
    }
    chat.last_message
        .as_ref()
        .and_then(|m| m.service.clone())
        .unwrap_or_else(|| "iMessage".into())
}

pub fn is_sms(service: &str) -> bool {
    service.eq_ignore_ascii_case("SMS") || service.eq_ignore_ascii_case("RCS")
}
