//! Toast notifications in Windows' notification center. Each chat has
//! at most one (tagged with the chat): a new message replaces it, and
//! reading the chat anywhere withdraws it. Clicking one opens the chat;
//! Mark as Read is a button on it. They follow Do Not Disturb and the
//! app's switch in Settings > System > Notifications.
//!
//! The app isn't installed as a package, so it registers a name and icon
//! for its notifications under `HKCU\Software\Classes\AppUserModelId`,
//! and clicks reach it while it runs (in the tray, at least).

use crate::config::APP_ID;
use crate::sound::Sound;
use bb_core::{Names, Notification, Region};
use bb_store::Store;
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::channel::oneshot;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{
    NotificationSetting, ToastActivatedEventArgs, ToastNotification, ToastNotificationManager,
    ToastNotifier,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::UI::Shell::{
    QUNS_ACCEPTS_NOTIFICATIONS, SHQueryUserNotificationState,
    SetCurrentProcessExplicitAppUserModelID,
};
use windows::core::{HSTRING, IInspectable, Interface};

/// The app's identity to Windows: its notifications are filed under it,
/// and the taskbar groups its window by it.
const AUMID: &str = "BlueBubbles.GPUI";
/// The toasts' group, for withdrawing one by its chat.
const GROUP: &str = "chats";
/// Longest side of the sender's photo on a toast.
const PICTURE: u32 = 128;
/// The chat id recorded for the test notification: clicking it just
/// shows the window.
pub const NO_CHAT: i64 = -1;

/// What the user did with a notification.
#[derive(Debug)]
pub enum Response {
    Open {
        chat_id: i64,
        /// Always `None`: Wayland's, kept so the platforms share a shape.
        activation_token: Option<String>,
    },
    MarkRead {
        chat_id: i64,
    },
}

enum Request {
    Show(Notification, Option<PathBuf>, Sound),
    Withdraw(i64),
    Test(oneshot::Sender<Result<String, String>>, Sound),
}

/// Names the app to Windows, before any window or notification: its
/// AppUserModelID, with the name and icon its notifications show.
pub fn set_app_id() {
    let id = HSTRING::from(AUMID);
    if let Err(e) = unsafe { SetCurrentProcessExplicitAppUserModelID(&id) } {
        tracing::warn!("setting the app id: {e}");
    }
    if let Err(e) = register() {
        tracing::warn!("registering for notifications: {e}");
    }
}

fn register() -> windows::core::Result<()> {
    let key = windows_registry::CURRENT_USER
        .create(format!(r"Software\Classes\AppUserModelId\{AUMID}"))?;
    key.set_string("DisplayName", "BlueBubbles")?;
    if let Some(icon) = icon_file() {
        key.set_string("IconUri", icon.to_string_lossy().as_ref())?;
    }
    Ok(())
}

/// The app icon as a file, for the registration to point at.
fn icon_file() -> Option<PathBuf> {
    let path = cache_dir().join("bluebubbles.png");
    let bytes = include_bytes!("../../../assets/icons/bluebubbles-256.png");
    if std::fs::metadata(&path).map(|m| m.len()).ok() != Some(bytes.len() as u64) {
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(&path, bytes).ok()?;
    }
    Some(path)
}

fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_ID)
}

/// Sends notifications; dropping it stops the worker.
pub struct Notifier {
    requests: UnboundedSender<Request>,
    store: Store,
    region: Region,
}

