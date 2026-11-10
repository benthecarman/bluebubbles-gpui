//! Builds Messages' received-message sound into the app when
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
}
