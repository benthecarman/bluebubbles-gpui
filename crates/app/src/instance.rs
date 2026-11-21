//! One running copy. The first claims a socket in the runtime directory;
//! a second launch (the app grid, a dock icon) sends it "show" with the
//! launcher's activation token and exits, so the window comes back from
//! the tray instead of a second engine opening the same cache. On Windows
//! the socket is a loopback port, named in a file in the data directory.

use futures::channel::mpsc::{UnboundedReceiver, unbounded};

pub enum Claim {
    /// This is the running copy; requests to show the window arrive here,
    /// each with the activation token its launch was given.
    Primary(UnboundedReceiver<Option<String>>),
    /// Another copy runs and was asked to show itself.
    Handed,
}

#[cfg(unix)]
pub use unix::claim;
#[cfg(windows)]
pub use windows::claim;

#[cfg(unix)]
mod unix {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;

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
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::time::Duration;

    /// What the running copy answers, so a port some other program took
    /// over since isn't mistaken for it.
    const HELLO: &str = "bluebubbles-gpui";

    fn port_file() -> PathBuf {
        crate::config::data_dir().join("instance")
    }

    /// The running copy, as "port pid".
    fn running() -> Option<(u16, u32)> {
        let text = std::fs::read_to_string(port_file()).ok()?;
        let (port, pid) = text.trim().split_once(' ')?;
        Some((port.parse().ok()?, pid.parse().ok()?))
    }

    fn hand_over(port: u16, pid: u32) -> Option<()> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(500)).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        let mut hello = String::new();
        BufReader::new(stream.try_clone().ok()?)
            .read_line(&mut hello)
            .ok()?;
        if hello.trim_end() != HELLO {
            return None;
        }
        // This launch may bring a window forward (the user just started
        // it); let the running copy do so instead.
        unsafe {
            let _ = ::windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
        }
        stream.write_all(b"show\n").ok()
    }

    pub fn claim() -> Claim {
        if let Some((port, pid)) = running()
            && hand_over(port, pid).is_some()
        {
            return Claim::Handed;
        }
        let (tx, rx) = unbounded();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).and_then(|listener| {
            let port = listener.local_addr()?.port();
            let file = port_file();
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&file, format!("{port} {}", std::process::id()))?;
            Ok(listener)
        });
        match listener {
            Ok(listener) => {
                std::thread::Builder::new()
                    .name("bb-instance".into())
                    .spawn(move || {
                        for mut stream in listener.incoming().flatten() {
                            if stream.write_all(format!("{HELLO}\n").as_bytes()).is_err() {
                                continue;
                            }
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                            let mut line = String::new();
                            if BufReader::new(stream).read_line(&mut line).is_err()
                                || line.trim_end() != "show"
                            {
                                continue;
                            }
                            if tx.unbounded_send(None).is_err() {
                                break;
                            }
                        }
                    })
                    .ok();
            }
            Err(e) => {
                tracing::warn!("can't claim a port: {e}; a second launch opens a second copy")
            }
        }
        Claim::Primary(rx)
    }
}