impl Notifier {
    /// Starts the worker on its own thread (toasts need COM); responses
    /// arrive on the returned receiver.
    pub fn start(
        store: Store,
        region: Region,
        _executor: &gpui_kit::BackgroundExecutor,
    ) -> (Self, UnboundedReceiver<Response>) {
        let (requests, rx) = unbounded();
        let (responses, responses_rx) = unbounded();
        let started = std::thread::Builder::new()
            .name("bb-notify".into())
            .spawn(move || {
                if let Err(e) = run(rx, responses) {
                    tracing::warn!("notifications are off: {e}");
                }
            });
        if let Err(e) = started {
            tracing::warn!("notifications are off: {e}");
        }
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
    pub fn show(&self, n: Notification, sound: Sound, executor: &gpui_kit::BackgroundExecutor) {
        let (store, region, requests) = (self.store.clone(), self.region, self.requests.clone());
        executor
            .spawn(async move {
                let picture = n.sender.as_deref().and_then(|s| picture(&store, region, s));
                let _ = requests.unbounded_send(Request::Show(n, picture, sound));
            })
            .detach();
    }

    pub fn withdraw(&self, chat_id: i64) {
        let _ = self.requests.unbounded_send(Request::Withdraw(chat_id));
    }

    /// Sends a sample notification. Resolves to where it went, or the
    /// reason none showed.
    pub async fn test(&self, sound: Sound) -> Result<String, String> {
        let (tx, rx) = oneshot::channel();
        let _ = self.requests.unbounded_send(Request::Test(tx, sound));
        rx.await
            .unwrap_or_else(|_| Err("Windows' notifications can't be reached".into()))
    }
}

/// The photo of `address` as a PNG file a toast can show.
fn picture(store: &Store, region: Region, address: &str) -> Option<PathBuf> {
    let names = Names::load(store, region, [address]).ok()?;
    let bytes = names.photo(address)?.load(store)?;
    let image = image::load_from_memory(&bytes)
        .ok()?
        .thumbnail(PICTURE, PICTURE);
    let dir = cache_dir().join("notification-photos");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{}.png", file_key(address)));
    image.save(&path).ok()?;
    Some(path)
}

/// A file name for `s` (FNV-1a).
fn file_key(s: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn run(
    mut requests: UnboundedReceiver<Request>,
    responses: UnboundedSender<Response>,
) -> windows::core::Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?;
    // Kept so their click handlers stay registered.
    let mut shown: HashMap<i64, ToastNotification> = HashMap::new();
    futures::executor::block_on(async {
        while let Some(request) = requests.next().await {
            match request {
                Request::Show(n, picture, sound) => {
                    let xml = toast_xml(&n, picture.as_deref());
                    match show(&notifier, &xml, Some(n.chat_id), &responses) {
                        Ok(toast) => {
                            tracing::info!("notification for chat {}", n.chat_id);
                            shown.insert(n.chat_id, toast);
                            play(&notifier, sound);
                        }
                        Err(e) => tracing::warn!("notify: {e}"),
                    }
                }
                Request::Test(reply, sound) => {
                    let result = test(&notifier, &responses);
                    if result.is_ok() {
                        play(&notifier, sound);
                    }
                    tracing::info!("test notification: {result:?}");
                    let _ = reply.send(result);
                }
                Request::Withdraw(chat_id) => {
                    if shown.remove(&chat_id).is_some() {
                        let removed = ToastNotificationManager::History().and_then(|h| {
                            h.RemoveGroupedTagWithId(
                                &HSTRING::from(chat_id.to_string()),
                                &HSTRING::from(GROUP),
                                &HSTRING::from(AUMID),
                            )
                        });
                        if let Err(e) = removed {
                            tracing::debug!("withdrawing chat {chat_id}'s notification: {e}");
                        }
                    }
                }
            }
        }
    });
    Ok(())
}

fn test(notifier: &ToastNotifier, responses: &UnboundedSender<Response>) -> Result<String, String> {
    if notifier.Setting().ok() != Some(NotificationSetting::Enabled) {
        return Err(
            "Notifications for BlueBubbles are off in Windows' settings \
                    (System > Notifications)"
                .into(),
        );
    }
    let xml = format!(
        r#"<toast launch="open:{NO_CHAT}"><visual><binding template="ToastGeneric"><text>Notifications are on</text><text>New messages will show like this.</text></binding></visual><audio silent="true"/></toast>"#
    );
    show(notifier, &xml, None, responses)
        .map(|_| "Windows".into())
        .map_err(|e| format!("Windows refused it: {e}"))
}

/// Shows a toast, replacing the chat's last one, with its clicks sent
/// to `responses`.
fn show(
    notifier: &ToastNotifier,
    xml: &str,
    chat_id: Option<i64>,
    responses: &UnboundedSender<Response>,
) -> windows::core::Result<ToastNotification> {
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(xml))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    if let Some(chat_id) = chat_id {
        toast.SetTag(&HSTRING::from(chat_id.to_string()))?;
        toast.SetGroup(&HSTRING::from(GROUP))?;
    }
    let responses = responses.clone();
    toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
        move |_, args| {
            let arguments = args
                .as_ref()
                .and_then(|a| a.cast::<ToastActivatedEventArgs>().ok())
                .and_then(|a| a.Arguments().ok());
            if let Some(response) = arguments.and_then(|a| response(&a.to_string_lossy())) {
                let _ = responses.unbounded_send(response);
            }
            Ok(())
        },
    ))?;
    notifier.Show(&toast)?;
    Ok(toast)
}

/// Plays the sound, as the toast itself is silent, unless Windows
/// wouldn't have shown it (Do Not Disturb, a full-screen app, or the
/// app's notifications switched off).
fn play(notifier: &ToastNotifier, sound: Sound) {
    let enabled = notifier.Setting().ok() == Some(NotificationSetting::Enabled);
    let accepting =
        unsafe { SHQueryUserNotificationState() }.is_ok_and(|s| s == QUNS_ACCEPTS_NOTIFICATIONS);
    if enabled && accepting {
        sound.play();
    }
}

/// What a click on the toast or its button means.
fn response(arguments: &str) -> Option<Response> {
    let (action, chat) = arguments.split_once(':')?;
    let chat_id = chat.parse().ok()?;
    match action {
        "open" => Some(Response::Open {
            chat_id,
            activation_token: None,
        }),
        "read" => Some(Response::MarkRead { chat_id }),
        _ => None,
    }
}

/// The toast: the chat's name over the message (with the sender's name
/// before it in groups), the sender's photo, and a Mark as Read button.
fn toast_xml(n: &Notification, picture: Option<&Path>) -> String {
    let body = match &n.subtitle {
        Some(sender) if n.is_group => format!("{sender}: {}", n.body),
        _ => n.body.clone(),
    };
    let image = picture
        .map(|p| {
            format!(
                r#"<image placement="appLogoOverride" hint-crop="circle" src="{}"/>"#,
                escape(&file_uri(p))
            )
        })
        .unwrap_or_default();
    let id = n.chat_id;
    format!(
        r#"<toast launch="open:{id}"><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text>{image}</binding></visual><actions><action content="Mark as Read" arguments="read:{id}" activationType="foreground"/></actions><audio silent="true"/></toast>"#,
        escape(&n.title),
        escape(&body),
    )
}

fn file_uri(path: &Path) -> String {
    format!("file:///{}", path.to_string_lossy().replace('\\', "/"))
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}
