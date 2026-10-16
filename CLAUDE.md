# Kopico Desktop — AI Agent Context

## Project Overview

**kopico-desktop** is a GNOME desktop application written in **Rust** using:
- [`gtk4`](https://crates.io/crates/gtk4) — GTK4 Rust bindings (`adw::gtk::*` re-exports GTK widgets)
- [`libadwaita`](https://crates.io/crates/libadwaita) crate alias `adw` — Adwaita widgets for GNOME HIG compliance
- **Blueprint** markup (`.blp` files) — compiled to GtkBuilder XML (`.ui`) then bundled as a GResource
- **Meson** — primary build system (orchestrates Blueprint → `.ui` → `.gresource` → `cargo`)
- **Cargo** — also works standalone via `build.rs` (no Meson required for dev)

App ID: `in.tl2a.kopico-desktop`

Human-facing docs: `README.md` (purpose, requirements, build per platform) and `docs/`
(`development.md`, `architecture.md`, `receiver.md`, `platform-decisions.md`). When a
platform constraint or decision changes, update `docs/platform-decisions.md` too.

---

## Build Commands

```bash
# Dev (cargo-only, no Meson needed)
cargo build
cargo run          # macOS: runs from "~/Applications/Kopico Desktop.app" (notifications)

# Production / install (Meson)
meson setup builddir
ninja -C builddir
ninja -C builddir install

# Check Blueprint syntax without building
blueprint-compiler compile --output /dev/null data/ui/window.blp
```

> `build.rs` automatically runs `blueprint-compiler` + `glib-compile-resources` as part of
> every `cargo build`. The compiled `.gresource` is embedded into the binary via
> `include_bytes!(env!("GRESOURCE_FILE"))`.

---

## Project Structure

```
kopico-desktop/
├── build.rs                                    # Drives blp→ui→gresource for cargo builds
├── dev-install.sh                              # Registers the dev build (.desktop / .app) for notifications
├── .cargo/config.toml                          # macOS: `cargo run` runner → scripts/macos-app.sh
├── scripts/macos-app.sh                        # Builds/signs/registers Kopico Desktop.app, runs from it
├── meson.build                                 # Meson build (Blueprint + gresource + cargo)
├── Cargo.toml
├── src/
│   ├── main.rs                                 # Entry point, app actions, window + signal handlers
│   ├── receiver.rs                             # croc receive loop (background thread)
│   ├── tray.rs                                 # System tray icon + menu (tray-icon crate)
│   └── notification.rs                         # System notifications (gio on Linux, notify-rust on macOS)
└── data/
    ├── in.tl2a.kopico-desktop.desktop         # Linux desktop entry (dev-install.sh patches Exec=)
    ├── macos/Info.plist                        # macOS bundle metadata (scripts/macos-app.sh builds the .app)
    ├── in.tl2a.kopico-desktop.gresource.xml   # GResource manifest
    ├── style.css                               # App CSS (auto-loaded by AdwApplication)
    └── ui/
        └── window.blp                          # ✏️  Edit UI here (Blueprint markup)
```

### UI Pipeline

```
data/ui/*.blp
  → [blueprint-compiler compile]    → $OUT_DIR/data/ui/*.ui   (GtkBuilder XML)
  → [glib-compile-resources]        → $OUT_DIR/in.tl2a.kopico-desktop.gresource
  → [include_bytes! in main.rs]     → embedded in binary
```

---

## Architecture

### Imports

```rust
use adw::prelude::*;                         // Must be in scope for property methods
use adw::{Application, ApplicationWindow, …}; // Adwaita widgets live in adw::
use adw::gtk::{Button, Label, ListBox, …};   // GTK widgets re-exported via adw::gtk
use gtk4::gio;                               // For gio::Resource / resources_register
```

`adw` is the crate alias for `libadwaita`. Never import from both `gtk4::` and `adw::gtk::`
for the same type — pick one path consistently.

### Resource Registration

```rust
fn main() -> gtk4::glib::ExitCode {
    let bytes = include_bytes!(env!("GRESOURCE_FILE"));
    let resource = gio::Resource::from_data(&gtk4::glib::Bytes::from_static(bytes))
        .expect("Failed to load gresource");
    gio::resources_register(&resource);
    // …
}
```

### Loading Widgets from UI Resource

```rust
let builder = gtk4::Builder::from_resource("/in/tl2a/kopico-desktop/ui/window.ui");
let window: adw::ApplicationWindow = builder.object("window").expect("window");
let button: adw::gtk::Button      = builder.object("button").expect("button");
```

Widget IDs in `.blp` files map 1-to-1 to `builder.object("id")` calls.
Always annotate the type — Rust cannot infer it from `object()`.

---

## Blueprint UI Files (`data/ui/*.blp`)

### Conventions

- One `.blp` file per top-level window/dialog.
- All user-visible strings must be wrapped with `_()` for i18n:
  `label: _("Click me!");`
- Widget IDs use `snake_case` and match the Rust variable names exactly.
- CSS classes use the `styles [...]` block:
  ```blueprint
  styles ["pill", "suggested-action"]
  ```
- Adwaita breakpoints are declared inline in Blueprint (no Rust `Breakpoint` code needed):
  ```blueprint
  Adw.Breakpoint {
    condition ("max-width: 600sp")
    setters { split_view.collapsed: true; }
  }
  ```

### Adding a New UI File

1. Create `data/ui/my_dialog.blp`.
2. Add `<file preprocess="xml-stripblanks">ui/my_dialog.ui</file>` to
   `data/in.tl2a.kopico-desktop.gresource.xml`.
3. Add `'data/ui/my_dialog.blp'` to the `blueprints` list in `meson.build`.
4. `build.rs` auto-discovers all `*.blp` in `data/ui/` — no changes needed there.
5. Load with `gtk4::Builder::from_resource("/in/tl2a/kopico-desktop/ui/my_dialog.ui")`.

---

## GTK4 + Libadwaita Patterns

### Property Bindings (bidirectional sync)

```rust
source
    .bind_property("some-prop", &target, "active")
    .bidirectional()
    .sync_create()   // immediately syncs current value
    .build();
```

### App State = Stateful Actions (preferred over bindings for app-wide state)

App-wide toggles live in `gio::SimpleAction::new_stateful(name, None, &bool)` registered
in `setup_app()`. Widgets bind to them in Blueprint with `action-name: "app.<name>";` —
a `Gtk.ToggleButton` then reflects and changes the state with **no Rust code**, and the
tray menu dispatches to the same action. Side effects go in `connect_change_state`
(call `action.set_state(...)` there). Read state with `action_bool(app, "<name>")`.

`Adw.SwitchRow` is not actionable for state — set it from `action_bool()` and call
`app.change_action_state()` from `connect_active_notify` (see `tray_row` in `main.rs`).

### Signal Handlers (clone pattern)

```rust
{
    let cloned = widget.clone();
    button.connect_clicked(move |_| {
        cloned.do_something();
    });
}
```
Use a scoped block `{ }` to limit the clone to the closure registration.

### Toasts

```rust
toast_overlay.add_toast(adw::Toast::new("Message text"));
// or with custom duration / action:
let toast = adw::Toast::builder()
    .title("Copied!")
    .timeout(2)
    .build();
toast_overlay.add_toast(toast);
```

### Tabs: ViewStack + ViewSwitcher (current window layout)

```blueprint
Adw.ApplicationWindow window {
  Adw.Breakpoint {                       // narrow: tabs move to the bottom bar
    condition ("max-width: 550sp")
    setters { switcher_bar.reveal: true; view_switcher.visible: false; }
  }

  content: Adw.ToolbarView {
    [top]
    Adw.HeaderBar { title-widget: Adw.ViewSwitcher view_switcher { stack: view_stack; policy: wide; }; }

    content: Adw.ViewStack view_stack {
      Adw.ViewStackPage { name: "home"; title: _("Home"); icon-name: "go-home-symbolic"; child: …; }
    };

    [bottom]
    Adw.ViewSwitcherBar switcher_bar { stack: view_stack; }
  };
}
```
Add a tab = add an `Adw.ViewStackPage` (needs `title` + `icon-name` for the switcher).
Settings-style tabs use `Adw.PreferencesPage` (give it a `title`) → `Adw.PreferencesGroup` → rows.

### Custom CSS

`data/style.css` is registered in the gresource at `/in/tl2a/kopico-desktop/style.css`;
`AdwApplication` loads it automatically (no `CssProvider` code). Use libadwaita CSS
variables (`var(--accent-bg-color)`, …) instead of hard-coded colours so light/dark and the
system accent colour keep working. `build.rs`/`meson.build` pass `data/` as a second
`--sourcedir`, so new hand-written resources only need a `<file>` entry in the manifest.

### NavigationView (multi-page navigation)

```blueprint
Adw.NavigationView nav {
  Adw.NavigationPage {
    tag: "main";
    title: _("Main");
    child: Adw.ToolbarView { … };
  }
}
```
Push/pop in Rust: `nav.push_by_tag("detail")` / `nav.pop()`.

---

## Adwaita HIG Checklist

- Use `Adw.ApplicationWindow` (not `Gtk.ApplicationWindow`) — it supports breakpoints.
- Use `Adw.ToolbarView` + `Adw.HeaderBar` instead of `Gtk.HeaderBar` directly.
- Use `Adw.StatusPage` for empty states.
- Use `Adw.PreferencesPage` / `Adw.PreferencesGroup` / `Adw.ActionRow` for settings.
- Never hardcode pixel sizes — use `sp` units in breakpoints.
- All destructive actions: CSS class `"destructive-action"`.
- Primary actions: CSS class `"suggested-action"`, optionally add `"pill"`.

---

## Crate Versions (Cargo.toml)

| Crate | Version | Notes |
|---|---|---|
| `gtk4` | `0.11` | `features = ["gnome_50"]` |
| `libadwaita` (alias `adw`) | `0.9` | `features = ["v1_9"]` |
| `tray-icon` | `0.26` | `default-features = false, features = ["ksni"]` — see System Tray |
| `async-channel` | `2` | Background thread → GTK main loop events (no polling) |
| `dirs-next` | `2` | Downloads / home folder paths |
| `objc2-app-kit`, `objc2-foundation` | `0.3` | macOS only: `NSPasteboard` for received text (same versions as `tray-icon`) |
| `notify-rust` | `4.18` | macOS only, `default-features = false, features = ["preview-macos-un"]` (UNUserNotificationCenter) |

- `libadwaita 0.9` is built against `gtk4 0.11`; bump them together (`cargo info libadwaita@<ver>`
  lists the matching `gtk` version). `cargo tree -i gtk4` must show a single version.
- Use `gtk4::gio` / `gtk4::glib` — do **not** add `gio` or `glib` as separate crates; a
  different version gives incompatible types and a second copy of the bindings.
- Feature flags are *minimum system library* versions, checked at build time:
  `gnome_50` = GTK 4.22 + GLib/GIO 2.88, `v1_9` = libadwaita 1.9. Never set a flag newer
  than the installed library (`pkg-config --modversion gtk4 libadwaita-1`).

---

## System Tray (`src/tray.rs`)

With tray support on (default), closing the window **hides it to the tray** instead of
quitting. Tray menu:

| Item | Action | Behaviour |
|---|---|---|
| Enable (checkbox) | `app.enabled` | Stateful bool; same state as the Home page power toggle. Icon turns **green** when on |
| Settings | `app.show-window` | Re-presents the hidden window (creates it if needed) |
| Quit | `app.quit` (`Ctrl/Cmd+Q`) | `app.quit()` → shutdown drops the tray + hold guard → process exits |

Not in the tray menu: `app.tray` (stateful bool, default `true`) — Advanced →
*Show in System Tray*. Turning it off drops the `Tray` (icon + hold guard), and the
close button then quits normally, so the app can never be hidden with no way back.

How it fits together:

- **Menu item id == action name.** `MenuEvent` handler → `glib::idle_add_once` →
  `gio::Application::default().activate_action(id)`. The handler can run on a
  non-GTK thread (ksni worker on Linux), so only the `String` id crosses threads — never
  touch widgets inside it. Add a tray item = add an `app.*` action + a menu item with that id.
- **Lifecycle.** `tray::Tray::new()` creates the icon and takes `app.hold()` so the process
  survives with every window hidden; dropping the `Tray` removes both. `setup_app()` keeps it
  in an `Rc<RefCell<Option<Tray>>>`, created at startup, recreated/dropped by `app.tray`,
  and dropped in `connect_shutdown`. `install_menu_handler()` runs once at startup.
- **Single window.** `connect_activate` → `show_window()` reuses `app.windows()[0]`, so
  relaunching the app or clicking Settings never creates a duplicate.
- **Close → hide.** `window.connect_close_request` hides the window and returns
  `Propagation::Stop` only while `app.tray` is on; otherwise it returns `Proceed` and the
  app exits as usual. With the tray on, only `app.quit` really exits.

Platform notes:

- **Linux:** must use the `ksni` backend. The default `libappindicator` backend loads
  **GTK 3**, which aborts when mixed with GTK 4 in one process. GNOME Shell shows tray
  icons only with the *AppIndicator and KStatusNotifierItem Support* extension (KDE etc. work
  out of the box).
- **macOS:** the icon is a template image (drawn black, recoloured by the menu bar). The Dock
  icon stays while the window is hidden; hiding it would need
  `NSApplication.setActivationPolicy(.accessory)` via `objc2-app-kit`.
- The tray icon is generated in code (`tray_icon_image(colour)`); replace it with real artwork
  via `Icon::from_rgba` when the app has an icon. `Tray::set_enabled()` swaps it:
  on → `GREEN` drawn as-is; off → `NORMAL` (macOS: black **template**, recoloured by the menu
  bar; elsewhere Adwaita blue). A template icon can never be coloured — that's why the
  enabled icon must use `set_icon`, not `set_icon_templated`.

### Notifications (`src/notification.rs`)

`notification::send_enabled(app)` fires from `app.enabled`'s `change_state` on an
off → on transition only (never at startup).

Also: files received, text received (the text itself is never in a notification). One API,
`send_enabled` / `send_files_received` / `send_text_received`, two backends:

- **Linux:** `gio::Notification` + `app.send_notification()`; clicking it runs
  `app.show-window`. GNOME Shell only shows these for apps with an installed
  `in.tl2a.kopico-desktop.desktop` file — `./dev-install.sh` installs it for dev builds.
- **macOS:** `notify-rust` with `preview-macos-un` → Apple's `UNUserNotificationCenter`
  (not the deprecated `NSUserNotificationCenter`). `gio::Notification` can't be used — GLib
  has no macOS backend in MacPorts/Homebrew builds. Requirements, all verified on macOS 26:
  - The process must run **from inside an `.app` bundle** (needs a bundle identifier) —
    a bare `target/debug` binary gets `NoBundleIdentifier`; Kopico then logs one line and
    skips notifications.
  - The bundle must be **signed** — unsigned, `request_auth` is answered "not allowed"
    without a prompt. Ad-hoc signing (`codesign --sign -`) is enough for development.
  - `notification::init()` (startup) calls `request_auth`; the first time, macOS shows the
    "Allow notifications?" prompt and lists Kopico Desktop in System Settings →
    Notifications. Permission is tied to the bundle id and survives rebuilds/re-signing.
  - `scripts/macos-app.sh` does all of this: copies the binary into
    `~/Applications/Kopico Desktop.app` (Info.plist from `data/macos/`), ad-hoc signs it,
    registers it (`lsregister`) and runs it. `.cargo/config.toml` makes it the `cargo run`
    runner on macOS (non-app binaries such as tests pass straight through);
    `./dev-install.sh` calls `scripts/macos-app.sh install`.
  - Use the **async** API on the GTK main loop (`glib::spawn_future_local` +
    `show_async()` / `request_auth()`). The blocking `show()` fails with
    `Mainthread not running` from any thread, because GTK's loop never idles inside
    `NSRunLoop`. The request itself is sent from notify-rust's worker thread.
  - Clicking a notification does nothing yet.

---

## Receiving Files (`src/receiver.rs`)

While Kopico is enabled (`app.enabled`), a background thread runs **croc** (external CLI,
must be installed) again and again, because a croc receiver can't wait for a sender:

| `app.relay-mode` | croc flags | Poll every | Wait per run | Rate-limit back-off |
|---|---|---|---|---|
| `local` (default) | `--local`, or only `--ip <app.sender-ip>` when set | 3 s | croc exits by itself (~0.5–1 s) | — |
| `own-relay` | `--relay <app.relay-address>`, `CROC_PASS` | 10 s | 8 s | 60 s |
| `public-relay` | — | 30 s | 25 s | 60 s |

Intervals count from the start of a run. Public relays allow ~30 joins/min per IP
(`croc relay --source-join-limit`), hence 30 s + 60 s back-off. Received files go to
`~/Downloads/Kopico` (`receiver::default_out_dir()`).

**croc 11.5 output facts the parser relies on (re-verify on croc upgrades):**

- Progress/status goes to **stderr**, separated by `\r` *and* `\n`; stdout only carries
  errors. There is no `Received 'name'` line, and names in the progress bar are truncated.
- Phases: `looking for sender…` / `waiting for sender…` → `authenticating code…` →
  `Receiving N files (…)`. The wait limit applies **only before** a sender is found — a
  transfer is never killed.
- `--local`, no sender: exit 1, stdout `could not connect to : found no relay addresses`.
- `--local --ip host[:port]` (port defaults to 9009) skips multicast discovery and dials
  the sender's built-in relay directly — needed where discovery fails (seen on macOS).
  No sender there: exit 1 in ~0.5 s, stdout `… connection refused`.
- Relay, no sender: croc waits in the room forever → our wait limit kills it.
- Rate limited: stdout `… relay admission rate limited` (exit 1) or stderr
  `The relay is rate limiting reconnects…`. Match `rate limit` only — not `429`.
- Success: exit 0. Received names = new entries in the output folder (diff before/after).
  `--rename` keeps existing files and guarantees new names, so the diff is reliable.
- The code goes in `CROC_SECRET` (croc refuses codes in argv on UNIX); minimum 6 chars.
- **Text** (`croc send --text …`): stderr `Receiving text message (N B)`, stdout = the
  exact text (no trailing newline added), no file written → `ReceiverEvent::Text`, which
  `copy_to_clipboard()` in `main.rs` puts on the clipboard. The text is never logged
  (`KOPICO_DEBUG` shows only its length) or shown in notifications.

**Clipboard:** macOS writes to `NSPasteboard` directly — GTK 4.22's macOS clipboard logs
`'text/plain' is not a valid UTI string` and copies nothing. Linux on Wayland tries
`wl-copy` (from `wl-clipboard`) first; its data-control protocol can set the clipboard
without keyboard focus when supported by the compositor. If `wl-copy` is missing or fails,
Kopico falls back to `gdk::Display::clipboard()`. Compositors without data-control support
may still ignore writes from an unfocused app (for example, while Kopico is hidden in the
tray).

**Threading:** `receive_loop` runs on a `croc-receive` thread; both pipes are drained by
reader threads (a full pipe would block croc). Events (`ReceiverEvent::Received/Failed`)
reach GTK through `async-channel` + `glib::spawn_future_local` — never poll with
`glib::idle_add` (it spins the main loop). `ReceiverHandle::stop()` kills croc and returns
within ~200 ms. `find_croc()` also checks `/usr/local/bin`, `/opt/homebrew/bin`,
`/opt/local/bin`, `~/.local/bin`, `~/go/bin` (GUI launches get a minimal `PATH`).

**Settings** are stateful string actions created in `setup_app()`: `app.relay-mode`,
`app.relay-address` (`host:port`), `app.relay-password`, `app.receive-code` (≥ 6 chars),
`app.sender-ip` (optional, Local mode only; `receiver::is_valid_sender_ip`). An
invalid value is ignored by `change_state` (the row gets the `error` style); an accepted
one restarts the loop. `receiver_config()` turns them into a `RelayMode`.

**Debugging:** `KOPICO_DEBUG=1 cargo run` logs every croc run — command line (the code
and password are never printed), exit status, croc's status lines and stdout, and the
outcome (`NoSender`, `Received`, `RateLimited`, …).
`stdout: could not connect to : found no relay addresses` in Local mode means croc's
multicast discovery found nobody (croc 11.5 gives up in < 1 s; unreliable on macOS) —
set a Sender IP, which skips discovery.

**Tests:** `cargo test` runs the parser unit tests. The end-to-end tests start a local
`croc relay` / `croc send` and real croc processes (own relay, local + `--ip`, rate
limiting, stop latency):
`cargo test -- --ignored --test-threads=1`. `--local` discovery can't be tested on one
machine (croc doesn't see its own multicast) — use two devices on the same network.

---

## Common Pitfalls

| Mistake | Fix |
|---|---|
| Forgetting `use adw::prelude::*` | Widget property setters won't compile without it |
| Using `gtk4::ApplicationWindow` instead of `adw::ApplicationWindow` | Adwaita breakpoints need `AdwApplicationWindow` |
| Missing `fs = import('fs')` in `meson.build` | `fs.stem()` fails at configure time |
| Widget ID mismatch between `.blp` and `builder.object("id")` | Panics at runtime — keep IDs in sync |
| Editing the generated `.ui` file | It's regenerated from `.blp` every build — edit `.blp` only |
| `tray-icon` with default features on Linux | Pulls in GTK 3 → crash; keep `default-features = false, features = ["ksni"]` |
| No Kopico notifications on macOS / not listed in Settings → Notifications | Start via `cargo run` or the `.app` (not `target/debug/kopico-desktop`); the bundle must be signed — `scripts/macos-app.sh` does both |
| `gio::Notification` on macOS | Silently dropped (no GLib backend) — `notification.rs` uses notify-rust there |
| notify-rust `show()` / `*_blocking()` on macOS | `Mainthread not running` under GTK — use `show_async()` / `request_auth()` with `glib::spawn_future_local` |
| Touching widgets inside `MenuEvent::set_event_handler` | May be off the main thread — forward the id with `glib::idle_add_once` |
| Parsing croc **stdout** for received files | croc prints progress on stderr and never names files reliably — diff the output folder |
| Timing out a croc run that already found a sender | Kills the transfer; the wait limit only applies before `authenticating code…` |
| `gdk::Clipboard::set_text` on macOS | Silently copies nothing (invalid UTI) — use `copy_to_clipboard()` (NSPasteboard) |
| `glib::idle_add` returning `Continue` to poll a channel | Busy-loops the main thread; use `async-channel` + `glib::spawn_future_local` |
| Calling `build_ui()` directly from `activate` | Creates duplicate windows — go through `show_window()` |
| `GLib-GObject-CRITICAL … type '(null)'` then segfault at startup | Two GLib installs linked (Homebrew + MacPorts). Check `otool -L target/debug/kopico-desktop`; don't put Homebrew dirs in `PKG_CONFIG_PATH` |
