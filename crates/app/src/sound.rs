//! The sound a notification makes: Messages' own where the build has it
//! (see `build.rs`), one of the app's own (made by
//! `scripts/make-sounds.py`), one the user picked, the desktop theme's,
//! or none. The desktop plays it, from a file the notification names, so
//! it follows Do Not Disturb and the per-app sound setting.

use crate::config::{self, APP_ID};
use std::path::{Path, PathBuf};
use std::process::Stdio;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    /// Messages' received-message sound, in builds that have it.
    Messages,
    Chime,
    Marimba,
    Glass,
    Pop,
    /// A file the user picked, converted to Ogg Vorbis.
    Custom,
    /// The theme's `message-new-instant`.
    System,
    None,
}

/// Messages' sound where the build has it, else Chime.
impl Default for Sound {
    fn default() -> Self {
        if cfg!(messages_sound) {
            Sound::Messages
        } else {
            Sound::Chime
        }
    }
}

/// The longest a picked sound plays.
const CUSTOM_MAX_SECONDS: &str = "8";

impl Sound {
    pub const ALL: [Sound; 8] = [
        Sound::Messages,
        Sound::Chime,
        Sound::Marimba,
        Sound::Glass,
        Sound::Pop,
        Sound::System,
        Sound::None,
        Sound::Custom,
    ];

    /// Its name in the config file.
    pub fn key(self) -> &'static str {
        match self {
            Sound::Messages => "messages",
            Sound::Chime => "chime",
            Sound::Marimba => "marimba",
            Sound::Glass => "glass",
            Sound::Pop => "pop",
            Sound::Custom => "custom",
            Sound::System => "system",
            Sound::None => "none",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::choices().find(|s| s.key() == key)
    }

    /// The sounds this build offers.
    pub fn choices() -> impl Iterator<Item = Sound> {
        Self::ALL
            .into_iter()
            .filter(|s| *s != Sound::Messages || cfg!(messages_sound))
    }

    pub fn label(self) -> &'static str {
        match self {
            Sound::Messages => "Messages",
            Sound::Chime => "Chime",
            Sound::Marimba => "Marimba",
            Sound::Glass => "Glass",
            Sound::Pop => "Pop",
            Sound::Custom => "Custom…",
            Sound::System => "System",
            Sound::None => "None",
        }
    }

    fn bytes(self) -> Option<&'static [u8]> {
        Some(match self {
            #[cfg(messages_sound)]
            Sound::Messages => include_bytes!("../../../assets/private/received-message.oga"),
            #[cfg(not(messages_sound))]
            Sound::Messages => return None,
            Sound::Chime => include_bytes!("../../../assets/sounds/chime.oga"),
            Sound::Marimba => include_bytes!("../../../assets/sounds/marimba.oga"),
            Sound::Glass => include_bytes!("../../../assets/sounds/glass.oga"),
            Sound::Pop => include_bytes!("../../../assets/sounds/pop.oga"),
            Sound::Custom | Sound::System | Sound::None => return None,
        })
    }

    /// The picked sound, once there is one.
    fn custom_file() -> PathBuf {
        config::data_dir().join("custom-sound.oga")
    }

    /// Takes `source` (any audio ffmpeg reads: an iPhone `.m4r`, a Mac
    /// `.caf`, MP3) as the custom sound, in Ogg Vorbis since that and WAV
    /// are all the desktop's sound player reads. Without ffmpeg only Ogg
    /// and WAV files can be used.
    pub fn import(source: &Path) -> Result<(), String> {
        convert(source, &Self::custom_file())
    }

    /// The sound as a file the desktop can play: the app's own is
    /// written to the cache the first time.
    pub fn file(self) -> Option<PathBuf> {
        if self == Sound::Custom {
            return Some(Self::custom_file()).filter(|f| f.exists());
        }
        let bytes = self.bytes()?;
        let path = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(APP_ID)
            .join("sounds")
            .join(format!("{}.oga", self.key()));
        let current = std::fs::metadata(&path).is_ok_and(|m| m.len() == bytes.len() as u64);
        if !current {
            std::fs::create_dir_all(path.parent()?).ok()?;
            let part = path.with_extension("part");
            std::fs::write(&part, bytes).ok()?;
            std::fs::rename(&part, &path).ok()?;
        }
        Some(path)
    }

    /// Plays it here, for choosing one in Settings, with whichever player
    /// the desktop has. Quietly does nothing without one.
    #[cfg(not(windows))]
    pub fn preview(self) {
        let file = self.file();
        std::thread::spawn(move || {
            let tries: Vec<Vec<String>> = match (self, file) {
                (Sound::None, _) => return,
                (_, Some(f)) => {
                    let f = f.to_string_lossy().into_owned();
                    vec![
                        vec!["pw-play".into(), f.clone()],
                        vec!["paplay".into(), f.clone()],
                        vec!["canberra-gtk-play".into(), "-f".into(), f],
                    ]
                }
                (_, None) => vec![vec![
                    "canberra-gtk-play".into(),
                    "-i".into(),
                    "message-new-instant".into(),
                ]],
            };
            for args in tries {
                let played = crate::process::command(&args[0])
                    .args(&args[1..])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                if played.is_ok_and(|s| s.success()) {
                    return;
                }
            }
        });
    }
}

