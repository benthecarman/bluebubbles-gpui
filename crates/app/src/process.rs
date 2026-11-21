//! Running helper programs (ffmpeg, a sound player).

use std::ffi::OsStr;
use std::process::Command;

/// `program`, set up to run unseen: on Windows a console program started
/// from a windowed app otherwise flashes a console window.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}
