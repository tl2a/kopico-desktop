use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use adw::{
    ActionRow, Application, ApplicationWindow, ComboRow, EntryRow, PasswordEntryRow, SwitchRow,
    Toast, ToastOverlay,
};
use adw::gtk::Label;
use gtk4::{gio, glib};

mod notification;
mod receiver;
mod tray;

use receiver::{ReceiverEvent, ReceiverHandle, RelayMode};
use tray::{ACTION_ENABLED, ACTION_QUIT, ACTION_SHOW_WINDOW, ACTION_TRAY, Tray};

const APP_ID: &str = "in.tl2a.kopico-desktop";

/// Default receive code.  Both sender and receiver must agree on this value.
/// Users can override it in Advanced → Receive Code.
const DEFAULT_RECEIVE_CODE: &str = "my-code-phase";
/// croc rejects shorter codes.
const MIN_CODE_LEN: usize = 6;

// Receiver settings — stateful string actions (`app.<name>`), see setup_app()
const ACTION_RELAY_MODE: &str = "relay-mode";
const ACTION_RELAY_ADDRESS: &str = "relay-address";
const ACTION_RELAY_PASSWORD: &str = "relay-password";
const ACTION_RECEIVE_CODE: &str = "receive-code";
/// Optional, Local mode only: `--ip` for when multicast discovery doesn't work.
const ACTION_SENDER_IP: &str = "sender-ip";

/// `app.relay-mode` values, in the same order as the Relay Mode combo row.
const RELAY_MODES: [&str; 3] = ["local", "own-relay", "public-relay"];

fn main() -> gtk4::glib::ExitCode {
    let bytes = include_bytes!(env!("GRESOURCE_FILE"));
    let resource = gio::Resource::from_data(&gtk4::glib::Bytes::from_static(bytes))
        .expect("Failed to load gresource data");
    gio::resources_register(&resource);

    let app = Application::builder().application_id(APP_ID).build();
    app.connect_startup(setup_app);
    app.connect_activate(show_window);
    app.run()
}

/// Current value of a boolean stateful action.
fn action_bool(group: &impl IsA<gio::ActionGroup>, name: &str) -> bool {
    group
        .action_state(name)
        .and_then(|state| state.get::<bool>())
        .unwrap_or(false)
}

/// Current value of a string stateful action.
fn action_string(group: &impl IsA<gio::ActionGroup>, name: &str) -> String {
    group
        .action_state(name)
        .and_then(|state| state.get::<String>())
        .unwrap_or_default()
}

/// Receiver configuration from the `app.relay-*` / `app.receive-code` action states.
fn receiver_config(app: &Application) -> Result<(RelayMode, String), String> {
    let mode = match action_string(app, ACTION_RELAY_MODE).as_str() {
        "own-relay" => {
            let address = action_string(app, ACTION_RELAY_ADDRESS);
            if address.is_empty() {
                return Err("Enter your relay address to start receiving".into());
            }
            RelayMode::OwnRelay { address, password: action_string(app, ACTION_RELAY_PASSWORD) }
        }
        "public-relay" => RelayMode::PublicRelay,
        _ => {
            let ip = action_string(app, ACTION_SENDER_IP);
            RelayMode::Local { sender_ip: (!ip.is_empty()).then_some(ip) }
        }
    };
    Ok((mode, action_string(app, ACTION_RECEIVE_CODE)))
}

// ── Receiver state ─────────────────────────────────────────────────────────────

/// The croc receive loop; runs while `app.enabled` is on. Its configuration comes
/// from the receiver setting actions, so (re)starting always picks up the latest.
#[derive(Default)]
struct ReceiverState {
    handle: Option<ReceiverHandle>,
}

