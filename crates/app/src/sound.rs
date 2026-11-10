//! The sound a notification makes: one of the app's own (made by
//! `scripts/make-sounds.py`), one the user picked, the desktop theme's,
//! or none. The desktop plays it, from a file the notification names, so
//! it follows Do Not Disturb and the per-app sound setting.

use crate::config::{self, APP_ID};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sound {
    #[default]
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

/// The longest a picked sound plays.
const CUSTOM_MAX_SECONDS: &str = "8";

impl Sound {
    pub const ALL: [Sound; 7] = [
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
        Self::ALL.into_iter().find(|s| s.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
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
                let played = Command::new(&args[0])
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

/// Converts `source` to Ogg Vorbis at `dest`; see [`Sound::import`].
fn convert(source: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    let part = dest.with_extension("part.oga");
    let converted = Command::new("ffmpeg")
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
        for s in Sound::ALL {
            assert_eq!(Sound::from_key(s.key()), Some(s));
        }
        assert_eq!(Sound::from_key("tri-tone"), None);
        let file = Sound::Pop.file().unwrap();
        assert!(std::fs::read(&file).unwrap().starts_with(b"OggS"));
        assert_eq!(Sound::System.file(), None);
    }

    #[test]
    fn a_picked_sound_becomes_ogg() {
        let dir = std::env::temp_dir().join(format!("bb-sound-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("Rebound.m4a");
        let made = Command::new("ffmpeg")
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
        let probe = Command::new("ffprobe")
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
