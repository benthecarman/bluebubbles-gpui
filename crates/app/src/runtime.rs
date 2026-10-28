//! The Tokio runtime the engine and network calls run on. GPUI tasks await
//! its `JoinHandle`s, which work on any executor.

use gpui_kit::{App, Global};

pub struct Runtime(tokio::runtime::Runtime);

impl Global for Runtime {}

impl Runtime {
    pub fn init(cx: &mut App) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("bb-io")
            .enable_all()
            .build()
            .expect("start the Tokio runtime");
        cx.set_global(Self(runtime));
    }

    pub fn handle(cx: &App) -> tokio::runtime::Handle {
        cx.global::<Self>().0.handle().clone()
    }
}