impl ReceiverState {
    fn start(&mut self, app: &Application) {
        if self.handle.is_some() {
            return;
        }
        let (mode, code) = match receiver_config(app) {
            Ok(config) => config,
            Err(message) => {
                println!("[receiver] not started: {message}");
                show_toast(app, &message);
                return;
            }
        };
        let out_dir = receiver::default_out_dir();
        println!("[receiver] started ({mode:?}, saving to {})", out_dir.display());

        let app_weak = app.downgrade();
        let handle = ReceiverHandle::start(mode, out_dir.clone(), code, move |event| {
            // Runs on the GTK main thread
            let Some(app) = app_weak.upgrade() else { return };
            match event {
                ReceiverEvent::Received(names) => {
                    let message = received_message(&names, &out_dir);
                    notification::send_files_received(&app, &message);
                    show_toast(&app, &message);
                }
                ReceiverEvent::Text(text) => {
                    copy_to_clipboard(&text);
                    let message = "Received text copied to clipboard";
                    notification::send_text_received(&app, message);
                    show_toast(&app, message);
                }
                ReceiverEvent::Failed(message) => show_toast(&app, &message),
            }
        });
        self.handle = Some(handle);
    }

    fn stop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.stop();
            println!("[receiver] stopped");
        }
    }

    /// Apply new settings — only if the loop is running (Kopico enabled).
    fn restart(&mut self, app: &Application) {
        if self.handle.is_some() || action_bool(app, ACTION_ENABLED) {
            self.stop();
            self.start(app);
        }
    }
}

/// "“photo.jpg” saved to ~/Downloads/Kopico" / "3 items saved to …".
fn received_message(names: &[String], out_dir: &std::path::Path) -> String {
    let dir = receiver::display_path(out_dir);
    match names {
        [] => format!("Files saved to {dir}"),
        [name] => format!("\u{201c}{name}\u{201d} saved to {dir}"),
        _ => format!("{} items saved to {dir}", names.len()),
    }
}

/// Put `text` on the system clipboard (paste anywhere with Cmd/Ctrl+V).
///
/// macOS: written to NSPasteboard directly (stays after Kopico quits) — GTK's macOS
/// clipboard backend rejects its own MIME types and copies nothing.
#[cfg(target_os = "macos")]
fn copy_to_clipboard(text: &str) {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::NSString;

    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    // SAFETY: reading an immutable AppKit constant
    let string_type = unsafe { NSPasteboardTypeString };
    if !pasteboard.setString_forType(&NSString::from_str(text), string_type) {
        eprintln!("[receiver] could not copy the received text to the clipboard");
    }
}

/// Linux/Wayland: use wl-copy's data-control support so copying works while unfocused
/// when the compositor supports it. Fall back to GDK when wl-copy is unavailable.
#[cfg(target_os = "linux")]
fn copy_to_clipboard(text: &str) {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        match copy_with_wl_copy(text) {
            Ok(()) => return,
            Err(err) => eprintln!("[receiver] wl-copy failed ({err}); falling back to GDK"),
        }
    }
    copy_with_gdk(text);
}

/// BSD and other non-macOS platforms: GDK clipboard.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn copy_to_clipboard(text: &str) {
    copy_with_gdk(text);
}

#[cfg(not(target_os = "macos"))]
fn copy_with_gdk(text: &str) {
    match gtk4::gdk::Display::default() {
        Some(display) => display.clipboard().set_text(text),
        None => eprintln!("[receiver] no display — text not copied to the clipboard"),
    }
}

#[cfg(target_os = "linux")]
fn copy_with_wl_copy(text: &str) -> std::io::Result<()> {
    use std::{
        io::{self, Write},
        process::{Command, Stdio},
    };

    let mut child = Command::new("wl-copy").stdin(Stdio::piped()).spawn()?;
    let mut stdin = child.stdin.take().expect("wl-copy stdin is piped");
    let write_result = stdin.write_all(text.as_bytes());
    drop(stdin);
    let status = child.wait()?;
    write_result?;
    if !status.success() {
        return Err(io::Error::other(format!("exited with {status}")));
    }
    Ok(())
}

/// Toast on the main window (if it exists — it may be hidden in the tray).
fn show_toast(app: &Application, message: &str) {
    for window in app.windows() {
        if let Some(window) = window.downcast_ref::<ApplicationWindow>() {
            show_toast_on_window(window, message);
        }
    }
}

