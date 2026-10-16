# Architecture

How Kopico Desktop is put together. For the croc side in detail see
[receiver.md](receiver.md); for platform-specific choices see
[platform-decisions.md](platform-decisions.md).

## Overview

```
┌──────────────────────── GTK main thread ────────────────────────┐
│                                                                  │
│  AdwApplication ── app actions (the app's state) ──┬── window    │
│        │            enabled, tray, relay-mode, …   ├── tray menu │
│        │                                            └── shortcuts │
│        ├── Tray (tray.rs)            ── icon + menu              │
│        ├── ReceiverState (main.rs)   ── starts/stops ──┐         │
│        └── notification.rs, clipboard                   │         │
│                 ▲                                       │         │
│                 │ ReceiverEvent (async-channel)         ▼         │
└─────────────────┼──────────────────────────── croc-receive thread ┘
                  └──── receive_loop (receiver.rs) ── runs croc again and again
```

| File | Responsibility |
|---|---|
| `src/main.rs` | Startup, app actions, receiver lifecycle, window wiring, clipboard |
| `src/receiver.rs` | The croc loop on a background thread; parses croc's output |
| `src/tray.rs` | Tray / menu bar icon and menu |
| `src/notification.rs` | System notifications (Linux: gio, macOS: notify-rust) |
| `data/ui/window.blp` | The window (Blueprint) |

## App state lives in actions

Every piece of app-wide state is a **stateful `gio::SimpleAction`** registered on the
application in `setup_app()`:

| Action | Type | Default | Used by |
|---|---|---|---|
| `app.enabled` | bool | `true` | power toggle (Home), tray *Enable*; starts/stops the receiver |
| `app.tray` | bool | `true` | Advanced → *Show in System Tray* |
| `app.relay-mode` | string | `local` | `local` / `own-relay` / `public-relay` |
| `app.relay-address` | string | `""` | Own Relay, `host:port` |
| `app.relay-password` | string | `""` | Own Relay |
| `app.receive-code` | string | built-in default | croc code, ≥ 6 characters |
| `app.sender-ip` | string | `""` | Local mode, optional `host[:port]` |
| `app.show-window`, `app.quit` | — | | tray *Settings* / *Quit*, `Ctrl/Cmd+Q` |

Why actions:

- **One source of truth.** The power toggle is bound in Blueprint with
  `action-name: "app.enabled"`; the tray menu activates the same action; both always agree
  without extra code.
- **Validation in one place.** Every setting's `change_state` handler validates the value
  (e.g. code length) and ignores invalid ones; the UI marks the field with the `error`
  style when its value didn't stick.
- **Side effects in one place.** An accepted receiver setting restarts the receive loop;
  `app.enabled` also updates the tray icon and sends the "enabled" notification;
  `app.tray` creates or removes the tray.

The window reads the current action states when it is built, so it always shows the
current values.

## Lifecycle

- **startup** (once per process): tray, actions, notification permission (macOS), receiver
  start (Kopico starts enabled).
- **activate** (each launch / tray *Settings*): `show_window()` presents the one window,
  creating it on first use — never a second one.
- **close button**: hides the window while the tray is on (`Propagation::Stop`); with the
  tray off it closes normally and the app exits.
- **quit**: `app.quit()` → shutdown drops the tray (icon + hold guard) and stops the
  receiver (kills croc) → the process exits with nothing left running.

The tray keeps an `ApplicationHoldGuard`, which is what keeps the process alive while
every window is hidden.

## Threads and events

| Thread | What runs there |
|---|---|
| GTK main thread | everything touching widgets, actions, clipboard, notifications |
| `croc-receive` | `receive_loop`: spawn croc, watch it, wait for the next poll |
| two reader threads per croc run | drain croc's stdout and stderr (a full pipe would block croc) |
| tray menu thread (Linux `ksni`) | tray-icon delivers menu clicks here |

Crossing to the main thread:

- **Receiver → UI:** `ReceiverEvent` (`Received(names)`, `Text(text)`, `Failed(msg)`) over an
  `async-channel`, consumed by a `glib::spawn_future_local` future. It wakes only when an
  event arrives — no polling.
- **Tray → app:** the menu item id is the action name; the click handler forwards only that
  string with `glib::idle_add_once`, which then activates `app.<id>` on the main thread.
- **macOS notifications:** sent with notify-rust's async API from a future on the GTK loop.

Stopping the receiver sets an `AtomicBool`; the loop notices within 200 ms, kills croc and
exits, so turning Kopico off never blocks the UI.

## UI

One `Adw.ApplicationWindow` with a `ViewStack` of two tabs, switched from the header bar
(or a bottom bar on narrow windows):

- **Home:** the power toggle and a status line.
- **Advanced:** receive settings (relay mode, relay address/password, sender IP, receive
  code, save folder) and *Show in System Tray*. Rows that don't apply to the selected mode
  are hidden.

Text fields apply on Enter or ✓. Toasts confirm saves and report received transfers.
Coding patterns: [gtk4-libadwaita-patterns.md](gtk4-libadwaita-patterns.md).