/// Windows plays notification sounds itself only from WAV files, and
/// its toasts play only its own sounds, so the app plays the sound.
#[cfg(windows)]
impl Sound {
    /// Plays it for a notification or a preview, unless it's `None`.
    pub fn play(self) {
        use windows::Win32::Media::Audio::{
            PlaySoundW, SND_ALIAS, SND_ASYNC, SND_FILENAME, SND_NODEFAULT,
        };
        use windows::core::HSTRING;
        let played = match self {
            Sound::None => return,
            // The sound Windows' own messaging apps make.
            Sound::System => unsafe {
                PlaySoundW(
                    &HSTRING::from("Notification.IM"),
                    None,
                    SND_ALIAS | SND_ASYNC,
                )
            },
            _ => {
                let Some(wav) = self.wav() else {
                    return;
                };
                unsafe {
                    PlaySoundW(
                        &HSTRING::from(wav.as_os_str()),
                        None,
                        SND_FILENAME | SND_ASYNC | SND_NODEFAULT,
                    )
                }
            }
        };
        if !played.as_bool() {
            tracing::debug!("couldn't play the {} sound", self.key());
        }
    }

    pub fn preview(self) {
        std::thread::spawn(move || self.play());
    }

    /// The sound as a WAV file beside its Ogg one, made when the Ogg one
    /// is newer. A picked WAV file is used as it is.
    fn wav(self) -> Option<PathBuf> {
        let source = self.file()?;
        let bytes = std::fs::read(&source).ok()?;
        if bytes.starts_with(b"RIFF") {
            return Some(source);
        }
        let wav = source.with_extension("wav");
        let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        if modified(&wav) < modified(&source) {
            let part = wav.with_extension("part.wav");
            std::fs::write(&part, ogg_to_wav(&bytes)?).ok()?;
            std::fs::rename(&part, &wav).ok()?;
        }
        Some(wav)
    }
}

/// Decodes Ogg Vorbis into a 16-bit PCM WAV file's bytes.
#[cfg(windows)]
fn ogg_to_wav(ogg: &[u8]) -> Option<Vec<u8>> {
    let mut reader = lewton::inside_ogg::OggStreamReader::new(std::io::Cursor::new(ogg)).ok()?;
    let channels = u16::from(reader.ident_hdr.audio_channels);
    let rate = reader.ident_hdr.audio_sample_rate;
    let mut samples: Vec<i16> = Vec::new();
    while let Some(packet) = reader.read_dec_packet_itl().ok()? {
        samples.extend(packet);
    }
    Some(wav_bytes(&samples, channels, rate))
}

/// `samples` (interleaved) as a WAV file.
#[cfg_attr(not(windows), allow(dead_code))]
fn wav_bytes(samples: &[i16], channels: u16, rate: u32) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Converts `source` to Ogg Vorbis at `dest`; see [`Sound::import`].
fn convert(source: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    let part = dest.with_extension("part.oga");
    let converted = crate::process::command("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y", "-i"])
        .arg(source)
        .args([
            "-vn",
            "-t",
            CUSTOM_MAX_SECONDS,
            "-c:a",
            "libvorbis",
            "-q:a",
            "6",
        ])
        .args(["-map_metadata", "-1"])
        .arg(&part)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match converted {
        Ok(s) if s.success() => {}
        Ok(_) => {
            std::fs::remove_file(&part).ok();
            return Err("That file isn't a sound ffmpeg can read".into());
        }
        Err(_) => {
            let playable = source
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .is_some_and(|e| matches!(e.as_str(), "oga" | "ogg" | "wav"));
            if !playable {
                return Err("Install ffmpeg to use this kind of file, or pick an \
                                .ogg or .wav"
                    .into());
            }
            std::fs::copy(source, &part).map_err(|e| e.to_string())?;
        }
    }
    std::fs::rename(&part, dest).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_files_are_written() {
        for s in Sound::choices() {
            assert_eq!(Sound::from_key(s.key()), Some(s));
        }
        assert_eq!(
            Sound::choices().any(|s| s == Sound::Messages),
            cfg!(messages_sound)
        );
        assert_eq!(Sound::from_key("tri-tone"), None);
        let file = Sound::Pop.file().unwrap();
        assert!(std::fs::read(&file).unwrap().starts_with(b"OggS"));
        assert_eq!(Sound::System.file(), None);
    }

    #[test]
    fn wav_header_describes_the_samples() {
        let wav = wav_bytes(&[0, 1, -1, 2], 2, 48000);
        assert_eq!(wav.len(), 44 + 8);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 36 + 8);
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 2);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 48000);
        assert_eq!(u32::from_le_bytes(wav[28..32].try_into().unwrap()), 192000);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(&wav[44..], &[0, 0, 1, 0, 0xff, 0xff, 2, 0]);
    }

    #[test]
    fn a_picked_sound_becomes_ogg() {
        let dir = std::env::temp_dir().join(format!("bb-sound-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("Rebound.m4a");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=f=880:d=20"])
            .arg(&source)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("skipped: no ffmpeg");
            return;
        }
        let dest = dir.join("custom.oga");
        convert(&source, &dest).unwrap();
        assert!(std::fs::read(&dest).unwrap().starts_with(b"OggS"));
        // Cut to the longest a notification sound plays.
        let probe = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "csv=p=0",
            ])
            .arg(&dest)
            .output()
            .unwrap();
        let seconds: f64 = String::from_utf8_lossy(&probe.stdout)
            .trim()
            .parse()
            .unwrap();
        assert!(seconds <= 8.1, "{seconds}");
        std::fs::write(&source, b"not a sound").unwrap();
        assert!(convert(&source, &dest).is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
