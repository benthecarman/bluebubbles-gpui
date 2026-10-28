//! Desktop notifications through the freedesktop interface that GNOME,
//! KDE and other desktops serve on the session bus. Each chat has at most
//! one notification: a new message replaces it, and reading the chat
//! anywhere withdraws it.

use crate::config::APP_ID;
use bb_core::{Names, Notification, Region};
use bb_store::Store;
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::channel::oneshot;
use std::collections::HashMap;
use zbus::zvariant::{Structure, Value};

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications",
    gen_blocking = false
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    fn get_capabilities(&self) -> zbus::Result<Vec<String>>;

    fn get_server_information(&self) -> zbus::Result<(String, String, String, String)>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;

    /// Sent just before `ActionInvoked`: a token that lets the app raise
    /// its window on Wayland.
    #[zbus(signal)]
    fn activation_token(&self, id: u32, activation_token: String) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}

/// Longest side of the picture sent with a notification.
const PICTURE: u32 = 128;
const OPEN: &str = "default";
/// The chat id recorded for the test notification: clicking it just
/// shows the window.
pub const NO_CHAT: i64 = -1;
const MARK_READ: &str = "mark-read";

/// What the user did with a notification.
#[derive(Debug)]
pub enum Response {
    Open {
        chat_id: i64,
        activation_token: Option<String>,
    },
    MarkRead {
        chat_id: i64,
    },
}

enum Request {
    Show(Notification, Option<Picture>),
    Withdraw(i64),
    /// Sends a sample notification; answers with the notification
    /// server's name and version, or why it couldn't.
    Test(oneshot::Sender<Result<String, String>>),
}

/// An RGBA picture for the `image-data` hint.
struct Picture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Sends notifications; dropping it stops the worker.
pub struct Notifier {
    requests: UnboundedSender<Request>,
    store: Store,
    region: Region,
}

impl Notifier {
    /// Starts the worker on `executor`; responses arrive on the returned
    /// receiver. Without a notification server it logs once and drops
    /// requests.
    pub fn start(
        store: Store,
        region: Region,
        executor: &gpui_kit::BackgroundExecutor,
    ) -> (Self, UnboundedReceiver<Response>) {
        let (requests, rx) = unbounded();
        let (responses, responses_rx) = unbounded();
        executor
            .spawn(async move {
                if let Err(e) = run(rx, responses).await {
                    tracing::warn!("desktop notifications are off: {e}");
                }
            })
            .detach();
        (
            Self {
                requests,
                store,
                region,
            },
            responses_rx,
        )
    }

    /// Shows `n` with the sender's photo, read off the UI thread.
    pub fn show(&self, n: Notification, executor: &gpui_kit::BackgroundExecutor) {
        let (store, region, requests) = (self.store.clone(), self.region, self.requests.clone());
        executor
            .spawn(async move {
                let picture = n.sender.as_deref().and_then(|s| picture(&store, region, s));
                let _ = requests.unbounded_send(Request::Show(n, picture));
            })
            .detach();
    }

    pub fn withdraw(&self, chat_id: i64) {
        let _ = self.requests.unbounded_send(Request::Withdraw(chat_id));
    }

    /// Sends a sample notification. Resolves to the desktop's notification
    /// server ("gnome-shell 50.0"), or the reason none showed.
    pub async fn test(&self) -> Result<String, String> {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.unbounded_send(Request::Test(tx));
        rx.await
            .unwrap_or_else(|_| Err("The desktop's notification service can't be reached".into()))
    }
}

