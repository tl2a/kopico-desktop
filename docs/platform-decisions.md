# Platform constraints and decisions

The constraints we ran into on Linux and macOS, and what we decided. Each entry: the
constraint, the decision, and why — so a future change can tell whether the reason still
holds.

- [UI toolkit and crate versions](#ui-toolkit-and-crate-versions)
- [System tray](#system-tray)
- [Notifications](#notifications)
  - [Linux](#linux-notifications)
  - [macOS](#macos-notifications)
- [Clipboard](#clipboard)
- [Receiving with croc](#receiving-with-croc)
- [macOS: GTK libraries](#macos-gtk-libraries)
- [Open questions](#open-questions)

## UI toolkit and crate versions

**Constraint.** The Rust bindings `gtk4` and `libadwaita` are released together; each
libadwaita version is built against exactly one gtk4 version. Only one copy of a native
library may be linked, so mismatched versions don't build
(`gtk4-sys … conflicts with a previous package which links to gtk-4`).

**Decision.** `gtk4 0.11` + `libadwaita 0.9`, always bumped together. `gio`/`glib` are used
through `gtk4::gio` / `gtk4::glib`, never added as separate crates (a separate version is a
second, incompatible copy of the bindings).

**Feature flags** (`gnome_50`, `v1_9`) set the minimum *system* GTK 4.22 / libadwaita 1.9
and are checked at build time. They can be lowered for older distributions as long as the
code doesn't use newer APIs.

## System tray

**Constraint.** `tray-icon`'s default Linux backend (`libappindicator`) loads **GTK 3**. GTK 3
and GTK 4 cannot be loaded in the same process.

**Decision.** `tray-icon` with `default-features = false, features = ["ksni"]` — the
StatusNotifierItem D-Bus backend, no GTK. The feature has no effect on macOS.

Consequences:

- ksni delivers menu clicks on its own thread. Menu item ids are action names; the handler
  only forwards the id to the GTK thread (`glib::idle_add_once`) and activates `app.<id>`.
- GNOME Shell shows tray icons only with the *AppIndicator and KStatusNotifierItem Support*
  extension; KDE and most other desktops show them natively.
- macOS: the off-state icon is a **template image** (the menu bar recolours it for
  light/dark). Template images are always monochrome, so the green "on" icon is set as a
  normal image.
- Tray support can be switched off (Advanced). Without the tray, the close button really
  quits — the app can never end up hidden with no way back.

## Notifications

GLib's `gio::Notification` is the GNOME way, but GLib has **no macOS backend** in the
MacPorts/Homebrew builds — notifications sent through it on macOS silently disappear. So
there are two backends behind one API in `src/notification.rs`.

### Linux notifications

**Decision.** `gio::Notification` + `app.send_notification()`. Clicking one opens the
window.

**Constraint.** GNOME Shell only shows notifications for an app it has a `.desktop` file
for. `dev-install.sh` installs one for development (with `Exec=` patched to the debug
binary); Meson installs it for real installs.

### macOS notifications

**Decision.** `notify-rust` with the `preview-macos-un` feature, i.e. Apple's current
`UNUserNotificationCenter` — not the deprecated `NSUserNotificationCenter`.

We first used notify-rust's default (deprecated) API. It never asks for permission, so
Kopico never appeared in System Settings → Notifications, and macOS 26 had no reason to show
its notifications. The current API fixes both, but has requirements (all verified on
macOS 26.7):

| Requirement | What happens otherwise | How we meet it |
|---|---|---|
| Run from inside an **`.app` bundle** with a bundle id | `NoBundleIdentifier` | `cargo run` starts Kopico from `~/Applications/Kopico Desktop.app` |
| The bundle is **signed** | permission is refused without a prompt | ad-hoc signature (`codesign --sign -`), no Apple account needed |
| Bundle id known to LaunchServices | no proper attribution | `lsregister -f` |
| Permission requested | not listed in Settings | `notification::init()` calls `request_auth` at startup |

`scripts/macos-app.sh` does the bundle work and is the `cargo run` runner on macOS
(`.cargo/config.toml`). Permission is tied to the bundle id and survives rebuilds.

**Constraint.** notify-rust's blocking calls (`show()`, `request_auth_blocking()`) need the
main thread to idle inside `NSRunLoop`. GTK's main loop idles in GLib's poll instead, so
they fail with `Mainthread not running`. **Decision:** use the async calls (`show_async()`,
`request_auth()`) as futures on the GTK main loop; the request itself is sent from
notify-rust's worker thread.

Not done yet: reacting to a click on a macOS notification; a real Developer ID signature
for distribution.

## Clipboard

**Constraint (macOS).** GTK 4.22's macOS clipboard passes MIME types where AppKit expects
UTIs (`'text/plain;charset=utf-8' is not a valid UTI string`) and copies nothing.
**Decision:** write to `NSPasteboard` directly via `objc2-app-kit` (already in the build
through tray-icon). The text stays on the clipboard after Kopico quits.

**Constraint (Linux/Wayland).** Compositors may ignore clipboard writes from an app without
keyboard focus — exactly Kopico's situation while it sits in the tray. **Decision:** on
Wayland use `wl-copy` (from `wl-clipboard`), whose data-control protocol can set the
clipboard without focus where supported; fall back to GTK's clipboard otherwise. On X11
GTK's clipboard is used; Kopico owns the content while it runs.

Received text replaces the current clipboard without asking.

## Receiving with croc

Details in [receiver.md](receiver.md). The decisions in short:

| Constraint | Decision |
|---|---|
| croc's receiver can't wait for a sender | run croc in a loop on a background thread |
| public relays admit ~30 joins/min | poll every 30 s there; 60 s back-off on rate limit; **Local** is the default |
| a relay receiver waits forever | end a run after 8 s / 25 s — but only before a sender is found |
| croc prints progress on stderr and never names received files reliably | detect phases on stderr; diff the output folder; `--rename` so nothing is overwritten |
| LAN discovery fails on some machines (seen on macOS) | optional Sender IP → `croc --ip` |
| codes can't be passed as arguments | `CROC_SECRET` (and `CROC_PASS` for relay passwords) |
| background thread → UI | `async-channel` + `glib::spawn_future_local`; never poll from `idle_add` (it spins a CPU core) |

## macOS: GTK libraries

**Constraint.** If the build links some GTK/GLib libraries from Homebrew and others from
MacPorts, two GLib copies end up in one process and GTK crashes at startup (`type
'(null)'`, `handle > 0`).

**Decision.** The development setup uses **MacPorts only** for the GTK stack. Don't put
Homebrew's `pkgconfig` folders in `PKG_CONFIG_PATH`; verify with `otool -L`. See
[development.md](development.md#macos-one-glib-only).

## Open questions

- **Saving settings.** GSettings (GNOME standard, needs a schema install) or a small config
  file in the user's config folder.
- **Default receive code** is compiled in. A per-install random code would be safer, but
  only makes sense once settings are saved.
- **`meson.build` minimum versions** (GTK ≥ 4.14, libadwaita ≥ 1.6) are below what the
  feature flags need (4.22 / 1.9).
