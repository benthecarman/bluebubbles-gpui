//! Attachment downloads. The server sends no event when a file reaches
//! the Mac, so a failed download is retried with backoff. A finished one is
//! written to the disk cache under its GUID and recorded in the store,
//! which re-renders its bubble.
//!
//! A few run at once and the last asked for goes first: opening a chat
//! asks for its photos oldest to newest, so the ones on screen at the
//! bottom come first instead of sharing the connection with every photo
//! further up, and a newly opened chat goes ahead of the last one.

use bb_store::AttachmentRow;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;
use tokio::time::Instant;

/// Attempts before giving up; asking again starts over.
const MAX_ATTEMPTS: u32 = 6;
/// Downloads at once.
pub(crate) const PARALLEL: usize = 3;

/// Downloads waiting, in flight or waiting to retry, by attachment GUID.
#[derive(Default)]
pub(crate) struct Downloads {
    pending: HashMap<String, Pending>,
    /// Waiting for a free slot; the last is next.
    queue: Vec<String>,
    running: HashSet<String>,
}

struct Pending {
    attempts: u32,
    /// `None` while waiting or in flight.
    retry_at: Option<Instant>,
}

impl Downloads {
    /// Asks for `guid`. One already waiting moves to the front of the
    /// queue; returns whether this is a new download.
    pub fn begin(&mut self, guid: &str) -> bool {
        if let Some(i) = self.queue.iter().position(|g| g == guid) {
            let g = self.queue.remove(i);
            self.queue.push(g);
            return false;
        }
        if self.pending.contains_key(guid) {
            return false;
        }
        self.pending.insert(
            guid.to_owned(),
            Pending {
                attempts: 0,
                retry_at: None,
            },
        );
        self.queue.push(guid.to_owned());
        true
    }

    /// The next download to start, while fewer than [`PARALLEL`] run.
    pub fn next(&mut self) -> Option<String> {
        if self.running.len() >= PARALLEL {
            return None;
        }
        let guid = self.queue.pop()?;
        self.running.insert(guid.clone());
        Some(guid)
    }

    pub fn finished(&mut self, guid: &str) {
        self.running.remove(guid);
        self.pending.remove(guid);
    }

    /// Records a failed attempt; returns whether it will be retried.
    pub fn failed(&mut self, guid: &str, now: Instant, base: Duration) -> bool {
        self.running.remove(guid);
        let Some(p) = self.pending.get_mut(guid) else {
            return false;
        };
        p.attempts += 1;
        if p.attempts >= MAX_ATTEMPTS {
            self.pending.remove(guid);
            return false;
        }
        p.retry_at = Some(now + backoff(base, p.attempts));
        true
    }

    /// Queues the downloads whose retry is due, behind the ones already
    /// waiting.
    pub fn retry_due(&mut self, now: Instant) {
        for (guid, p) in &mut self.pending {
            if p.retry_at.is_some_and(|at| at <= now) {
                p.retry_at = None;
                self.queue.insert(0, guid.clone());
            }
        }
    }
}

/// `base`, doubling per attempt, at most 30 × `base`.
fn backoff(base: Duration, attempts: u32) -> Duration {
    (base * 2u32.saturating_pow(attempts.saturating_sub(1))).min(base * 30)
}

/// The cache file's name: the GUID, made safe for a path, and an
/// extension so other apps can open it. The server's own name is not used
/// on disk.
pub(crate) fn file_name(a: &AttachmentRow) -> String {
    let stem: String = a
        .guid
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{stem}.{}", extension(a))
}

/// The extension of the file the server sends: it converts HEIC and TIFF
/// to JPEG and CAF (voice messages) to MP3.
fn extension(a: &AttachmentRow) -> &'static str {
    let named = a
        .transfer_name
        .as_deref()
        .and_then(|n| Path::new(n).extension())
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    let from_mime = match a.mime_type.as_deref().unwrap_or_default() {
        "image/heic" | "image/heif" | "image/tiff" | "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/quicktime" => "mov",
        "video/mp4" => "mp4",
        "audio/x-caf" | "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "application/pdf" => "pdf",
        "text/vcard" | "text/x-vcard" => "vcf",
        _ => "bin",
    };
    match named.as_deref() {
        Some("heic" | "heif" | "tif" | "tiff" | "jpeg" | "jpg") => "jpg",
        Some("caf") => "mp3",
        Some(e) => KNOWN
            .iter()
            .find(|k| **k == e)
            .copied()
            .unwrap_or(from_mime),
        None => from_mime,
    }
}

/// Extensions kept as the server named them.
const KNOWN: &[&str] = &[
    "png", "gif", "webp", "mov", "mp4", "m4v", "mp3", "m4a", "wav", "aac", "pdf", "vcf", "txt",
    "zip", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "pages", "numbers", "key", "csv", "json",
    "html", "rtf", "svg", "ics",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(guid: &str, name: Option<&str>, mime: Option<&str>) -> AttachmentRow {
        AttachmentRow {
            guid: guid.into(),
            transfer_name: name.map(Into::into),
            mime_type: mime.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn names_files_by_guid_with_the_served_extension() {
        let name = |g, n, m| file_name(&attachment(g, n, m));
        assert_eq!(
            name("at_0_ABC-1", Some("IMG_1.HEIC"), Some("image/heic")),
            "at_0_ABC-1.jpg"
        );
        assert_eq!(name("g/../x", Some("a b.pdf"), None), "g____x.pdf");
        assert_eq!(name("g", Some("Audio.caf"), None), "g.mp3");
        assert_eq!(name("g", Some("evil.sh"), None), "g.bin");
        assert_eq!(name("g", None, Some("video/quicktime")), "g.mov");
    }

    #[test]
    fn retries_with_backoff_then_gives_up() {
        let base = Duration::from_secs(1);
        let now = Instant::now();
        let mut d = Downloads::default();
        let due = |d: &mut Downloads, at| {
            d.retry_due(at);
            d.next()
        };
        assert!(d.begin("g"));
        assert!(!d.begin("g"));
        assert_eq!(d.next().as_deref(), Some("g"));
        assert!(d.failed("g", now, base));
        assert_eq!(due(&mut d, now), None);
        assert_eq!(
            due(&mut d, now + Duration::from_secs(1)).as_deref(),
            Some("g")
        );
        assert!(d.failed("g", now, base));
        assert_eq!(due(&mut d, now + Duration::from_secs(1)), None);
        assert!(due(&mut d, now + Duration::from_secs(2)).is_some());
        for _ in 2..MAX_ATTEMPTS - 1 {
            assert!(d.failed("g", now, base));
        }
        assert!(!d.failed("g", now, base));
        assert!(d.begin("g"), "asking again starts over");
    }

    #[test]
    fn a_few_at_once_and_the_newest_first() {
        let mut d = Downloads::default();
        for g in ["old", "a", "b", "c", "new"] {
            d.begin(g);
        }
        let started: Vec<String> = std::iter::from_fn(|| d.next()).collect();
        assert_eq!(started, ["new", "c", "b"]);
        assert_eq!(started.len(), PARALLEL);
        // Asking again for a waiting one moves it up.
        d.begin("old");
        d.finished("new");
        assert_eq!(d.next().as_deref(), Some("old"));
        assert_eq!(d.next(), None);
        d.finished("c");
        assert_eq!(d.next().as_deref(), Some("a"));
    }
}
