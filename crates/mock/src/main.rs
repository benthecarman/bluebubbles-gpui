//! Runs the mock server with demo conversations, for trying the app
//! without a Mac.
//!
//! Usage: bb-mock-server [--host 127.0.0.1] [--port 1234] [--password demo] [--live]
//!                       [--no-private-api]
//!
//! `--host` binds another address, such as a Tailscale one.
//! `--no-private-api` reports a server without the Private API helper.
//!
//! `--live` makes someone type and send a message every so often.

use bb_mock::{MockServer, seed_demo};
use std::time::Duration;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let mut host: std::net::IpAddr = [127, 0, 0, 1].into();
    let mut port = 1234u16;
    let mut password = "demo".to_owned();
    let mut live = false;
    let mut private_api = true;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => host = args.next().and_then(|h| h.parse().ok()).unwrap_or(host),
            "--port" => port = args.next().and_then(|p| p.parse().ok()).unwrap_or(port),
            "--password" => password = args.next().unwrap_or(password),
            "--live" => live = true,
            "--no-private-api" => private_api = false,
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let server = MockServer::start_on((host, port).into(), &password).await;
    seed_demo(&server);
    if !private_api {
        server.set_info(|i| {
            i.private_api = Some(false);
            i.helper_connected = Some(false);
        });
    }
    println!(
        "mock BlueBubbles server at {} (password {password:?})",
        server.url()
    );
    if live {
        let lines = [
            "On my way!",
            "Did you see this? 😂",
            "Running 5 minutes late",
            "Sounds good",
            "What time works for you?",
        ];
        let mut i = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(20)).await;
            let chat = format!("iMessage;-;{}", bb_mock::demo::ALEX);
            server.typing(&chat, true);
            tokio::time::sleep(Duration::from_secs(4)).await;
            server.typing(&chat, false);
            server.receive(&chat, bb_mock::demo::ALEX, lines[i % lines.len()]);
            i += 1;
        }
    } else {
        std::future::pending::<()>().await;
    }
}
