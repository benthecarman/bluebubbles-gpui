//! One running copy. The first claims a socket in the runtime directory;
//! a second launch (the app grid, a dock icon) sends it "show" with the
//! launcher's activation token and exits, so the window comes back from
//! the tray instead of a second engine opening the same cache.

use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

pub enum Claim {
    /// This is the running copy; requests to show the window arrive here,
    /// each with the activation token its launch was given.
    Primary(UnboundedReceiver<Option<String>>),
    /// Another copy runs and was asked to show itself.
    Handed,
}

fn socket_path() -> PathBuf {
    match dirs::runtime_dir() {
        Some(dir) => dir.join(format!("{}.sock", crate::config::APP_ID)),
        None => {
            use std::os::unix::fs::MetadataExt;
            let uid = std::fs::metadata("/proc/self").map_or(0, |m| m.uid());
            std::env::temp_dir().join(format!("{}-{uid}.sock", crate::config::APP_ID))
        }
    }
}

/// The token a launcher hands a new process: xdg-activation on Wayland,
/// startup notification on X11.
fn activation_token() -> Option<String> {
    ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

pub fn claim() -> Claim {
    claim_at(socket_path(), activation_token())
}

fn claim_at(path: PathBuf, token: Option<String>) -> Claim {
    if let Ok(mut stream) = UnixStream::connect(&path) {
        let line = match token {
            Some(token) => format!("show {token}\n"),
            None => "show\n".into(),
        };
        if stream.write_all(line.as_bytes()).is_ok() {
            return Claim::Handed;
        }
    }
    // Nobody answered: a socket left by a copy that crashed.
    let _ = std::fs::remove_file(&path);
    let (tx, rx) = unbounded();
    match UnixListener::bind(&path) {
        Ok(listener) => {
            std::thread::Builder::new()
                .name("bb-instance".into())
                .spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let mut line = String::new();
                        if BufReader::new(stream).read_line(&mut line).is_err() {
                            continue;
                        }
                        let Some(rest) = line.trim_end().strip_prefix("show") else {
                            continue;
                        };
                        let token = Some(rest.trim()).filter(|t| !t.is_empty());
                        if tx.unbounded_send(token.map(str::to_owned)).is_err() {
                            break;
                        }
                    }
                })
                .ok();
        }
        Err(e) => tracing::warn!(
            "can't claim {}: {e}; a second launch opens a second copy",
            path.display()
        ),
    }
    Claim::Primary(rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn a_second_launch_hands_over_its_token() {
        let path =
            std::env::temp_dir().join(format!("bb-instance-test-{}.sock", std::process::id()));
        let Claim::Primary(mut shows) = claim_at(path.clone(), None) else {
            panic!("the first launch is the primary");
        };
        assert!(matches!(
            claim_at(path.clone(), Some("tok".into())),
            Claim::Handed
        ));
        assert!(matches!(claim_at(path.clone(), None), Claim::Handed));
        let got = futures::executor::block_on(async {
            (shows.next().await.unwrap(), shows.next().await.unwrap())
        });
        assert_eq!(got, (Some("tok".into()), None));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_stale_socket_is_taken_over() {
        let path =
            std::env::temp_dir().join(format!("bb-instance-stale-{}.sock", std::process::id()));
        drop(UnixListener::bind(&path).unwrap());
        assert!(matches!(claim_at(path.clone(), None), Claim::Primary(_)));
        std::fs::remove_file(path).ok();
    }
}
