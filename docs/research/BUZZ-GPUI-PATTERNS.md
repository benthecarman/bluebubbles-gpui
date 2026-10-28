# How buzz-gpui does it: GPUI patterns for bluebubbles-gpui

Source: `~/projects/buzz-gpui` at `e3be6748` (2026-10-27). All paths below
are relative to that repo unless they start with `~`. This guide covers
GPUI *mechanics* only. The new app copies Apple Messages for its look, so
Buzz's visual design (palette, spacing, component styling) is only
summarised in §3.

---

## 1. Workspace, Cargo and the vendored GPUI patches

### Layout

```
Cargo.toml            # workspace; [patch.crates-io] lives here
crates/app            # bin "buzz-gpui" (default-member); all UI + services
crates/ui             # design system: theme, fonts, icons, components (7k lines)
crates/relay          # network client + SQLite store (no GPUI)
crates/keychain       # OS credential store wrapper
crates/identity, host, host-rpc, buzzd, pair-mobile   # Buzz-specific
third_party/gpui-pre-linux, third_party/gpui-base     # vendored, patched
assets/{fonts,icons,sounds}   packaging/linux   scripts   flake.nix
```

Keep the network and storage crates GPUI-free. Then they test with plain
`#[tokio::test]` and need no display.

### Root `Cargo.toml` (copy this shape)

```toml
[workspace]
members = ["crates/*"]
exclude = ["third_party"]        # vendored crates are patched in, not members
default-members = ["crates/app"]
resolver = "3"

[workspace.package]
version = "0.2.0"
edition = "2024"
license = "Apache-2.0"
publish = false

[workspace.dependencies]
gpui-kit = "=0.7.1"              # exact pin: the patches below are version-exact

[patch.crates-io]
gpui-base = { path = "third_party/gpui-base" }
gpui-pre-linux = { path = "third_party/gpui-pre-linux" }
```

### What gpui-kit 0.7.1 resolves to

`gpui-kit 0.7.1` (Longbridge) is one dependency that re-exports the whole
stack (`pub use ::gpui::*`):

| crate | version | note |
|---|---|---|
| `gpui` → **`gpui-pre 0.3.8`** | `=0.3.8` | a published snapshot of Zed's gpui at `zed@279fe07` (`package = "gpui-pre"`) |
| `gpui-base` | 0.7.1 | text input, Root, selectable text, positioner |
| `gpui-component` | 0.7.1 | shadcn-style components, `window_border` |
| `gpui-kit-assets` | 0.7.1 | default icon bundle |
| `gpui_platform` → `gpui-pre-platform 0.3.8` | | depends on `gpui_linux` = `gpui-pre-linux =0.3.8` |

So you write `use gpui_kit::*;` everywhere and never depend on `gpui`
directly. For a crate that needs a vendored crate's own API, add it with
the exact version (cargo then routes it to the patch):

```toml
# crates/app/Cargo.toml
gpui-base = "=0.7.1"   # for the concealed-ranges API gpui-kit doesn't re-export
[target.'cfg(any(target_os = "linux", target_os = "freebsd"))'.dependencies]
gpui-pre-linux = "=0.3.8"   # lib name is `gpui_linux`: gpui_linux::set_activation_token
zbus = "5"                   # already in the tree through GPUI
arboard = { version = "3", features = ["wayland-data-control"] }
```

### How the vendoring was done (`third_party/README.md`)