/// Walk the widget tree to find the first `ToastOverlay` and add a toast to it.
fn show_toast_on_window(window: &ApplicationWindow, message: &str) {
    fn find_overlay(widget: &adw::gtk::Widget) -> Option<ToastOverlay> {
        if let Some(o) = widget.downcast_ref::<ToastOverlay>() {
            return Some(o.clone());
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            if let Some(found) = find_overlay(&c) {
                return Some(found);
            }
            child = c.next_sibling();
        }
        None
    }

    if let Some(overlay) = find_overlay(window.upcast_ref()) {
        overlay.add_toast(Toast::new(message));
    }
}

// ── App setup ──────────────────────────────────────────────────────────────────

/// Runs once per process on the primary instance: registers actions, tray, receiver.
fn setup_app(app: &Application) {
    let tray: Rc<RefCell<Option<Tray>>> = Rc::new(RefCell::new(None));
    let receiver: Rc<RefCell<ReceiverState>> = Rc::default();
    tray::install_menu_handler();
    notification::init(); // macOS: permission prompt on first run

    // ── app.enabled — power toggle + tray checkbox ────────────────────────────
    let enabled_action = gio::SimpleAction::new_stateful(ACTION_ENABLED, None, &true.to_variant());
    {
        let tray = tray.clone();
        let receiver = receiver.clone();
        let app_notif = app.downgrade();
        let app_recv = app.downgrade();

        enabled_action.connect_change_state(move |action, value| {
            let Some(enabled) = value.and_then(|v| v.get::<bool>()) else { return };
            let was_enabled = action.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
            action.set_state(&enabled.to_variant());

            if let Some(t) = tray.borrow().as_ref() {
                t.set_enabled(enabled);
            }
            println!("Kopico {}", if enabled { "enabled" } else { "disabled" });

            // Start / stop the croc receive loop
            if let Some(app) = app_recv.upgrade() {
                if enabled {
                    receiver.borrow_mut().start(&app);
                } else {
                    receiver.borrow_mut().stop();
                }
            }

            // System notification on actual off→on transition only
            if enabled && !was_enabled {
                if let Some(app) = app_notif.upgrade() {
                    notification::send_enabled(&app);
                }
            }
        });
    }
    app.add_action(&enabled_action);

    // ── app.tray ──────────────────────────────────────────────────────────────
    let tray_action = gio::SimpleAction::new_stateful(ACTION_TRAY, None, &true.to_variant());
    {
        let tray = tray.clone();
        let app_w = app.downgrade();
        tray_action.connect_change_state(move |action, value| {
            let (Some(show), Some(app)) = (value.and_then(|v| v.get::<bool>()), app_w.upgrade())
            else { return };
            action.set_state(&show.to_variant());
            let mut t = tray.borrow_mut();
            match (show, t.is_some()) {
                (true, false) => *t = Some(Tray::new(&app, action_bool(&app, ACTION_ENABLED))),
                (false, true) => *t = None,
                _ => {}
            }
            println!("System tray {}", if show { "enabled" } else { "disabled" });
        });
    }
    app.add_action(&tray_action);
    *tray.borrow_mut() = Some(Tray::new(app, action_bool(app, ACTION_ENABLED)));

    // ── app.show-window ───────────────────────────────────────────────────────
    let show_action = gio::SimpleAction::new(ACTION_SHOW_WINDOW, None);
    {
        let app_w = app.downgrade();
        show_action.connect_activate(move |_, _| {
            if let Some(app) = app_w.upgrade() { show_window(&app); }
        });
    }
    app.add_action(&show_action);

    // ── app.quit ──────────────────────────────────────────────────────────────
    let quit_action = gio::SimpleAction::new(ACTION_QUIT, None);
    {
        let app_w = app.downgrade();
        quit_action.connect_activate(move |_, _| {
            if let Some(app) = app_w.upgrade() { app.quit(); }
        });
    }
    app.add_action(&quit_action);
    app.set_accels_for_action("app.quit", &["<primary>q"]);

    // ── Receiver settings: app.relay-mode / relay-address / relay-password / receive-code ──
    // Stateful string actions: the Advanced page reads them for its initial values and
    // changes them with change_action_state(); each accepted change restarts the loop.
    let settings: [(&str, &str, fn(&str) -> bool); 5] = [
        (ACTION_RELAY_MODE, RELAY_MODES[0], |v| RELAY_MODES.contains(&v)),
        (ACTION_RELAY_ADDRESS, "", |v| v.is_empty() || v.contains(':')),
        (ACTION_RELAY_PASSWORD, "", |_| true),
        (ACTION_RECEIVE_CODE, DEFAULT_RECEIVE_CODE, |v| v.chars().count() >= MIN_CODE_LEN),
        (ACTION_SENDER_IP, "", |v| v.is_empty() || receiver::is_valid_sender_ip(v)),
    ];
    for (name, default, is_valid) in settings {
        let action = gio::SimpleAction::new_stateful(name, None, &default.to_variant());
        let receiver = receiver.clone();
        let app_w = app.downgrade();
        action.connect_change_state(move |action, value| {
            let (Some(value), Some(app)) = (value.and_then(|v| v.get::<String>()), app_w.upgrade())
            else { return };
            let value = value.trim().to_owned();
            if !is_valid(&value) || action.state().and_then(|s| s.get::<String>()) == Some(value.clone()) {
                return;
            }
            action.set_state(&value.to_variant());
            receiver.borrow_mut().restart(&app);
        });
        app.add_action(&action);
    }

    // Kopico starts enabled → start receiving right away (Local mode by default)
    if action_bool(app, ACTION_ENABLED) {
        receiver.borrow_mut().start(app);
    }

    app.connect_shutdown(move |_| {
        tray.borrow_mut().take();
        receiver.borrow_mut().stop();
    });
}

