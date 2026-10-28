# BlueBubbles GPUI

A native Linux client for the
[BlueBubbles Server](https://github.com/BlueBubblesApp/bluebubbles-server),
written in Rust with [GPUI](https://github.com/zed-industries/zed). It
looks and behaves like Apple's Messages and works with or without the
server's Private API.

It does conversations, replies and tapbacks, attachments, and
notifications. It keeps running in the tray, and it checks your spelling as
you type. Everything is cached in SQLite, so it opens instantly and works
offline.

This is an independent project, not affiliated with BlueBubbles.

## Build

With Nix:

```sh
nix develop --command cargo build --release
./target/release/bluebubbles-gpui
```

Sign in with your server's address (for example `mac-mini:1234`, a
Tailscale IP, or an `https://` URL) and its password. The password is
kept in the system keyring.

To build an Ubuntu `.deb` (needs Docker):

```sh
scripts/package-deb.sh   # -> dist/bluebubbles-gpui_<version>-1_amd64.deb
```

## Try it without a Mac

`bb-mock` is a fake server with demo conversations:

```sh
nix develop --command cargo run -p bb-mock --bin bb-mock-server -- --live
BB_SERVER=http://127.0.0.1:1234 BB_PASSWORD=demo ./target/debug/bluebubbles-gpui
```

## Tests

```sh
nix develop --command cargo test --workspace
```

The engine, store, REST and socket clients are tested against the mock
server without a display.

## More

- [ARCHITECTURE.md](ARCHITECTURE.md): how the pieces fit together.
- [docs/BUGS.md](docs/BUGS.md): the Flutter desktop app's known bugs and
  how this design avoids them.
- [NOTICE.md](NOTICE.md): bundled fonts, icons and dictionary.

Licensed under the [Apache License 2.0](LICENSE).