1. Copy the crate from `~/.cargo/registry/src/*/gpui-pre-linux-0.3.8`
   unchanged, without its `Cargo.lock`. Keep the normalised `Cargo.toml`
   and `Cargo.toml.orig`. Commit it alone ("Vendor GPUI's Linux platform
   crate", `eee252df`).
2. Make each patch a separate commit, so `git log -p third_party/<crate>`
   is the full patch set.
3. Test it on its own, since it is not a workspace member:
   ```sh
   cd third_party/gpui-pre-linux
   CARGO_TARGET_DIR=../../target/vendored cargo test --features gpui/test-support --lib
   rm Cargo.lock
   ```

### gpui-pre-linux patches (+133/-18 lines in total, all worth copying for a chat app)

| patch | commit | fixes | copy? |
|---|---|---|---|
| `set_activation_token(String)` | `33a48e2a` | GNOME/Wayland refuses to raise an unfocused app's window. GPUI always asked for a fresh token of its own, which the compositor refuses. The patch stores a token the app was **handed** (by a notification click, a tray click, or a second launch) in a thread-local, and `WaylandWindow::activate()` uses it once via `xdg_activation_v1.activate(token, surface)`. On X11 the token is dropped. | **Yes**: clicking a notification must bring up the conversation |
| Name Wayland toplevels at once | `6fe927ff` | `set_title`/`set_app_id` were called after the renderer was created, and renderer setup can flush the connection. GNOME then briefly saw a toplevel with no app id: a second "unknown app" dock icon for ~180 ms on every launch. The calls move into `WaylandSurfaceState::new` right after `get_toplevel`. | **Yes** (5 lines) |
| Paste files (`text/uri-list`) | `33a48e2a` | GPUI read only text and images from the clipboard, so files copied in Nautilus pasted as paths. Now a `file://` uri-list reads as `ClipboardEntry::ExternalPaths` followed by the same paths as a `ClipboardEntry::String`, on Wayland and X11. Other URI lists still read as text. | **Yes**: pasting attachments |

Core of the activation patch (`third_party/gpui-pre-linux/src/linux.rs` and `linux/wayland/window.rs`):

```rust
thread_local! { static ACTIVATION_TOKEN: RefCell<Option<String>> = const { RefCell::new(None) }; }
pub fn set_activation_token(token: String) { ACTIVATION_TOKEN.with(|s| *s.borrow_mut() = Some(token)); }
pub(crate) fn take_activation_token() -> Option<String> { ACTIVATION_TOKEN.with(|s| s.borrow_mut().take()) }

// WaylandWindow::activate
fn activate(&self) {
    let state = self.borrow();
    if let Some(token) = crate::linux::take_activation_token()
        && let Some(activation) = &state.globals.activation
    {
        activation.activate(token, &state.surface);
        return;
    }
    // ...upstream fallback: request own token (usually refused)
}
```

Also re-export it: `pub use linux::{current_platform, linux_platform, set_activation_token};`
in `src/gpui_linux.rs`. X11's `activate()` just calls `take_activation_token()` and discards it.

Clipboard read order in `linux/wayland/clipboard.rs`:
`offer.read_files(..).or_else(|| offer.read_text(..)).or_else(|| offer.read_image(..))`.
On X11, `URI_LIST` is put first in the format atoms. If the list holds no
local files, the code reads it again as text.

### gpui-base patches (+1260/-258 lines)

| patch | commit | fixes | copy? |
|---|---|---|---|
| Decorations in every input mode | `f72acdb0` | Only `EditorState` (code editor) could hold styled ranges. The patch moves the decoration stores into the shared `InputBaseState`, so `create_decorations_collection` / `create_range_decorations_collection` work on `InputState` and `TextareaState`. The mode type parameter defaults to `EditorMode`, so existing code still compiles. | Only if you want live styling in the composer (e.g. highlight a link or mention as it is typed). Probably not for v1. |
| Selectable text in paint order | `da169516` | A `SelectableText` with no explicit `document_order` takes the next one while painting, so a selection dragged across several bubbles copies them in on-screen order. Before, such runs sorted first. | **Probably yes**, if you let users select text across messages |
| Backgrounds beside inline tokens | `49e15e21` | A line with an inline token (chip) dropped its text runs' background colours | Only if you use inline tokens (mention chips) |
| Concealed ranges | `47b39753` (773-line `concealment.rs`) | `set_concealed_ranges` hides byte ranges (or draws a short replacement) while value, history and clipboard keep every byte. Used to hide Markdown markers. | No, iMessage has no Markdown |

**Recommendation:** vendor `gpui-pre-linux` with all three patches. Vendor
`gpui-base` only if you need paint-order selection. If you skip it, drop
its `[patch]` line.

### Cargo.lock implications

- Patched crates show up in `Cargo.lock` **without** `source`/`checksum`
  (`name = "gpui-base" version = "0.7.1" dependencies = [...]`). Commit
  the lock and build with `--locked` (crane and package-deb.sh do).
- A patch applies only while its version satisfies every requirement in
  the graph. `gpui-pre-platform` pins `gpui_linux = "=0.3.8"`, and
  `gpui-component` wants `gpui-base "0.7.1"` (^). If you bump `gpui-kit`,
  cargo prints `warning: Patch ... was not used` and silently uses the
  registry crate. Then re-vendor (copy the new version, replay the
  commits).
- `[patch]` is read only from the workspace root manifest.
- Patched path crates are not in crane's vendored registry. That works
  as long as the flake's `src` filter keeps `third_party/`.

---

## 2. App bootstrap

### `crates/app/src/main.rs` (order matters)

```rust
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info".into()))
        .init();                                          // RUST_LOG=debug,... works
    if let Some(cache) = dirs::cache_dir() { /* media disk cache: ~/.cache/buzz-gpui/media */ }

    // Single instance: claim a Unix socket BEFORE GPUI starts.
    let link = deep_links::from_args(std::env::args().skip(1));  // Exec=buzz-gpui %U
    let socket = single_instance::socket_path(&community::data_dir());
    let handoff = single_instance::Handoff {
        link: link.clone().unwrap_or_default(),
        activation_token: std::env::var("XDG_ACTIVATION_TOKEN").ok(), // from the launcher
    };
    let instance = match single_instance::claim(&socket, &handoff) {
        Ok(Claim::Forwarded) => return,              // the running copy got it
        Ok(Claim::Primary(listener)) => Some(listener),
        Err(e) => { tracing::warn!(%e, "no single-instance"); None }
    };

    let config = config::Config::load();
    let app = gpui_kit::application().with_assets(buzz_ui::Assets);
    app.run(move |cx| {
        gpui_kit::init(cx);              // gpui-component + gpui-base
        buzz_ui::init(cx);               // fonts, grayscale AA, theme, kit theme sync
        runtime::Runtime::init(cx);      // tokio (see below)
        cx.set_http_client(Arc::new(http::Http::new(runtime::Runtime::handle(cx))));
        // ... app globals (AppState, Notifications, appearance, ...)
        let requests = navigation::init(cx);          // channel of Arrival{request, activation_token}
        if let Some(listener) = instance {
            single_instance::serve(listener, move |h| {
                let _ = requests.unbounded_send(navigation::Arrival {
                    request: if h.link.is_empty() { Request::Focus } else { Request::Url(h.link) },
                    activation_token: h.activation_token,
                });
            });
        }
        main_window::init(cx);           // cx.set_quit_mode(QuitMode::Explicit); Ctrl+Q
        main_window::open(cx);
    });
    if instance_was_primary { single_instance::release(&socket); }
}
```

### Fonts and the colour-emoji fix (the "text readability" fix), in full

File: `crates/ui/src/fonts.rs` (63 lines), called from `buzz_ui::init`.
Commits: `f7cdd737` (bundle Inter + JetBrains Mono) and `4ad7ef7d` ("Draw
emoji in colour on Linux").

**Problems on Linux GPUI (cosmic-text + swash):**

1. Variable fonts: GPUI's Linux text stack does not reliably pick weights
   from a variable font. Fix: bundle **static** Inter 4.1 instances
   (Regular, Italic, Medium, MediumItalic, SemiBold, Bold, BoldItalic),
   taken from nixpkgs' `Inter.ttc`. GPUI can't load woff2.
2. Emoji were monochrome. GPUI paints a glyph in colour **only** when its
   font's PostScript name is `NotoColorEmoji`, through swash reading
   **CBDT** bitmaps (swash has no COLRv1). Left to cosmic-text's fallback,
   an emoji falls to the first system font that covers it, usually DejaVu
   Sans, in monochrome.
3. GPUI **drops a family that has no glyph for `m`** when it loads the
   family by name, so an emoji-only font can't be named as a fallback.
4. GPUI keeps a character in the primary font whenever that font covers
   it, so ❤ ⚠ ☀ ♥ came out as Inter's text glyphs, even with VS16
   (`❤️`).

**Fixes:**

- Bundle Noto Color Emoji 2.051 (the CBDT build), edited with fontTools:
  - cmap: map U+006D `m` to the **space** glyph. It is never drawn,
    since ASCII never falls back.
  - name table: family, full name and unique ID become `"Buzz Color
    Emoji"`, so a system Noto install isn't shadowed. The PostScript name
    (nameID 6) **stays `NotoColorEmoji`**, which turns on GPUI's colour path.
- Remove U+2600, U+2665, U+26A0 and U+2764 from the `cmap` of every
  bundled Inter face, so those fall through to colour emoji the way
  browsers draw them. Arrows, shapes and ©®™ keep Inter's glyphs.
- Name the emoji font as the fallback of every text style from the root
  element (`.emoji_fallback()`).

Reproduction script (verified 2026-10-27: run against nixpkgs
`noto-fonts-color-emoji-2.051` and `inter-4.1`, it gives files the same
size as buzz-gpui's, with the same name and cmap tables. Only the table
checksums differ.):

```python
# nix-shell -p 'python3.withPackages (p: [p.fonttools])' --run \
#   "python3 -I edit.py $(nix build --print-out-paths nixpkgs#noto-fonts-color-emoji)/share/fonts/noto/NotoColorEmoji.ttf \
#                       $(nix build --print-out-paths nixpkgs#inter)/share/fonts/truetype/Inter.ttc out/"
import sys
from fontTools.ttLib import TTFont
from fontTools.ttLib.ttCollection import TTCollection
emoji_src, ttc_src, out = sys.argv[1:4]
f = TTFont(emoji_src)
space = f.getBestCmap()[0x20]
for t in f['cmap'].tables:
    if t.isUnicode() and t.format != 14:
        t.cmap[0x6D] = space
for n in f['name'].names:
    if n.nameID in (1, 3, 4, 16):
        n.string = "Buzz Color Emoji"      # pick your own, e.g. "BlueBubbles Color Emoji"
f.save(f"{out}/BuzzColorEmoji.ttf")       # nameID 6 stays "NotoColorEmoji"
for font in TTCollection(ttc_src).fonts:
    for t in font['cmap'].tables:
        for cp in (0x2600, 0x2665, 0x26A0, 0x2764):
            t.cmap.pop(cp, None)
    font.save(f"{out}/{font['name'].getDebugName(6)}.ttf")   # keep only the faces you need
```

Registration code (copy):

```rust
use gpui_kit::{App, FontFallbacks};
use std::borrow::Cow;

pub const EMOJI: &str = "Buzz Color Emoji";
pub const EMOJI_FONT: &[u8] = include_bytes!("../../../assets/fonts/buzz-color-emoji/BuzzColorEmoji.ttf");
const FONTS: &[&[u8]] = &[
    include_bytes!("../../../assets/fonts/inter/Inter-Regular.ttf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Italic.ttf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Medium.ttf"),
    include_bytes!("../../../assets/fonts/inter/Inter-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/inter/Inter-Bold.ttf"),
    EMOJI_FONT,
];

pub fn register(cx: &mut App) {
    let fonts = FONTS.iter().map(|b| Cow::Borrowed(*b)).collect();
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        eprintln!("failed to register bundled fonts: {err:#}");
    }
}
pub fn fallbacks() -> FontFallbacks { FontFallbacks::from_fonts(vec![EMOJI.to_string()]) }

// crates/ui/src/styled.rs: set once at the window root; children inherit
pub trait TypeStyled: Styled + Sized {
    fn emoji_fallback(mut self) -> Self {
        self.text_style().font_fallbacks = Some(crate::fonts::fallbacks());
        self
    }
}
impl<T: Styled> TypeStyled for T {}
// root: div().font_family("Inter").emoji_fallback().text_color(...)
```

Notes for the Messages clone:
- Apple uses SF Pro, which you can't redistribute. Inter is the usual
  stand-in. Whatever font you pick, ship static TTF instances and strip
  the emoji-presentation codepoints from its cmap.
- `EMOJI_FONT` is also exposed as bytes. buzz-gpui uses `cosmic-text
  0.19` to rasterise emoji avatars outside GPUI
  (`crates/app/src/ui/settings/avatar/raster.rs`). Useful for big-emoji
  "jumbomoji" or tapback glyphs drawn as images.
- 10.7 MB font, `include_bytes!`. The binary grows by that much.
- Licences: OFL.txt next to each font, attribution in `NOTICE.md`
  stating the modification.

**Other text-rendering settings** (`crates/ui/src/lib.rs`, `tokens.rs`):

```rust
cx.set_text_rendering_mode(TextRenderingMode::Grayscale);   // = -webkit-font-smoothing: antialiased
```
- Grayscale AA instead of subpixel: it matches macOS-style rendering and
  avoids colour fringes on bubbles with coloured backgrounds. **Copy it**
  (Messages on macOS is grayscale).
- Type sizes are `Rems`, so one global interface scale works:
  `kit.font_size = px(16. * scale)` is the window rem size. Bound to
  Ctrl+= / Ctrl+- / Ctrl+0 (`crates/ui/src/theme.rs`).
- Message prose has its own role, 14px with **1.625 line height**
  (`ef1fbfc8` "Give messages more breathing room"). GPUI has no
  letter-spacing control (`tracking_em` is informational only).
- `bb2cdbd6`: large images drawn small looked pixelated, because the GPU
  samples **without mipmaps**. Decode and downscale off the main thread
  to the device-pixel size you draw (see §4).
- No gamma or contrast env tweaks anywhere. The terminal has a
  WCAG-contrast helper (`crates/app/src/ui/terminal/emulator.rs::ensure_contrast`)
  if you need one.

### Window creation (`crates/app/src/main_window.rs`)

```rust
const MIN_SIZE: Size<Pixels> = size(px(480.), px(400.));
let (handle, root) = gpui_kit::open_window(       // wraps your view in gpui-base's Root
    WindowOptions {
        window_bounds: Some(saved.unwrap_or_else(|| WindowBounds::Windowed(
            Bounds::centered(None, size(px(1200.), px(800.)), cx)))),
        window_min_size: Some(MIN_SIZE),
        app_id: Some("buzz-gpui".into()),          // == .desktop basename == StartupWMClass
        titlebar: Some(TitlebarOptions {
            title: Some("Buzz".into()),            // taskbar/Alt+Tab
            appears_transparent: cfg!(any(target_os = "macos", windows)),
            traffic_light_position: cfg!(target_os = "macos").then_some(point(px(20.), px(17.))),
        }),
        window_decorations: Some(WindowDecorations::Client),  // GNOME/Wayland draws none
        ..Default::default()
    },
    cx,
    |window, cx| {
        window.on_window_should_close(cx, |window, cx| { close(window, cx); false });
        cx.new(|cx| Root::new(workspace, window, cx))
    },
)?;
```

Client-side decorations (`crates/app/src/ui/window_controls.rs`, 163 lines, copy it):

```rust
// whole window: resize edges come from gpui-component's window_border
window_border().shadow_size(px(0.)).child(content)   // shadow needs a translucent surface; not all GPUs

// your header:
pub fn title_bar(el: Stateful<Div>) -> Stateful<Div> {
    el.when(cfg!(windows), |t| t.window_control_area(WindowControlArea::Drag))
      .on_mouse_down(MouseButton::Left, |e, window, _| {
          if e.click_count == 2 { window.titlebar_double_click() } else { window.start_window_move() }
      })
      .on_mouse_down(MouseButton::Right, |e, window, _| {
          if window.window_controls().window_menu { window.show_window_menu(e.position) }
      })
}
// buttons inside it: .on_mouse_down(Left, |_, _, cx| cx.stop_propagation())
// minimize/maximize only if window.window_controls().minimize/.maximize;
// Hyprland: no minimize (XDG_CURRENT_DESKTOP check).
```

Window geometry is saved as JSON in SQLite `device_prefs`, debounced 500
ms after `observe_window_bounds`. Bounds under `MIN_SIZE` are never saved,
because compositors report 1x1 while mapping or hiding.

`cx.set_quit_mode(QuitMode::Explicit)`: closing the window doesn't quit,
the app decides. There is close-to-tray (only when a tray icon is
actually showing) and Ctrl+Q via `KeyBinding::new("secondary-q", Quit, None)`.

### Single instance (`crates/app/src/single_instance.rs`, 448 lines, copy the unix part)

- Socket: `$XDG_RUNTIME_DIR/buzz-gpui-<fnv1a(data_dir)>.sock`. Keying by
  data dir keeps separate profiles and dev runs (different
  `XDG_DATA_HOME`) apart.
- `flock` on `<sock>.lock` around bind/connect, so two launches never
  both find the socket stale. `UnixListener::bind`; on `AddrInUse`,
  `connect` and write `"<link>\n<activation-token>\n"`. If connect
  fails, the socket is stale: remove it and retry.
- `serve()` runs a plain `std::thread` that accepts and pushes into an
  unbounded futures channel. A GPUI task drains it:

```rust
// crates/app/src/ui/navigation.rs
cx.spawn(async move |cx| {
    while let Some(arrival) = arrivals.next().await {
        cx.update(|cx| {
            crate::main_window::open(cx);                 // reopen if hidden to tray
            with_workspace(cx, move |ws, window, cx| {
                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                if let Some(token) = arrival.activation_token {
                    gpui_linux::set_activation_token(token);   // patched crate
                }
                #[cfg(target_os = "macos")] cx.activate(true);
                window.activate_window();
                ws.handle(arrival.request, window, cx);
            })
        });
    }
}).detach();
```

The notification click, tray click and second launch all feed this one
`Arrival` channel.

### tokio ↔ GPUI (`crates/app/src/runtime.rs`, 24 lines, copy verbatim)

```rust
pub struct Runtime(tokio::runtime::Runtime);
impl Global for Runtime {}
impl Runtime {
    pub fn init(cx: &mut App) {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2).thread_name("buzz-io").enable_all().build()
            .expect("start the tokio runtime");
        cx.set_global(Self(rt));
    }
    pub fn handle(cx: &App) -> tokio::runtime::Handle { cx.global::<Self>().0.handle().clone() }
}
```

Usage pattern: **spawn on tokio, await the JoinHandle from a GPUI task, `cx.update` the result back**:

```rust
let found = Runtime::handle(cx).spawn(banners::available());   // tokio future
cx.spawn(async move |cx| {
    let ok = matches!(found.await, Ok(true));                 // JoinHandle is executor-agnostic
    cx.update(|cx| Notifications::service(ok, cx));
}).detach();
```

Long-lived tokio actors (socket, notification bus) talk to the UI over
`tokio::sync::watch`/`mpsc` or `futures::channel::mpsc`. The UI side
loops on `.next().await` inside `cx.spawn`. CPU work that isn't async
goes to `cx.background_spawn(...)`. Timers on the UI side use
`cx.background_executor().timer(d).await`.

### GPUI's HTTP client (`crates/app/src/http.rs`, 75 lines, copy it)

The platform default on Linux **cannot fetch at all**, so `img("https://...")`
never loads. buzz-gpui implements `gpui_kit::http_client::HttpClient`
over reqwest on the tokio runtime (GET + https only, 25 MB cap,
redirects limited to 5) and installs it with `cx.set_http_client(...)`.
BlueBubbles attachments need the server password as a query parameter or
header, so do what buzz does for relay media and use a custom `Asset`
(§4) instead of the global client.

---

## 3. Theming (summary only; the look will be Apple Messages)

Reusable **mechanics**, not the Buzz look:

- `crates/ui/src/theme.rs` (307 lines): a `Theme` global with
  `preference: System|Light|Dark`, a resolved `appearance`, `interface_scale`
  and `colors: Colors`. `Theme::edit` recomputes, re-syncs the kit theme,
  then runs `cx.set_global(ThemeEdited)` (observers) and `cx.refresh_windows()`.
- Following the system appearance for every window, gpui-kit's `Root`
  included:
  ```rust
  cx.observe_new(|_: &mut gpui_kit::base::Root, window, cx| {
      if let Some(window) = window {
          Theme::sync_system_appearance(window, cx);          // Appearance::from(window.appearance())
          window.observe_window_appearance(|w, cx| Theme::sync_system_appearance(w, cx)).detach();
      }
  }).detach();
  // initial: cx.window_appearance()
  ```
- `crates/ui/src/colors.rs` (315 lines): a `colors!` macro generating
  `Colors::light()`/`dark()` from `field = "--css-var": "#light", "#dark";`
  rows, plus a test that checks them against a JSON file. **This is a good
  shape for Messages' palette** (blue/green bubbles, grey received bubble,
  system backgrounds), with your own values.
- `crates/ui/src/kit.rs` (92 lines): **you need an equivalent** whatever
  the look. It projects your palette onto gpui-kit's
  `component::Theme` (`KitTheme::change(mode)`, then
  `KitTheme::update(cx, |kit| { kit.font_family, kit.font_size (rem),
  kit.background, foreground, border, ring, caret, selection, popover,
  primary*, scrollbar_thumb, ... })`). Without it, kit inputs, scrollbars
  and menus keep the shadcn defaults.
- `crates/ui/src/scope.rs` (103 lines): `ThemeScope` forces an appearance
  for a subtree by pushing onto a theme stack during layout and paint.
  Skip it.
- Components consume tokens through `cx.theme().colors` (an `ActiveTheme`
  trait on `App`). Don't import gpui-kit's own `ActiveTheme` in the same
  file, because the names clash.
- Buzz-specific, don't copy: `background.rs` (shell gradient), `icon.rs`
  (Tabler icons and a `?stroke=` SVG rewrite in `assets.rs`),
  `components/*` (4.7k lines of Buzz-styled buttons, menus and dialogs).
  `assets.rs` is worth copying as a pattern: `rust-embed` with the
  `debug-embed` feature, falling back to `gpui_kit::assets::Assets`.
  Commit `591d866c` explains `debug-embed`: without it, debug builds
  read from the source dir, which breaks when a build cache serves
  another checkout's artifact.

---

## 4. UI mechanics

### Virtualised message list with scroll anchoring (`crates/app/src/ui/channel.rs`, `scroll.rs`)

```rust
const LOAD_OLDER_WITHIN: usize = 5;

// Row 0 is a synthetic "edge" row (loading spinner / start of conversation); items are 1..
let list = ListState::new(1, ListAlignment::Bottom, px(1000.));   // overdraw px
list.set_follow_mode(FollowMode::Tail);    // stick to bottom while at bottom (gpui-pre 0.3.8 API)

let weak = cx.entity().downgrade();
list.set_scroll_handler(move |event, _, cx| {
    if event.visible_range.start < LOAD_OLDER_WITHIN + 1 {
        let _ = weak.update(cx, |this, cx| this.load_near_top(cx));
    }
});

// render
list(self.list.clone(), move |index, window, cx| {
    if index == 0 { return edge_row(..).into_any_element(); }
    match items.get(index - 1) { Some(item) => bubble(item, cx), None => div().into_any_element() }
}).size_full()
```

**Keeping the reading position while history loads above:** never call
`reset` for prepends. Diff the old and new id lists and `splice` exact
ranges:

```rust
fn sync_items(&mut self, cx: &mut Context<Self>) {
    let new: Vec<Id> = ...; let old: Vec<Id> = self.items.iter().map(|i| i.id).collect();
    match old.first().and_then(|f| new.iter().position(|id| id == f)) {
        Some(prepended) if new[prepended..].starts_with(&old) => {
            self.list.splice(1..1, prepended);                         // older rows above
            let end = prepended + old.len();
            self.list.splice(end + 1..end + 1, new.len() - end);       // newer rows below
            if !old.is_empty() {                                       // re-measure neighbours whose
                self.list.splice(prepended + 1..prepended + 2, 1);     // grouping/tail flags changed
                self.list.splice(end..end + 1, 1);
            }
        }
        _ => self.list.reset(new.len() + 1),
    }
    self.list.splice(0..1, 1);     // edge row state changed
}
```

Other ListState calls in use: `logical_scroll_top()` (→ `ListOffset {
item_ix, offset_in_item }`), `is_following_tail()`, `scroll_to(ListOffset{..})`,
`scroll_to_reveal_item(ix)`, `scroll_by(px)`, and `set_follow_mode(Tail)`
for "End" / "jump to latest". Keyboard: PageUp/PageDown/Home/End on a
focusable wrapper (`channel.rs:680`).

Persisted position (`scroll.rs`): save `{bottom: is_following_tail(),
anchor: {id of top row, y: -offset_in_item}}` per chat; restore once the
first page is loaded. Keep the `ListState` and its measured rows for the
last 3 chats in memory, so reopening paints instantly without
re-measuring jumps.

**Gotcha `2dd49c09`:** don't start the load *synchronously* inside the
scroll handler if the load touches the list (e.g. pausing follow-tail).
The list is still borrowed: the double borrow panics, and on macOS it
aborts inside AppKit. Defer it (`cx.spawn`/`cx.defer`, or notify and
check on the next render).

`uniform_list` isn't used. The emoji picker uses `list` + `ListAlignment::Top`
too, with sticky category headers driven from `set_scroll_handler`
(`emoji_picker.rs:684`). For a long sidebar (the chat list) buzz uses
plain divs with **windowing by hand** (`1bed23a5`: off-screen rows become
fixed-height placeholders that keep focus handles) and
`AnyView::cached(..)` (`f239fa9b`). With fixed-height chat rows,
`uniform_list` is simpler.

### Multiline composer (`crates/app/src/ui/composer.rs`)

```rust
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState, Escape, MoveUp, Backspace};

let input = cx.new(|cx| TextareaState::new(window, cx)
    .placeholder("iMessage")
    .submit_on_enter(true)       // Enter submits, Shift+Enter inserts a newline
    .auto_grow(1, 9));           // min/max rows
cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
    InputEvent::PressEnter { shift: false, .. } => this.submit(window, cx),
    InputEvent::Change => this.changed(window, cx),        // drafts (400 ms debounce), typing indicator
    _ => {}
});

// render
Textarea::new(&self.input)
    .on_paste(move |item: &ClipboardItem, window, cx| {   // return true = consumed
        weak.update(cx, |this, cx| this.paste(item, window, cx)).unwrap_or(false)
    })

// shell around it
div().key_context("Composer")
    .capture_key_down(cx.listener(|_, e: &KeyDownEvent, _, cx| {
        // a held Enter repeats; never send on a repeat
        if e.is_held && e.keystroke.key == "enter" && !e.keystroke.modifiers.shift { cx.stop_propagation() }
    }))
    .capture_action(cx.listener(|this, _: &MoveUp, window, cx| { /* Up in empty composer → edit last */ }))
    .capture_action(cx.listener(|this, _: &Escape, window, cx| { /* close picker / cancel reply */ }))
    .on_drag_move(cx.listener(|this, e: &DragMoveEvent<ExternalPaths>, _, cx| {
        if e.bounds.contains(&e.event.position) { this.set_drag_over(true, cx) }
    }))
    .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
        this.add_paths(paths.paths().to_vec(), cx); cx.stop_propagation();
    }))
```

Paste handling (`composer.rs:3197`): loop over `item.entries()`.
`ClipboardEntry::Image(img)` → bytes + `img.format` (Png/Jpeg/Gif/Webp)
→ attachment. `ClipboardEntry::ExternalPaths(p)` → file attachments
(needs the gpui-pre-linux patch). Otherwise fall through to text.
Read and modify the value with `state.value()`, `state.selected_range()`,
`state.set_value(..)`, `state.focus(window, cx)`.

Copying images *out* (`crates/app/src/image_clipboard.rs`): GPUI's
clipboard writes only text on Wayland, so use `arboard` with
`wayland-data-control`, in **one `Clipboard` kept alive for the app's
lifetime** (on Linux a copied image exists only while its owner runs).
It blocks, so run it off the UI thread.

Shortcut precedence gotcha (`composer.rs:70`): GPUI ranks a binding with
no context above any with one. To override an app-wide shortcut inside
the composer, bind it with `None` context *after* the app's binding.

### Async images, caching, blurhash (`crates/app/src/ui/media.rs`)

A custom `Asset` gives one load per key, shared by every view, and GPUI
redraws the asking view when the load lands:

```rust
#[derive(Clone)] struct Remote { url: SharedString, client: MediaClient }
impl Hash for Remote { fn hash<H: Hasher>(&self, s: &mut H) { self.url.hash(s) } }   // key = URL only

enum RemoteImage {}
impl Asset for RemoteImage {
    type Source = Remote;
    type Output = Result<Arc<Image>, SharedString>;
    #[allow(clippy::manual_async_fn)]       // async fn would borrow &mut App past the call
    fn load(src: Remote, _: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        async move {
            let bytes = src.client.fetch(&src.url).await.map_err(|e| e.to_string())?; // runs on tokio inside
            let format = image_format(&bytes).ok_or("Not a supported image")?;
            Ok(Arc::new(Image::from_bytes(format, bytes.to_vec())))
        }
    }
}

fn decoded(r: &Remote, window: &mut Window, cx: &mut App) -> Option<Result<Arc<RenderImage>, SharedString>> {
    match window.use_asset::<RemoteImage>(r, cx)? {          // None while loading
        Ok(image) => image.use_render_image(window, cx).map(Ok),
        Err(e) => Some(Err(e)),
    }
}
pub fn image(url: &str, client: &MediaClient) -> ImageSource {
    let r = Remote { url: url.to_string().into(), client: client.clone() };
    ImageSource::Custom(Arc::new(move |window, cx| decoded(&r, window, cx).map(|x| x.map_err(ImageCacheError::Asset))))
}
// img(image(url, &client)).size_full().object_fit(ObjectFit::Cover)
```

Blurhash placeholder: another `Asset` with `Source = SharedString` (the
hash), decoded at 32×32. **Swap R and B**, because `RenderImage` frames
are BGRA:

```rust
let mut px = blurhash::decode(hash, 32, 32, 1.).ok()?;              // blurhash = "0.2"
for p in px.as_chunks_mut::<4>().0 { p.swap(0, 2); }
Arc::new(RenderImage::new(vec![image::Frame::new(image::RgbaImage::from_raw(32, 32, px)?)]))
```

`thumbnail()` puts the blurhash `img` and the real `img` absolutely in
the same box. Once the real image decodes it replaces the placeholder.
It is built inside a `Lazy` element (`media.rs:275`) because row
renderers get only `&App`, while `use_asset` needs `&mut Window`.
BlueBubbles sends no blurhash, so use a fixed-size grey box sized from
the attachment's width and height.

Disk cache: `buzz_relay_client::set_disk_cache(~/.cache/buzz-gpui/media)`
(`crates/relay/src/media_disk.rs`). Downscale large images to their drawn
device-pixel size (`bb2cdbd6`; there are no mipmaps). Use `image = "0.25"`
with `default-features = false, features = ["png","jpeg","gif","webp"]`,
matching GPUI's version. HEIC (iMessage photos!) is not decodable by
`image`. buzz shells out to `ffmpeg` (a `.deb` depends on it), or ask the
BlueBubbles server for a converted JPEG.

### Context menus, popovers, modals (`crates/app/src/ui/overlay.rs`, 205 lines, copy it)

The primitives are `deferred(anchored()...)` with priorities. Dialogs sit
at layer 1, menus at 3. A full-window occluding backdrop catches outside
clicks:

```rust
fn layer(id: &'static str, window: &Window, on_dismiss: Option<Dismiss>) -> Stateful<Div> {
    let size = window.viewport_size();
    div().id(id).occlude().w(size.width).h(size.height)
        .when_some(on_dismiss, |t, d| t.on_mouse_down(MouseButton::Left, move |_, w, cx| d(w, cx)))
}

pub fn context_menu(window: &Window, at: Point<Pixels>, content: impl IntoElement, on_dismiss: Dismiss) -> impl IntoElement {
    let right = on_dismiss.clone();
    let backdrop = layer("context-menu-backdrop", window, Some(on_dismiss))
        .on_mouse_down(MouseButton::Right, move |_, w, cx| right(w, cx));   // right-click elsewhere closes too
    div()
        .child(deferred(anchored().position(point(px(0.), px(0.))).child(backdrop)).with_priority(3))
        .child(deferred(anchored().position(at).snap_to_window_with_margin(px(8.))
            .child(div().id("context-menu").occlude().child(content))).with_priority(4))
}

pub fn modal(window: &Window, content: impl IntoElement, on_dismiss: Option<Dismiss>) -> impl IntoElement {
    deferred(anchored().position(point(px(0.), px(0.))).child(
        layer("modal", window, on_dismiss).child(Scrim::new().child(
            div().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(content)))))
    .with_priority(1)
}
// dropdown(): same, anchored under a `relative` trigger, 4px gap
```

Open a menu by storing `Option<Point<Pixels>>` from `on_mouse_down(MouseButton::Right, |e, ..| e.position)`,
then render `context_menu(..)` while it is `Some`. A tapback picker
(reactions bar) is the same pattern positioned at the bubble's bounds.
The modal's Tab trap: `overlay::modal_shown()` plus a frame flag
(`main_window.rs::tab_through`). Escape closes it via the `"BuzzDialog"`
key context (`KeyBinding::new("escape", Cancel, Some(DIALOG_CONTEXT))`).

Tooltips: `div().id("x").tooltip(Tooltip::text("..."))`, where
`Tooltip::text` returns `impl Fn(&mut Window,&mut App)->AnyView`
(`crates/ui/src/components/tooltip.rs`, 59 lines).

### Emoji picker, reactions, avatars, badges, settings

- Emoji picker: `crates/app/src/ui/emoji_picker.rs` (1755 lines, using
  the `emojis = "0.9"` crate, `ListState` sections, and search via
  `InputState`). Buzz-specific in behaviour (emoji-mart parity) but a
  working reference. Not needed for iMessage tapbacks (6 fixed ones +
  emoji tapback on iOS 18).
- Reactions row: `crates/app/src/ui/message.rs:2039 fn reactions`.
- Avatar: `crates/ui/src/components/avatar.rs` (`Avatar::new(name).image(src).size(..)`,
  initials fallback). GPUI **can't mask images to arbitrary shapes**,
  only rounded rects/circles via `.rounded()`.
- Unread: `crates/app/src/ui/unread_badge.rs` (dot or bold label, no
  numerals).
- Toasts: `crates/app/src/ui/toasts.rs`.
- Settings pages: `crates/app/src/ui/settings/{appearance,notifications,about,...}.rs`.
  Each is a plain view with sections. Preferences persist as JSON in
  SQLite (`device_pref`/`set_device_pref`) and errors surface as toasts
  (`crates/app/src/appearance.rs`).

### GPUI re-entrancy panics to avoid

- `77f80317`: focusing a child from inside your own render read an
  entity being updated, which panics. Call `state.focus(window, cx)` on
  the input entity directly, or defer.
- `3e386d74`: `window.request_animation_frame()` from a click handler
  panics outside a draw. Use `cx.notify()` instead.
- `2dd49c09`: ListState double borrow from a scroll handler (above).

---

## 5. Desktop notifications on Linux (`crates/app/src/notifications{.rs,/banners.rs}`)

Raw zbus (`zbus = "5"`, already in the tree via GPUI), no
`notify-rust`. One tokio task owns the session-bus connection. The UI
sends `Command::Show/Close` over an `mpsc` and never awaits the bus.

```rust
const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

async fn connect() -> zbus::Result<zbus::Proxy<'static>> {
    let c = zbus::Connection::session().await?;
    zbus::Proxy::new_owned(c, SERVICE, PATH, SERVICE).await
}

async fn notify(proxy: &zbus::Proxy<'_>, title: &str, body: &str, markup: bool) -> zbus::Result<u32> {
    let actions = ["default", "Open"];                 // "default" = click on the banner body
    let hints: HashMap<&str, Value<'_>> = HashMap::from([
        ("desktop-entry", Value::from("buzz-gpui")),   // app name/icon/settings via the .desktop file
        ("category", Value::from("im.received")),
        ("suppress-sound", Value::from(true)),         // app plays its own sound
    ]);
    proxy.call("Notify", &("Buzz", 0u32, "buzz-gpui", title, escape_if(markup, body).as_ref(),
                           &actions[..], hints, -1i32)).await
}
```

Run loop (`banners.rs::run`):
1. `GetCapabilities`. Require `"actions"`, note `"body-markup"`. If the
   server parses markup, escape `& < >` in the body.
2. Call `proxy.receive_all_signals().await` **before the first Notify**,
   so no click is missed.
3. `tokio::select!` over commands and signals:
   - `ActivationToken (u32 id, String token)`: store the token per id
     (it arrives *before* `ActionInvoked`).
   - `ActionInvoked (u32 id, String action)`: if `action == "default"`,
     send `Arrival { request: Open(target), activation_token }` to
     navigation, which calls `gpui_linux::set_activation_token` and then
     `window.activate_window()` (§2).
   - `NotificationClosed (u32 id, u32 reason)`: forget the id.
4. Withdraw a banner when its message is read:
   `proxy.call("CloseNotification", &(id,))`. Keep id → chat/message maps
   bounded (`notifications/shown.rs`, 68 lines).

Suppressing when focused (`notifications.rs:345`):

```rust
fn in_front(key: &Key, viewing: bool, open: Option<&Conversation>, focused: bool) -> bool {
    focused && (viewing || open.is_some_and(|o| o.shows(key)))
}
// focused = cx.active_window().is_some(); OnScreen global set by the conversation view.
// An unfocused window still notifies. Plus a 2-minute dedupe (HashMap with cap 2048).
```

Body text is flattened to plain text and capped at 200 characters; the
sender name at 64.

- **Inline reply:** not implemented. The spec route would be the
  `inline-reply` capability plus the `NotificationReplied` signal (GNOME
  doesn't support it, KDE does). Wire it as another `select!` arm if you
  want it.
- **Sounds** (`crates/app/src/sounds.rs`, 312 lines): bundled audio
  played through GStreamer `playbin` + `appsrc` from `&'static [u8]`, one
  pipeline per slot, played only after the server accepted the banner.
  For a chat app without video, `rodio` would be lighter, or pass a
  `sound-name` hint (`message-new-instant`) and drop `suppress-sound`.
- **Tray** (`crates/app/src/tray/linux.rs`, 472 lines, copy it): a
  hand-written StatusNotifierItem plus `com.canonical.dbusmenu` via
  `#[zbus::interface]`. It registers with `org.kde.StatusNotifierWatcher`,
  uses ARGB `IconPixmap`, `Status = NeedsAttention` while unread
  (draws a dot), and a menu with Open/Quit. It implements
  `ProvideXdgActivationToken(token)`, stores it, and hands it to the
  Open arrival. GNOME needs the AppIndicator extension; Ubuntu enables it.
- macOS/Windows: `notifications/system.rs` goes through GPUI's own
  notification API (`cx.set_app_identity(..)` on Windows).
- Add `X-GNOME-UsesNotifications=true` to the `.desktop` file so the app
  appears in GNOME Settings → Notifications before its first banner.

E2E check: `scripts/test-gnome-activation.sh` runs a headless GNOME
Shell 50 on Wayland under `dbus-run-session`, with an unsafe-mode
extension to drive `org.gnome.Shell.Eval`. It checks that a notification
click and a second launch with a token raise the window, and that a
launch without a token does *not*. Worth copying once notifications work.

---

## 6. Storage

### SQLite: `rusqlite = { version = "0.38", features = ["bundled"] }` (`crates/relay/src/store.rs`)

```rust
#[derive(Clone)] pub struct Store { connection: Arc<Mutex<Connection>> }

fn setup(c: Connection) -> Result<Store, StoreError> {
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.pragma_update(None, "synchronous", "FULL")?;     // outbox durability; prefs use a relaxed path (NORMAL)
    c.pragma_update(None, "secure_delete", "ON")?;
    c.pragma_update(None, "foreign_keys", "ON")?;
    let v: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
    match v {
        0 => c.execute_batch(&format!("BEGIN; {SCHEMA} PRAGMA user_version = {SCHEMA_VERSION}; COMMIT;"))?,
        SCHEMA_VERSION => {}
        newer => return Err(StoreError::Corrupt(format!("schema {newer} is newer than this app"))),
    }
    Ok(Store { connection: Arc::new(Mutex::new(c)) })
}
pub fn with<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T, StoreError>) -> Result<T, StoreError>
pub fn in_memory() -> Result<Self, StoreError>   // for tests
```

Migrations: `user_version` only, with no migration framework (schema v1).
`crates/relay/src/archive.rs:150` shows a v1 → v2 upgrade inside
`BEGIN IMMEDIATE` with `busy_timeout(2s)`, `auto_vacuum=INCREMENTAL` set
before the first table, and refusal of newer versions. For BlueBubbles,
use a `match version { 0 => .., 1 => .., }` ladder in one transaction.
Generic key/value tables (`device_prefs(name, value_json)`,
`view_state(...)`) hold all UI state as JSON, so few migrations are needed.

### Directories (`dirs = "6"`)

| what | path |
|---|---|
| data (SQLite, socket fallback) | `dirs::data_dir()/buzz-gpui` → `~/.local/share/buzz-gpui` |
| config | `dirs::config_dir()/buzz-gpui/config.toml` |
| media cache | `dirs::cache_dir()/buzz-gpui/media` |
| instance socket | `dirs::runtime_dir()` (`$XDG_RUNTIME_DIR`) |

### Secrets: `keyring = "=3.6.3"` via `buzz-credential-store` (a git dep from buzz-app)

```toml
[target.'cfg(target_os = "linux")'.dependencies]
keyring = { version = "=3.6.3", default-features = false,
            features = ["sync-secret-service", "crypto-rust", "vendored"] }  # Secret Service over libdbus, vendored
[target.'cfg(target_os = "windows")'.dependencies]
keyring = { version = "=3.6.3", default-features = false, features = ["windows-native"] }
[target.'cfg(target_os = "macos")'.dependencies]
security-framework = "3.7"     # buzz-keychain implements macOS itself via passwords::*_generic_password
```

API: `read(service, account) -> Zeroizing<Vec<u8>>`, `add` (create-only,
returns `Occupied`), `delete`. Errors map to
`Absent/Denied/Corrupt/Unavailable/Occupied/Busy`. Store the BlueBubbles
server password under, e.g., `service = "app.bluebubbles.gpui"`,
`account = <server url>`.

Fallbacks for machines without a Secret Service (headless test desktops):
- `config.toml` → `[embedded] credentials = "files"`.
- A `BUZZ_DEV_NSEC_FILE` env var stands in for the keyring entry.
- `crates/keychain/src/files.rs`: `write_private` (mode 0600, `sync_all`)
  and `private_dir` (0700).

The `.deb` depends on `gnome-keyring | kwalletmanager | keepassxc`.

### Config (`crates/app/src/config.rs`)

`toml = "0.9"` + serde (`#[serde(deny_unknown_fields, default)]`). On a
parse error it logs and uses defaults. To rewrite only part of the file
while keeping the user's comments, it uses `toml_edit = "0.25"`
(`save_hosts_to`).

### Atomic writes (`crates/app/src/atomic_file.rs`, 48 lines, copy it)

Write `.<name>.<pid>.<counter>.tmp` beside the target, then `rename` over
it. Remove the temp file on error. Unique names mean two writers never
share a temp file.

---

## 7. HTTP / WebSocket

```toml
reqwest = { version = "0.13", default-features = false, features = ["rustls", "json"] }
rustls = "0.23"
rustls-platform-verifier = "0.7"
tokio-tungstenite = { version = "0.29", features = ["rustls-tls-native-roots", "rustls-tls-webpki-roots"] }
tokio = { version = "1", features = ["macros", "net", "rt", "sync", "time"] }
```

- TLS for `wss://` uses the **platform verifier**, the same as reqwest
  (system roots + `SSL_CERT_FILE`), so private-CA servers work. This
  matters for self-hosted BlueBubbles behind Tailscale or Caddy. See
  `crates/relay/src/socket.rs:44 fn tls()`: an `OnceLock<Option<Arc<rustls::ClientConfig>>>`
  built with `builder_with_provider(default or aws_lc_rs).with_safe_default_protocol_versions()
  .with_platform_verifier().with_no_client_auth()`, passed as
  `tokio_tungstenite::Connector::Rustls`.
- Reconnect actor (`socket.rs:495`), adaptable as is:

```rust
pub struct SocketOptions {
    ping_interval: Duration::from_secs(25),   // heartbeat; native sockets aren't torn down on network change
    dead_after:    Duration::from_secs(70),   // no frames for this long = lost
    min_backoff:   Duration::from_millis(500),
    max_backoff:   Duration::from_secs(30),
    healthy_after: Duration::from_secs(60),   // only a long-enough healthy link resets the backoff
    jitter: random_unit, max_reconnects: None,
}
// on End::Lost:
let ceiling = opts.min_backoff.saturating_mul(1 << attempts.min(16)).min(opts.max_backoff);
let retry_in = ceiling.mul_f64(0.5 + jitter().clamp(0.0, 1.0) / 2.0);   // equal jitter
state.send_replace(SocketState::Waiting { error, retry_in });            // tokio::sync::watch for the UI
// wait() select!s the sleep against commands so Subscribe/Publish queue while offline,
// and Command::Reconnect (the user's "Retry") cuts the wait short and resets attempts.
fn random_unit() -> f64 {   // no rand dependency
    use std::hash::{BuildHasher, Hasher};
    let bits = std::collections::hash_map::RandomState::new().build_hasher().finish();
    (bits >> 11) as f64 / (1u64 << 53) as f64
}
```

Also: `MAX_FRAME = 1 MiB` (larger inbound frames are dropped unread), and
`WebSocketConfig` limits. When the window becomes active again, sessions
recheck (`main_window.rs`: `observe_window_activation` →
`session.visible()`), like a browser's `visibilitychange`. Note that
BlueBubbles' server uses **Socket.IO**, not raw WebSocket. Either use
`rust_socketio` (async, tokio) or speak Engine.IO v4 over
tokio-tungstenite and reuse this actor.

---

## 8. Testing

- **No GPUI `TestAppContext` tests anywhere** (0 uses in 1866 tests).
  The strategy is to keep logic out of views and unit-test plain
  functions and structs: `in_front()`, the `Shown` maps,
  `notification_body` escaping, geometry `usable()`, window-control
  `Policy::for_session`, the `Colors` table against JSON, and "every icon
  is embedded" (`crates/ui/src/assets.rs`).
- gpui-kit 0.7.1 does ship a UI test harness behind the `test-support`
  feature (`gpui_kit::test`: `#[gpui_kit::test]`,
  `TestWindowExt::{find, click, right_click, scroll, press("enter"),
  input("text"), drag_to}`, `ElementSnapshot`). It is unused by buzz but
  available: add `gpui-kit = { workspace = true, features = ["test-support"] }`
  under `[dev-dependencies]`. Import kit items explicitly in test modules,
  because the glob import brings in GPUI's `test` macro and shadows
  `#[test]`.
- Mock servers: `crates/relay/tests/socket.rs` and `live.rs` contain an
  **in-process fake WebSocket server**:

```rust
struct FakeRelay { address: String, seen: mpsc::UnboundedReceiver<Seen>,
                   push: mpsc::UnboundedSender<Value>, kick: mpsc::UnboundedSender<()> }
impl FakeRelay {
    async fn start(behaviour: Behaviour) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("ws://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { loop {
            let (stream, _) = listener.accept().await.unwrap();
            let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else { continue };
            // select! { client frames → seen_tx; push_rx → ws.send; kick_rx → drop connection }
        }});
        ...
    }
}
// Behaviour { auth: Open|Accept|Refuse|Silent, ack: bool, drop_all: bool } toggles failure modes.
```

- Timing tests: `#[tokio::test(start_paused = true)]` with
  `tokio = { features = ["test-util"] }` (`crates/app/src/agents2/typing/tests.rs`),
  and `SocketOptions { jitter: || 0.5, min_backoff: ms(10), .. }` for
  deterministic backoff.
- Integration against a real server: `crates/relay/tests/test_relay.rs`
  is skipped unless `BUZZ_TEST_RELAY` points at fixtures
  (`scripts/test-relay.sh up && seed`, docker compose). Same idea:
  `BLUEBUBBLES_TEST_SERVER=url,password`.
- Store tests use `Store::in_memory()` and `tempfile::tempdir()`.
- Visual checks on a headless GNOME: `scripts/desktop.sh run|shot|eval`
  (systemd user unit `gui-testing`, private bus, `VK_LOADER_DRIVERS_DISABLE=*nvidia*`
  if the NVIDIA GPU stalls Vulkan's probe, see `7fc5210a`).

---

## 9. Packaging

### `flake.nix` (crane), as it is today

- Inputs: `nixpkgs/nixos-unstable`, `flake-utils`, `crane`.
- `packages.buzzd` only (the headless daemon). The **GUI is not
  nix-packaged**. It runs from the dev shell (`cargo run`) and ships as a
  `.deb`.
- crane pattern: `buildDepsOnly` + `buildPackage`, `cargoExtraArgs = "--locked -p buzzd"`,
  `doCheck = false` ("tests need a display, a keyring or a relay"), and a
  custom `src` filter (not `cleanCargoSource`, which would strip `assets/`
  and the `.ttf`s that `include_bytes!` needs):
  ```nix
  src = pkgs.lib.cleanSourceWith { src = ./.; filter = path: type:
    !(builtins.elem (baseNameOf path) [ "target" "target-deb" "dist" ".test-relay" ".claude" "docs" ]); };
  ```
- devShell:
  ```nix
  runtimeLibs = with pkgs; [ wayland vulkan-loader libxcb libxkbcommon libx11 libxcursor libxi
                             fontconfig freetype openssl dbus sqlite ];
  devShells.default = pkgs.mkShell {
    nativeBuildInputs = with pkgs; [ pkg-config cmake perl rustc cargo clippy rustfmt ];
    buildInputs = runtimeLibs ++ gstreamer-stuff ++ (with pkgs; [ vulkan-headers alsa-lib zstd ]);
    env = { RUST_BACKTRACE = "1"; LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs; };
  };
  ```
  `LD_LIBRARY_PATH` is needed because some libraries are **dlopen'd**,
  not linked: `wayland-backend` is built with the `dlopen` feature
  (libwayland-client), wgpu loads `libvulkan.so.1`, and the X11 cursor
  libraries are loaded on demand. `ldd target/debug/buzz-gpui` shows only
  libxcb, libxkbcommon(-x11), fontconfig, freetype and gstreamer as
  `DT_NEEDED`. Neither libwayland nor libvulkan appears there.
  SQLite, OpenSSL and libdbus are compiled in statically: rusqlite uses
  `bundled`, `openssl-src` is pulled in vendored by a transitive
  dependency, and keyring's `vendored` builds libdbus. So `sqlite`,
  `openssl` and `dbus` in `runtimeLibs` matter at build time at most,
  and `cmake`/`perl` are there for those vendored C builds.

### What packaging the GUI in nix would need (not done in buzz-gpui; proposed)

```nix
gui = craneLib.buildPackage (commonArgs // {
  pname = "bluebubbles-gpui";
  cargoExtraArgs = "--locked -p bluebubbles-gpui";
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
  doCheck = false;
  nativeBuildInputs = with pkgs; [ pkg-config cmake perl makeWrapper copyDesktopItems ];
  buildInputs = with pkgs; [ libxcb libxkbcommon fontconfig freetype dbus ];  # DT_NEEDED
  postInstall = ''
    install -Dm644 packaging/linux/bluebubbles-gpui.desktop $out/share/applications/bluebubbles-gpui.desktop
    for s in 128 256 512; do
      install -Dm644 assets/icons/icon-$s.png $out/share/icons/hicolor/''${s}x''${s}/apps/bluebubbles-gpui.png
    done
  '';
  # dlopen'd at runtime: add them to RPATH (as nixpkgs' zed-editor does) ...
  postFixup = ''
    patchelf --add-rpath ${pkgs.lib.makeLibraryPath (with pkgs; [
      wayland vulkan-loader libxkbcommon libx11 libxcursor libxi libxcb ])} $out/bin/bluebubbles-gpui
  '';
  # ... or, instead of patchelf:
  # postFixup = "wrapProgram $out/bin/bluebubbles-gpui --prefix LD_LIBRARY_PATH : ${makeLibraryPath [...]}";
  meta.mainProgram = "bluebubbles-gpui";
});
```

- Prefer `patchelf --add-rpath` over `wrapProgram`. A wrapper changes
  `/proc/self/exe` and `argv[0]`, which interacts with the single-instance
  handoff and `Exec=` in the desktop file. A wrapped `LD_LIBRARY_PATH`
  also leaks into child processes.
- On NixOS the Vulkan ICDs come from `/run/opengl-driver` (set by
  `hardware.graphics.enable`). On non-NixOS, nix-built GUI apps need
  nixGL. That is one reason buzz ships a `.deb` built on Ubuntu instead.
- Keep `third_party/` and `assets/` in the src filter, or the `[patch]`
  path and `include_bytes!` break.
- If you need GStreamer (voice notes or video in iMessage), add
  `wrapGAppsHook` or set `GST_PLUGIN_SYSTEM_PATH_1_0` through the wrapper.
  For images-only, skip GStreamer entirely.

### `.desktop` file (`packaging/linux/buzz-gpui.desktop`)

```ini
[Desktop Entry]
Type=Application
Name=Buzz
Comment=Chat with your community and manage your agents
Exec=buzz-gpui %U
Icon=buzz-gpui
Terminal=false
Categories=Network;Chat;InstantMessaging;
MimeType=x-scheme-handler/buzz;
StartupWMClass=buzz-gpui
X-GNOME-UsesNotifications=true
```

The basename `buzz-gpui.desktop`, `StartupWMClass`, `WindowOptions::app_id`,
the notification `desktop-entry` hint, the `Icon=` name and the binary
name **are all the same string**. GNOME matches windows to the dock entry
and notifications to settings by it.

### `.deb` (`scripts/package-deb.sh` + `[package.metadata.deb]`)

`scripts/package-deb.sh`: builds in `docker run ubuntu:26.04`, so the
binary links against Ubuntu's glibc and libraries, not `/nix/store`.
- Bind mounts: the repo at the same path, plus
  `~/.cache/buzz-gpui-deb/{cargo,rustup,target}` → `/root/.cargo`,
  `/root/.rustup` and `$repo/target-deb`, with
  `CARGO_TARGET_DIR=$repo/target-deb`. `target-deb/` in the repo is just
  that mount point (it shows up empty and root-owned). Add it to
  `.gitignore`.
- apt packages: `ca-certificates curl build-essential pkg-config cmake clang libxkbcommon-dev
  libxkbcommon-x11-dev libwayland-dev libxcb1-dev libx11-dev libx11-xcb-dev libvulkan-dev
  libfontconfig-dev libfreetype-dev libdbus-1-dev libssl-dev libzstd-dev` (+ gstreamer dev).
- rustup into the cached volume if it is missing. Then
  `cargo install --locked cargo-deb`, `cargo build --locked --release -p buzz-gpui`,
  `cargo deb --no-build -p buzz-gpui -o dist/`, and
  `chown -R "$OWNER" dist`. `rm -f dist/*.deb` runs first, so a stale
  package is never mistaken for the new one.

```toml
# crates/app/Cargo.toml
[package.metadata.deb]
name = "buzz-gpui"
maintainer = "Ben Carman <benthecarman@live.com>"
copyright = "2026, Buzz GPUI contributors"
license-file = ["../../LICENSE", "0"]
extended-description = "..."
section = "net"
priority = "optional"
depends = "$auto, libvulkan1, libxkbcommon-x11-0, libwayland-client0, libfontconfig1, libdbus-1-3, gnome-keyring | kwalletmanager | keepassxc"
assets = [
    ["target/release/buzz-gpui", "usr/bin/", "755"],          # cargo-deb maps target/release onto CARGO_TARGET_DIR
    ["../../packaging/linux/buzz-gpui.desktop", "usr/share/applications/", "644"],
    ["../../assets/icons/buzz-128.png", "usr/share/icons/hicolor/128x128/apps/buzz-gpui.png", "644"],
    ["../../assets/icons/buzz-256.png", "usr/share/icons/hicolor/256x256/apps/buzz-gpui.png", "644"],
    ["../../assets/icons/buzz-512.png", "usr/share/icons/hicolor/512x512/apps/buzz-gpui.png", "644"],
]
```

`$auto` picks up the `DT_NEEDED` libraries. The dlopen'd ones
(`libvulkan1`, `libwayland-client0`) must be listed by hand. Add
`ffmpeg` only if you shell out for HEIC.

Others: `scripts/package-macos.sh`, `scripts/package-windows.py`,
`packaging/windows/buzz.{ico,rc}` + `crates/app/build.rs` (`embed-resource`
for the exe icon), and `scripts/release.sh`.

---

## 10. GPUI-on-Linux gotchas the author hit

| issue | fix / where |
|---|---|
| Notification click / second launch doesn't raise the window on GNOME Wayland | patched `set_activation_token`; forward `XDG_ACTIVATION_TOKEN`, notification `ActivationToken`, tray `ProvideXdgActivationToken` (§1, §2, §5) |
| Ghost "unknown app" dock icon for ~180 ms at launch | patched: set app_id and title before the renderer (`6fe927ff`) |
| Pasted files arrive as path text | patched clipboard uri-list (`33a48e2a`) |
| GPUI clipboard can't write images on Wayland | `arboard` with `wayland-data-control`, kept alive for the app's lifetime |
| Emoji monochrome; ❤ ⚠ drawn by Inter | bundled renamed NotoColorEmoji with `m`→space; Inter cmap trimmed; `.emoji_fallback()` (§2) |
| Variable font weights unreliable | static font instances |
| `img("https://…")` never loads on Linux | `cx.set_http_client` with reqwest (`http.rs`) |
| No title bar on GNOME Wayland | `WindowDecorations::Client` + `window_border()` + own drag/menu/buttons; Hyprland has no minimize (`0a9df807`) |
| Window shadow/translucency broken on some GPUs | `window_border().shadow_size(px(0.))` |
| Compositor reports a 1×1 window while mapping or hiding | never save bounds below the min size |
| Images pixelated when drawn small | no mipmaps: pre-scale to device pixels (`bb2cdbd6`) |
| `RenderImage` is BGRA | swap R/B when building one from RGBA |
| ListState borrowed in the scroll handler → panic (abort on macOS) | defer loads out of `set_scroll_handler` (`2dd49c09`) |
| `request_animation_frame` outside a draw panics | `cx.notify()` instead (`3e386d74`) |
| Updating or reading your own entity from its render panics | focus the input entity directly or defer (`77f80317`) |
| Whole-window redraws (focus change) are slow with hundreds of rows | `AnyView::cached`, placeholder rows off-screen (`f239fa9b`, `1bed23a5`) |
| rust-embed debug builds read the source dir | `rust-embed` feature `debug-embed` (`591d866c`) |
| NVIDIA GPU stalls Vulkan's device probe on the test box | `VK_LOADER_DRIVERS_DISABLE=*nvidia*` (`7fc5210a`) |
| A binding with no context outranks contextual ones | bind composer overrides with `None` context after the app's |
| GPUI can't mask/rotate images, has no letter spacing, only two-stop gradients | design around it (rounded rects only) |
| Headless/dev sessions have no Secret Service | files backend / dev env var; `.deb` depends on a keyring provider |