/// The contact photo of `address`, scaled for a notification.
fn picture(store: &Store, region: Region, address: &str) -> Option<Picture> {
    let names = Names::load(store, region, [address]).ok()?;
    let contact = names.contact(address)?;
    if !contact.has_avatar {
        return None;
    }
    let bytes = store.contact_avatar(&contact.id).ok()??;
    let image = image::load_from_memory(&bytes)
        .ok()?
        .thumbnail(PICTURE, PICTURE)
        .to_rgba8();
    Some(Picture {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

async fn run(
    mut requests: UnboundedReceiver<Request>,
    responses: UnboundedSender<Response>,
) -> zbus::Result<()> {
    let connection = zbus::Connection::session().await?;
    let proxy = NotificationsProxy::new(&connection).await?;
    match proxy.get_server_information().await {
        Ok((name, _, version, spec)) => {
            tracing::info!("notifications go to {name} {version} (spec {spec})")
        }
        Err(e) => tracing::warn!("no notification server yet: {e}"),
    }
    // Asked when first needed: the notification server may start later
    // than the app.
    let mut capabilities: Option<Vec<String>> = None;
    let mut invoked = proxy.receive_action_invoked().await?.fuse();
    let mut tokens = proxy.receive_activation_token().await?.fuse();
    let mut closed = proxy.receive_notification_closed().await?.fuse();
    // Notification ids by chat, and the reverse.
    let mut shown: HashMap<i64, u32> = HashMap::new();
    let mut chats: HashMap<u32, i64> = HashMap::new();
    let mut token_for: HashMap<u32, String> = HashMap::new();
    loop {
        futures::select! {
            request = requests.next() => match request {
                None => return Ok(()),
                Some(Request::Show(n, picture)) => {
                    if capabilities.is_none() {
                        capabilities = proxy.get_capabilities().await.ok();
                    }
                    let has = |c: &str| capabilities.iter().flatten().any(|x| x == c);
                    let (markup, actions) = (has("body-markup"), has("actions"));
                    let replaces = shown.get(&n.chat_id).copied().unwrap_or(0);
                    let (summary, body) = text(&n, markup);
                    let mut hints = base_hints();
                    if let Some(p) = picture {
                        let image = Structure::from((
                            p.width as i32,
                            p.height as i32,
                            p.width as i32 * 4,
                            true,
                            8i32,
                            4i32,
                            p.rgba,
                        ));
                        hints.insert("image-data", Value::Structure(image));
                    }
                    let buttons: &[&str] = if actions {
                        &[OPEN, "Open", MARK_READ, "Mark as Read"]
                    } else {
                        &[]
                    };
                    match proxy
                        .notify("BlueBubbles", replaces, APP_ID, &summary, &body, buttons, hints, -1)
                        .await
                    {
                        Ok(id) => {
                            tracing::info!("notification {id} for chat {}", n.chat_id);
                            if replaces != 0 && replaces != id {
                                chats.remove(&replaces);
                            }
                            shown.insert(n.chat_id, id);
                            chats.insert(id, n.chat_id);
                        }
                        Err(e) => tracing::warn!("notify: {e}"),
                    }
                }
                Some(Request::Test(reply)) => {
                    let server = proxy
                        .get_server_information()
                        .await
                        .map(|(name, _, version, _)| format!("{name} {version}"));
                    let actions: &[&str] = &[OPEN, "Open"];
                    let sent = proxy
                        .notify(
                            "BlueBubbles",
                            0,
                            APP_ID,
                            "Notifications are on",
                            "New messages will show like this.",
                            actions,
                            base_hints(),
                            -1,
                        )
                        .await;
                    let result = match (sent, server) {
                        (Ok(id), server) => {
                            chats.insert(id, NO_CHAT);
                            Ok(server.unwrap_or_else(|_| "the desktop".into()))
                        }
                        (Err(e), _) => Err(format!("The desktop refused it: {e}")),
                    };
                    tracing::info!("test notification: {result:?}");
                    let _ = reply.send(result);
                }
                Some(Request::Withdraw(chat_id)) => {
                    if let Some(id) = shown.remove(&chat_id) {
                        chats.remove(&id);
                        let _ = proxy.close_notification(id).await;
                    }
                }
            },
            signal = tokens.next() => {
                if let Some(args) = signal.as_ref().and_then(|s| s.args().ok()) {
                    token_for.insert(args.id, args.activation_token);
                }
            }
            signal = invoked.next() => {
                let Some(args) = signal.as_ref().and_then(|s| s.args().ok()) else {
                    continue;
                };
                let Some(&chat_id) = chats.get(&args.id) else {
                    continue;
                };
                let response = match args.action_key.as_str() {
                    OPEN => Response::Open {
                        chat_id,
                        activation_token: token_for.remove(&args.id),
                    },
                    MARK_READ => Response::MarkRead { chat_id },
                    _ => continue,
                };
                let _ = responses.unbounded_send(response);
            }
            signal = closed.next() => {
                if let Some(args) = signal.as_ref().and_then(|s| s.args().ok())
                    && let Some(chat_id) = chats.remove(&args.id)
                {
                    shown.remove(&chat_id);
                    token_for.remove(&args.id);
                }
            }
            complete => return Ok(()),
        }
    }
}

/// What every notification says about the app: GNOME finds its name
/// and icon, and its per-app settings, through the desktop entry.
fn base_hints() -> HashMap<&'static str, Value<'static>> {
    let mut hints = HashMap::new();
    hints.insert("desktop-entry", Value::from(APP_ID));
    hints.insert("category", Value::from("im.received"));
    hints.insert("sound-name", Value::from("message-new-instant"));
    hints
}

/// The summary and body: the chat's name over the message, with the
/// sender's name before it in groups.
fn text(n: &Notification, markup: bool) -> (String, String) {
    let body = match &n.subtitle {
        Some(sender) if n.is_group => format!("{sender}: {}", n.body),
        _ => n.body.clone(),
    };
    let body = if markup { escape(&body) } else { body };
    (n.title.clone(), body)
}

/// Bodies are markup where the server supports it.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(group: bool) -> Notification {
        Notification {
            chat_id: 1,
            chat_guid: "c".into(),
            message_id: 2,
            message_guid: "m".into(),
            title: if group { "Trip".into() } else { "Sam".into() },
            subtitle: group.then(|| "Sam".into()),
            body: "fish & <chips>".into(),
            sender: Some("+15550104567".into()),
            is_group: group,
        }
    }

    #[test]
    fn names_the_sender_in_groups_and_escapes_markup() {
        assert_eq!(
            text(&notification(false), true),
            ("Sam".into(), "fish &amp; &lt;chips&gt;".into())
        );
        assert_eq!(
            text(&notification(true), false),
            ("Trip".into(), "Sam: fish & <chips>".into())
        );
    }
}
