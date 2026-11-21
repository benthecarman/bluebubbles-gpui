//! Puts the app icon in the Windows .exe, and builds Messages'
//! received-message sound into the app when
//! `assets/private/received-message.oga` is there. The sound is Apple's,
//! so it is never committed (the folder is ignored by git): builds from
//! a clean checkout have only the app's own sounds. To add it, take
//! `ReceivedMessage.caf` from a Mac's
//! `/System/Library/PrivateFrameworks/ToneLibrary.framework/Resources/AlertTones`
//! and run
//! `ffmpeg -i ReceivedMessage.caf -c:a libvorbis assets/private/received-message.oga`.

use std::path::Path;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(messages_sound)");
    let sound =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/private/received-message.oga");
    // While the file is missing this reruns every build, so it is
    // noticed when it arrives.
    println!("cargo::rerun-if-changed={}", sound.display());
    println!("cargo::rerun-if-changed=build.rs");
    if sound.exists() {
        println!("cargo::rustc-cfg=messages_sound");
    }
    #[cfg(windows)]
    windows_icon();
}

/// The icon Explorer and the taskbar show, as resource 1, which is where
/// GPUI looks for the window's. Built on Windows only: it needs the SDK's
/// resource compiler.
#[cfg(windows)]
fn windows_icon() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/bluebubbles.ico");
    println!("cargo::rerun-if-changed={}", icon.display());
    let mut res = winresource::WindowsResource::new();
    res.set_icon(&icon.to_string_lossy())
        .set("ProductName", "BlueBubbles")
        .set("FileDescription", "BlueBubbles");
    if let Err(e) = res.compile() {
        panic!("embedding the icon: {e}");
    }
}
