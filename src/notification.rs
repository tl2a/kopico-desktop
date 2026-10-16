//! System notifications.
//!
//! - **Linux/BSD:** `gio::Notification`, the GNOME-native API; clicking a notification
//!   runs `app.show-window`. GNOME Shell only shows them for an installed `.desktop` file
//!   (`dev-install.sh` installs one for development).
//! - **macOS:** `notify-rust` with Apple's `UNUserNotificationCenter` (feature
//!   `preview-macos-un`) — GLib has no macOS notification backend. Requires running from
//!   an `.app` bundle with Kopico's bundle id; [`init`] asks for permission, which also
//!   lists Kopico in System Settings → Notifications. Clicking one does nothing yet.
//!
//! Texts are never put in a notification — they can appear on the lock screen.

use adw::Application;
#[cfg(not(target_os = "macos"))]
use adw::prelude::*;
#[cfg(not(target_os = "macos"))]
use gtk4::gio;
#[cfg(target_os = "macos")]
use gtk4::glib;

/// Notification ids — re-using an id replaces the previous notification (Linux).
const ENABLED_ID: &str = "enabled";
const FILE_ID: &str = "file-received";

/// Tell the user Kopico was turned on (from the window or the tray).
pub fn send_enabled(app: &Application) {
    send(app, ENABLED_ID, "Kopico Enabled", "Kopico is running in the background.");
}

/// Tell the user a text message arrived and is on the clipboard.
pub fn send_text_received(app: &Application, message: &str) {
    send(app, FILE_ID, "Text Received", message);
}

/// Tell the user files arrived via croc. `message` is e.g. "“a.jpg” saved to ~/Downloads/Kopico".
pub fn send_files_received(app: &Application, message: &str) {
    // Same id for every transfer → a new one replaces the last instead of stacking
    send(app, FILE_ID, "Files Received", message);
}

#[cfg(not(target_os = "macos"))]
fn send(app: &Application, id: &str, title: &str, body: &str) {
    let notification = gio::Notification::new(title);
    notification.set_body(Some(body));
    notification.set_default_action(&format!("app.{}", crate::tray::ACTION_SHOW_WINDOW));
    app.send_notification(Some(id), &notification);
}

#[cfg(target_os = "macos")]
fn send(_app: &Application, _id: &str, title: &str, body: &str) {
    if !macos::available() {
        return;
    }
    let notification = notify_rust::Notification::new().summary(title).body(body).finalize();
    // Async on the GTK main loop: the blocking `show()` refuses to run unless the main
    // thread idles inside NSRunLoop ("Mainthread not running"), which GTK's GLib-based
    // loop never does. The request itself is sent from notify-rust's own worker thread.
    glib::spawn_future_local(async move {
        if let Err(err) = notification.show_async().await {
            eprintln!("[notification] failed to show notification: {err}");
        }
    });
}

/// macOS: ask for notification permission. Call once at startup (main thread, after GTK
/// is initialised). The first time, macOS shows its "Allow notifications?" prompt and
/// Kopico appears in System Settings → Notifications.
#[cfg(target_os = "macos")]
pub fn init() {
    if !macos::available() {
        return;
    }
    // Resolves once the user answers the prompt (or immediately if they already did)
    glib::spawn_future_local(async {
        match notify_rust::request_auth().await {
            Ok(true) => println!("[notification] notifications allowed"),
            Ok(false) => eprintln!(
                "[notification] notifications are off for Kopico Desktop — enable them in \
                 System Settings → Notifications"
            ),
            Err(err) => eprintln!("[notification] permission request failed: {err}"),
        }
    });
}

/// Linux: nothing to set up — permission is implied by the installed .desktop file.
#[cfg(not(target_os = "macos"))]
pub fn init() {}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::OnceLock;

    /// UNUserNotificationCenter only works for a process started from an `.app` bundle
    /// (it needs a bundle identifier). Plain `cargo run` has none — see
    /// scripts/macos-run.sh, which `.cargo/config.toml` uses to run from the bundle.
    pub fn available() -> bool {
        static AVAILABLE: OnceLock<bool> = OnceLock::new();
        *AVAILABLE.get_or_init(|| match notify_rust::check_bundle() {
            Ok(()) => true,
            Err(_) => {
                eprintln!(
                    "[notification] not running from an app bundle — macOS notifications \
                     are disabled. Start Kopico via `cargo run` (uses scripts/macos-run.sh) \
                     or from \"Kopico Desktop.app\"."
                );
                false
            }
        })
    }
}
