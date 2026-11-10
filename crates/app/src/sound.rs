//! The sound a notification makes: one of the app's own (made by
//! `scripts/make-sounds.py`), the desktop theme's, or none. The desktop
//! plays it, from a file the notification names, so it follows Do Not
//! Disturb and the per-app sound setting.

use crate::config::APP_ID;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sound {
    #[default]
    Chime,
    Marimba,
    Glass,
    Pop,
    /// The theme's `message-new-instant`.
    System,
    None,
}

impl Sound {
    pub const ALL: [Sound; 6] = [
        Sound::Chime,
        Sound::Marimba,
        Sound::Glass,
        Sound::Pop,
        Sound::System,
        Sound::None,
    ];

    /// Its name in the config file.
    pub fn key(self) -> &'static str {
        match self {
            Sound::Chime => "chime",
            Sound::Marimba => "marimba",
            Sound::Glass => "glass",
            Sound::Pop => "pop",
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
            Sound::System | Sound::None => return None,
        })
    }

    /// The app's own sound as a file the desktop can play, written to the
    /// cache the first time.
    pub fn file(self) -> Option<PathBuf> {
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
}
