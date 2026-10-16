# Kopico Desktop

Kopico Desktop is a small background app that **receives files and text sent with
[croc](https://github.com/schollz/croc)** — without you having to type a `croc` command
every time. Leave it running in the system tray: files land in `~/Downloads/Kopico`, and
text goes straight to your clipboard, ready to paste.

It is a GNOME-style app (GTK 4 + libadwaita, written in Rust) that runs on **Linux** and
**macOS**.

## What it does

- **Receives automatically.** While Kopico is on, it keeps asking croc for incoming
  transfers using a code you agree on with the sender.
- **Files** are saved to `~/Downloads/Kopico`. A file that already exists is never
  overwritten — the new one is saved as `name (1).ext`.
- **Text** (`croc send --text …`) is copied to the clipboard.
- **Lives in the tray / menu bar.** Closing the window hides it; the tray icon turns
  green while Kopico is on. Tray menu: *Enable*, *Settings*, *Quit*.
- **Notifies you** when files or text arrive, and when Kopico is switched on.
- **Three ways to find the sender:**

  | Mode | Use when | Checks every |
  |---|---|---|
  | **Local Network** (default) | Sender is on the same network. Optionally give the sender's IP — needed where automatic discovery doesn't work (e.g. on macOS) | 3 s |
  | **Own Relay** | You run your own `croc relay` | 10 s |
  | **Public Relay** | Sender is anywhere on the internet | 30 s |

  Why polling and why these intervals: see [docs/receiver.md](docs/receiver.md).

## Sending to Kopico

On the other machine, use the same code Kopico listens for (Advanced → Receive Code):

```sh
CROC_SECRET=<your-code> croc send photo.jpg            # a file (or several, or a folder)
CROC_SECRET=<your-code> croc send --text "hello"       # text → lands on the clipboard
```

croc requires the code in the `CROC_SECRET` environment variable on Linux/macOS. If you
set a **Sender IP** in Kopico, don't use `--no-local` on the sender — Kopico connects to
the small relay croc starts on the sender (port 9009).

## Requirements

| | Linux | macOS |
|---|---|---|
| Rust (2024 edition) | ✓ | ✓ |
| GTK ≥ 4.22, libadwaita ≥ 1.9 | distro packages (GNOME 50 era) | MacPorts |
| `blueprint-compiler`, `glib-compile-resources`, `pkg-config` | ✓ | MacPorts |
| [croc](https://github.com/schollz/croc) on `PATH` (or in `/usr/local/bin`, `/opt/homebrew/bin`, `/opt/local/bin`, `~/.local/bin`, `~/go/bin`) | ✓ | ✓ |
| Adwaita icon theme | usually installed | `sudo port install adwaita-icon-theme` |
| Tray icon on GNOME Shell | *AppIndicator and KStatusNotifierItem Support* extension | — |
| `wl-clipboard` (optional, Wayland) | lets Kopico set the clipboard while unfocused | — |

The GTK/libadwaita minimums come from the crate feature flags (`gnome_50`, `v1_9`), which
are checked at build time. Check yours with
`pkg-config --modversion gtk4 libadwaita-1`.

> **macOS:** install the GTK stack from **one** package manager. Mixing Homebrew and
> MacPorts libraries makes the app crash at startup — see
> [docs/development.md](docs/development.md#macos-one-glib-only).

## Build and run

### Linux

```sh
# Fedora:         sudo dnf install gtk4-devel libadwaita-devel blueprint-compiler glib2-devel
# Debian/Ubuntu:  sudo apt install libgtk-4-dev libadwaita-1-dev blueprint-compiler libglib2.0-dev-bin
./dev-install.sh      # once: installs the .desktop file (needed for notifications)
cargo run
```

### macOS

```sh
sudo port install gtk4 libadwaita blueprint-compiler adwaita-icon-theme pkgconfig
cargo build
./dev-install.sh      # once: creates and registers "~/Applications/Kopico Desktop.app"
cargo run             # runs Kopico from that app bundle (needed for notifications)
```

On the first start macOS asks to allow notifications; Kopico Desktop then appears in
System Settings → Notifications.

### Meson (install / packaging)

```sh
meson setup builddir
ninja -C builddir
ninja -C builddir install
```

`cargo build` alone is enough for development — `build.rs` compiles the Blueprint UI and
resources itself.

### Tests

```sh
cargo test                                    # unit tests
cargo test -- --ignored --test-threads=1      # end-to-end tests with real croc processes
```

Debug the receiver with `KOPICO_DEBUG=1 cargo run` (logs every croc run; never the code,
password or received text).

More: [docs/development.md](docs/development.md).

## Libraries

| Crate | Why |
|---|---|
| `gtk4` 0.11 + `libadwaita` 0.9 | UI toolkit and GNOME widgets. Versions move in lockstep (libadwaita 0.9 is built on gtk4 0.11) |
| `tray-icon` 0.26 (`ksni` backend) | Tray / menu bar icon. Its default Linux backend loads GTK 3, which can't run next to GTK 4 |
| `async-channel` | Delivers events from the background croc thread to the UI without polling |
| `dirs-next` | Finds the Downloads and home folders |
| `notify-rust` (macOS) | Notifications through Apple's `UNUserNotificationCenter` |
| `objc2-app-kit`, `objc2-foundation` (macOS) | Writes received text to the macOS clipboard (`NSPasteboard`) |

Linux notifications use GTK's own `gio::Notification`; no extra crate.

## Platform constraints and decisions

The short version — each one is explained in
[docs/platform-decisions.md](docs/platform-decisions.md):

| Constraint | Decision |
|---|---|
| croc can't wait for a sender; each run finds one or exits | Run croc in a loop; intervals per mode, 60 s back-off when a relay rate-limits |
| Public croc relays allow ~30 joins/min | Public mode checks every 30 s; **Local** is the default |
| croc's LAN discovery fails on some machines (seen on macOS) | Optional **Sender IP** — croc connects to the sender directly |
| `tray-icon`'s default Linux backend uses GTK 3 | Use the `ksni` (D-Bus) backend |
| GLib has no macOS notification backend | Linux: `gio::Notification`; macOS: `notify-rust` |
| macOS notifications require a signed app bundle | `cargo run` runs Kopico from an ad-hoc-signed `Kopico Desktop.app` |
| GNOME only shows notifications for apps with a `.desktop` file | `dev-install.sh` installs one; Meson installs it |
| GTK's clipboard copies nothing on macOS | Write to `NSPasteboard` directly |
| Wayland may refuse clipboard writes from an unfocused app | Use `wl-copy` when available, GTK otherwise |
| Two GLib copies in one process crash GTK | One package manager for the GTK stack on macOS |

## Known limitations

- **Settings are not saved yet** — receive code, relay mode, sender IP etc. reset when the
  app restarts.
- **The default receive code is in the source code.** Anyone who knows it and can reach
  you (or your relay) can send you files. Change it in Advanced, or in `src/main.rs`.
- Clicking a notification on macOS does nothing yet (on Linux it opens the window).
- macOS release builds would need a real Apple Developer signature instead of the ad-hoc
  development one.
- `meson.build` still asks for GTK ≥ 4.14 / libadwaita ≥ 1.6 although the code needs
  4.22 / 1.9.

## Project layout

```
src/main.rs           app setup, actions (= app state), window, clipboard
src/receiver.rs       croc receive loop (background thread)
src/tray.rs           tray icon and menu
src/notification.rs   system notifications (Linux: gio, macOS: notify-rust)
data/ui/window.blp    UI (Blueprint)
data/style.css        app CSS
data/macos/           macOS bundle metadata (Info.plist)
scripts/macos-app.sh  builds, signs and runs Kopico Desktop.app (macOS `cargo run` runner)
dev-install.sh        registers the dev build for notifications (Linux and macOS)
docs/                 deeper documentation
```

## Documentation

- [docs/development.md](docs/development.md) — setup, running, debugging, testing, troubleshooting
- [docs/architecture.md](docs/architecture.md) — how the app is put together
- [docs/receiver.md](docs/receiver.md) — the croc receive loop and croc's behaviour it relies on
- [docs/platform-decisions.md](docs/platform-decisions.md) — platform constraints and the decisions taken
- [docs/gtk4-libadwaita-patterns.md](docs/gtk4-libadwaita-patterns.md) — GTK 4 / libadwaita coding patterns used here

## License

GPL-3.0-or-later (as declared in `meson.build`).