// ── Window ─────────────────────────────────────────────────────────────────────

fn show_window(app: &Application) {
    let window = app
        .windows()
        .into_iter()
        .next()
        .unwrap_or_else(|| build_ui(app).upcast());
    window.present();
}

fn build_ui(app: &Application) -> ApplicationWindow {
    let builder = gtk4::Builder::from_resource("/in/tl2a/kopico-desktop/ui/window.ui");

    let window: ApplicationWindow   = builder.object("window").expect("window");
    let state_title: Label          = builder.object("state_title").expect("state_title");
    let state_description: Label    = builder.object("state_description").expect("state_description");
    let tray_row: SwitchRow         = builder.object("tray_row").expect("tray_row");
    let relay_mode_row: ComboRow    = builder.object("relay_mode_row").expect("relay_mode_row");
    let relay_address_row: EntryRow = builder.object("relay_address_row").expect("relay_address_row");
    let relay_password_row: PasswordEntryRow =
        builder.object("relay_password_row").expect("relay_password_row");
    let receive_code_row: EntryRow  = builder.object("receive_code_row").expect("receive_code_row");
    let sender_ip_row: EntryRow     = builder.object("sender_ip_row").expect("sender_ip_row");
    let save_to_row: ActionRow      = builder.object("save_to_row").expect("save_to_row");
    let toast_overlay: ToastOverlay = builder.object("toast_overlay").expect("toast_overlay");

    window.set_application(Some(app));

    // ── Home: status text follows app.enabled ─────────────────────────────────
    let update_state_text = move |enabled: bool| {
        if enabled {
            state_title.set_label("Kopico Is On");
            state_description.set_label(
                "Receiving files in the background. Turn off anytime from here or the tray.",
            );
        } else {
            state_title.set_label("Kopico Is Off");
            state_description.set_label("Kopico is paused. Turn it on to start receiving.");
        }
    };
    update_state_text(action_bool(app, ACTION_ENABLED));
    if let Some(action) = app.lookup_action(ACTION_ENABLED) {
        action.connect_notify_local(Some("state"), move |action, _| {
            update_state_text(action.state().and_then(|s| s.get()).unwrap_or(false));
        });
    }

    // ── Advanced: receive settings ────────────────────────────────────────────
    save_to_row.set_subtitle(&receiver::display_path(&receiver::default_out_dir()));

    // Relay mode — combo index ↔ RELAY_MODES; each mode shows only its own rows
    let update_relay_rows = {
        let relay_address_row = relay_address_row.clone();
        let relay_password_row = relay_password_row.clone();
        let sender_ip_row = sender_ip_row.clone();
        move |row: &ComboRow| {
            let own_relay = row.selected() == 1;
            relay_address_row.set_visible(own_relay);
            relay_password_row.set_visible(own_relay);
            sender_ip_row.set_visible(row.selected() == 0);
            row.set_subtitle(match row.selected() {
                0 => "Devices on this network · checks every 3 s",
                1 => "Your own croc relay · checks every 10 s",
                _ => "croc\u{2019}s public relays · checks every 30 s",
            });
        }
    };
    let current_mode = action_string(app, ACTION_RELAY_MODE);
    let index = RELAY_MODES.iter().position(|m| *m == current_mode).unwrap_or(0);
    relay_mode_row.set_selected(index as u32);
    update_relay_rows(&relay_mode_row);
    {
        let app_w = app.downgrade();
        relay_mode_row.connect_selected_notify(move |row| {
            let Some(app) = app_w.upgrade() else { return };
            update_relay_rows(row);
            let mode = RELAY_MODES.get(row.selected() as usize).unwrap_or(&RELAY_MODES[0]);
            app.change_action_state(ACTION_RELAY_MODE, &mode.to_variant());
        });
    }

    // Text settings: apply button → validated action change; invalid input is marked
    let text_rows: [(&EntryRow, &str, &str); 4] = [
        (&sender_ip_row, ACTION_SENDER_IP, "Use an IP address or host name, e.g. 192.168.1.20 or 192.168.1.20:9009"),
        (&relay_address_row, ACTION_RELAY_ADDRESS, "Use host:port, e.g. myrelay.example.com:9009"),
        (relay_password_row.upcast_ref(), ACTION_RELAY_PASSWORD, ""),
        (&receive_code_row, ACTION_RECEIVE_CODE, "The code needs at least 6 characters"),
    ];
    for (row, action, invalid_message) in text_rows {
        row.set_text(&action_string(app, action));
        row.connect_changed(|row| row.remove_css_class("error"));
        let app_w = app.downgrade();
        let toast = toast_overlay.clone();
        let action = action.to_owned();
        let invalid_message = invalid_message.to_owned();
        row.connect_apply(move |row| {
            let Some(app) = app_w.upgrade() else { return };
            let value = row.text().trim().to_owned();
            app.change_action_state(&action, &value.to_variant());
            if action_string(&app, &action) == value {
                toast.add_toast(Toast::new("Saved"));
            } else {
                row.add_css_class("error");
                toast.add_toast(Toast::new(&invalid_message));
            }
        });
    }

    // ── Advanced: tray switch ─────────────────────────────────────────────────
    tray_row.set_active(action_bool(app, ACTION_TRAY));
    {
        let app_w = app.downgrade();
        let toast = toast_overlay.clone();
        tray_row.connect_active_notify(move |row| {
            let Some(app) = app_w.upgrade() else { return };
            let show = row.is_active();
            app.change_action_state(ACTION_TRAY, &show.to_variant());
            if !show {
                toast.add_toast(Toast::new("Closing the window will now quit Kopico"));
            }
        });
    }

    // ── Close → hide to tray or quit ─────────────────────────────────────────
    window.connect_close_request(|window| {
        let in_tray = window
            .application()
            .is_some_and(|app| action_bool(&app, ACTION_TRAY));
        if in_tray {
            window.set_visible(false);
            println!("Window hidden — Kopico is still running in the tray");
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });

    window
}
