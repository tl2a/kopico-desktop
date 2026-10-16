# GTK4 + Libadwaita + Blueprint — Rust Patterns Reference

A focused reference for AI assistants working on this project.
Covers real patterns used in `src/main.rs` and `data/ui/*.blp`.

---

## Table of Contents

1. [Blueprint Markup](#1-blueprint-markup)
2. [GResource + Embedding](#2-gresource--embedding)
3. [Widget Loading in Rust](#3-widget-loading-in-rust)
4. [Signal Handlers](#4-signal-handlers)
5. [Property Bindings](#5-property-bindings)
6. [Adwaita Widget Cookbook](#6-adwaita-widget-cookbook)
7. [Responsive Layouts & Breakpoints](#7-responsive-layouts--breakpoints)
8. [CSS Styling](#8-css-styling)
9. [Navigation Patterns](#9-navigation-patterns)
10. [Dialogs & Overlays](#10-dialogs--overlays)
11. [GObject Properties & Values](#11-gobject-properties--values)
12. [Debugging Tips](#12-debugging-tips)
13. [Background Apps & System Tray](#13-background-apps--system-tray)
14. [Background Threads → GTK](#14-background-threads--gtk)

---

## 1. Blueprint Markup

### File Header
```blueprint
using Gtk 4.0;
using Adw 1;
```
Both `using` declarations are required in every `.blp` file that uses Adwaita widgets.

### Object IDs
Only widgets you need to access from Rust need an `id`:
```blueprint
Gtk.Button submit_btn {      // id = "submit_btn"
  label: _("Submit");
}
Gtk.Label {                  // no id — anonymous, Rust doesn't need it
  label: _("Static text");
}
```

### Child Type Annotations
Used for container slots that accept typed children:
```blueprint
Adw.ToolbarView {
  [top]              // top bar slot
  Adw.HeaderBar { }

  [bottom]           // bottom bar slot
  Gtk.ActionBar { }

  content: Gtk.ScrolledWindow { };   // main content slot
}

Adw.HeaderBar {
  [start]            // left of title
  Gtk.Button { }

  [end]              // right of title
  Gtk.MenuButton { }

  title-widget: Adw.WindowTitle { }; // replaces title text
}
```

### Translatable Strings
Wrap **every** user-visible string with `_()`:
```blueprint
label: _("Hello, World!");
tooltip-text: _("Click to submit");
placeholder-text: _("Search…");
```

### CSS Classes
```blueprint
Gtk.Button {
  styles ["suggested-action", "pill"]   // multiple classes, comma-separated
}
Gtk.ListBox {
  styles ["navigation-sidebar"]
}
```

### Signals in Blueprint (not used in this project)
Blueprint can wire signals to template methods — but since we use `gtk::Builder`
(not `CompositeTemplate`), we connect signals in Rust code instead. Do not add
`Gtk.Button { clicked => $on_click(); }` in the `.blp`.

### Enums
Blueprint accepts enum *nicknames* (lowercase, hyphenated):
```blueprint
halign: center;               // Gtk.Align.CENTER
valign: fill;                 // Gtk.Align.FILL
selection-mode: single;       // Gtk.SelectionMode.SINGLE
orientation: vertical;        // Gtk.Orientation.VERTICAL
```

---

## 2. GResource + Embedding

### Manifest (`data/*.gresource.xml`)
```xml
<?xml version="1.0" encoding="UTF-8"?>
<gresources>
  <gresource prefix="/in/tl2a/kopico-desktop">
    <file preprocess="xml-stripblanks">ui/window.ui</file>
    <file preprocess="xml-stripblanks">ui/preferences.ui</file>
    <!-- other file types (CSS, icons) don't need preprocess -->
    <file>style.css</file>
  </gresource>
</gresources>
```

> `preprocess="xml-stripblanks"` removes whitespace from XML at bundle time,
> reducing binary size. Always use it for `.ui` files.

### Embedding in Rust (`build.rs` → `main.rs`)
`build.rs` sets the env var:
```rust
println!("cargo:rustc-env=GRESOURCE_FILE={}", gresource_out.display());
```

`main.rs` embeds and registers:
```rust
let bytes = include_bytes!(env!("GRESOURCE_FILE"));
let resource = gio::Resource::from_data(&gtk4::glib::Bytes::from_static(bytes))
    .expect("gresource data");
gio::resources_register(&resource);
```

This must happen **before** `Application::new()` / `app.run()`.

### Resource Paths
The resource path is: `prefix` + `/` + `file-path-in-manifest`

For this project: `/in/tl2a/kopico-desktop/ui/window.ui`

---

## 3. Widget Loading in Rust

### Basic Pattern
```rust
let builder = gtk4::Builder::from_resource("/in/tl2a/kopico-desktop/ui/window.ui");

// Type annotation is required — Rust cannot infer it
let window: adw::ApplicationWindow = builder.object("window").expect("window");
let button: adw::gtk::Button       = builder.object("button").expect("button");
let entry:  adw::gtk::Entry        = builder.object("search_entry").expect("search_entry");
```

### Type Lookup Table

| Blueprint class | Rust type |
|---|---|
| `Adw.ApplicationWindow` | `adw::ApplicationWindow` |
| `Adw.ToolbarView` | `adw::ToolbarView` |
| `Adw.HeaderBar` | `adw::HeaderBar` |
| `Adw.WindowTitle` | `adw::WindowTitle` |
| `Adw.OverlaySplitView` | `adw::OverlaySplitView` |
| `Adw.NavigationView` | `adw::NavigationView` |
| `Adw.NavigationPage` | `adw::NavigationPage` |
| `Adw.StatusPage` | `adw::StatusPage` |
| `Adw.ToastOverlay` | `adw::ToastOverlay` |
| `Adw.PreferencesPage` | `adw::PreferencesPage` |
| `Adw.PreferencesGroup` | `adw::PreferencesGroup` |
| `Adw.ActionRow` | `adw::ActionRow` |
| `Adw.SwitchRow` | `adw::SwitchRow` |
| `Adw.ComboRow` | `adw::ComboRow` |
| `Adw.ExpanderRow` | `adw::ExpanderRow` |
| `Adw.Banner` | `adw::Banner` |
| `Gtk.Button` | `adw::gtk::Button` |
| `Gtk.ToggleButton` | `adw::gtk::ToggleButton` |
| `Gtk.Label` | `adw::gtk::Label` |
| `Gtk.Entry` | `adw::gtk::Entry` |
| `Gtk.ListBox` | `adw::gtk::ListBox` |
| `Gtk.ListBoxRow` | `adw::gtk::ListBoxRow` |
| `Gtk.ScrolledWindow` | `adw::gtk::ScrolledWindow` |
| `Gtk.Stack` | `adw::gtk::Stack` |
| `Gtk.Box` | `adw::gtk::Box` |

---

## 4. Signal Handlers

### Button Click
```rust
button.connect_clicked(move |_btn| {
    println!("clicked");
});
```

### Entry Activation (Enter key)
```rust
entry.connect_activate(move |entry| {
    let text = entry.text();
    println!("Submitted: {text}");
});
```

### ListBox Row Selected
```rust
list_box.connect_row_selected(move |_list, row| {
    let Some(row) = row else { return };  // row is None when deselected
    let index = row.index();
    println!("Selected row {index}");
});
```

### Window Close Request (intercept)
```rust
window.connect_close_request(move |_win| {
    // return true to prevent closing, false to allow
    glib::Propagation::Stop   // blocks close
    // glib::Propagation::Proceed  // allows close
});
```
To hide to the tray instead of closing, see [§13](#13-background-apps--system-tray).

### Clone Pattern for Multiple Captures
```rust
{
    let window = window.clone();
    let toast_overlay = toast_overlay.clone();
    button.connect_clicked(move |_| {
        window.set_title(Some("Changed"));
        toast_overlay.add_toast(adw::Toast::new("Done!"));
    });
}
// Scoped block drops the temporary clones once signals are wired
```

---

## 5. Property Bindings

### One-Way Binding
```rust
// source_widget.property → target_widget.property
source.bind_property("visible", &target, "sensitive")
    .sync_create()   // apply current value immediately
    .build();
```

### Bidirectional Binding
```rust
split_view
    .bind_property("show-sidebar", &toggle_button, "active")
    .bidirectional()
    .sync_create()
    .build();
```

### Binding with Transform
```rust
source.bind_property("value", &label, "label")
    .transform_to(|_, val: i32| Some(format!("{val}%")))
    .sync_create()
    .build();
```

---

## 6. Adwaita Widget Cookbook

### Toast Notification
```rust
// Simple
toast_overlay.add_toast(adw::Toast::new("Saved!"));

// With undo action
let toast = adw::Toast::builder()
    .title("Item deleted")
    .button_label("Undo")
    .timeout(5)
    .build();
toast.connect_button_clicked(move |_| { /* undo logic */ });
toast_overlay.add_toast(toast);
```

### About Dialog
```rust
let dialog = adw::AboutDialog::builder()
    .application_name("Kopico")
    .application_icon("in.tl2a.kopico-desktop")
    .version(env!("CARGO_PKG_VERSION"))
    .developer_name("Your Name")
    .license_type(gtk4::License::Gpl30)
    .website("https://example.com")
    .build();
dialog.present(Some(&window));
```

### Alert Dialog (async)
```rust
let dialog = adw::AlertDialog::builder()
    .heading("Delete item?")
    .body("This action cannot be undone.")
    .build();
dialog.add_response("cancel", "Cancel");
dialog.add_response("delete", "Delete");
dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
dialog.set_default_response(Some("cancel"));
dialog.set_close_response("cancel");

dialog.connect_response(None, move |_, response| {
    if response == "delete" {
        // perform deletion
    }
});
dialog.present(Some(&window));
```

### Action Row (settings list)
```blueprint
Adw.PreferencesGroup {
  title: _("Appearance");

  Adw.SwitchRow dark_mode_row {
    title: _("Dark Mode");
    subtitle: _("Force dark theme");
  }

  Adw.ActionRow {
    title: _("Accent Colour");
    activatable-widget: accent_combo;
    [suffix]
    Gtk.DropDown accent_combo { }
  }
}
```

### Banner
```blueprint
Adw.Banner banner {
  title: _("A new version is available.");
  button-label: _("Update");
  revealed: false;   // show/hide from Rust: banner.set_revealed(true)
}
```

---

## 7. Responsive Layouts & Breakpoints

### Blueprint Declaration (preferred)
```blueprint
Adw.ApplicationWindow window {
  Adw.Breakpoint {
    condition ("max-width: 600sp")
    setters {
      split_view.collapsed: true;
      header_bar.show-title: false;
    }
  }
  // …
}
```

### Rust-Only Declaration (when Blueprint can't express it)
```rust
use adw::{Breakpoint, BreakpointCondition};

let bp = Breakpoint::new(BreakpointCondition::parse("max-width: 600sp").unwrap());
bp.add_setter(&split_view, "collapsed", Some(&true.to_value()));
bp.connect_apply(move |_| { /* on narrow */ });
bp.connect_unapply(move |_| { /* on wide */ });
window.add_breakpoint(bp);
```

### Squeezer (deprecated — use Breakpoints instead)
`Adw.Squeezer` is available but Breakpoints are the modern approach.

---

## 8. CSS Styling

### Predefined CSS Classes (Adwaita)

| Class | Effect |
|---|---|
| `suggested-action` | Blue/accent primary button |
| `destructive-action` | Red destructive button |
| `pill` | Fully rounded button |
| `flat` | Flat/frameless button |
| `circular` | Circular icon button |
| `navigation-sidebar` | Sidebar ListBox styling |
| `boxed-list` | Rounded card-style ListBox |
| `card` | Card background |
| `dim-label` | Secondary/muted text colour |
| `title-1` … `title-4` | Heading sizes |
| `body` | Body text |
| `caption` | Small caption text |
| `monospace` | Monospace font |
| `numeric` | Tabular/numeric font features |
| `warning` | Yellow warning colour |
| `error` | Red error colour |
| `success` | Green success colour |

### Loading a Custom CSS File
**This project:** nothing to do — `AdwApplication` auto-loads `style.css` from the app's
resource base path (`/in/tl2a/kopico-desktop/style.css`, source `data/style.css`).
Manual loading is only needed for extra stylesheets:
```rust
// In main(), after registering resources:
let provider = gtk4::CssProvider::new();
provider.load_from_resource("/in/tl2a/kopico-desktop/style.css");
gtk4::style_context_add_provider_for_display(
    &gdk::Display::default().unwrap(),
    &provider,
    gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
);
```

---

## 9. Navigation Patterns

### NavigationView (push/pop stack)
```blueprint
Adw.NavigationView nav {
  Adw.NavigationPage {
    tag: "home";
    title: _("Home");
    child: Adw.ToolbarView { … };
  }
}
```
```rust
// Push a new page
let detail_page = adw::NavigationPage::builder()
    .tag("detail")
    .title("Detail")
    .child(&detail_view)
    .build();
nav.push(&detail_page);

// Push by tag (page must already be in the view)
nav.push_by_tag("settings");

// Pop back
nav.pop();
```

### OverlaySplitView (sidebar + content)
```rust
// Programmatically toggle sidebar
split_view.set_show_sidebar(!split_view.shows_sidebar());

// Check if currently collapsed (narrow mode)
if split_view.is_collapsed() {
    split_view.set_show_sidebar(false); // auto-hide after selection
}
```

---

## 10. Dialogs & Overlays

### FileDialog (async, GTK4.10+)
```rust
let dialog = gtk4::FileDialog::builder()
    .title("Open File")
    .build();

dialog.open(Some(&window), gio::Cancellable::NONE, move |result| {
    if let Ok(file) = result {
        println!("Selected: {}", file.path().unwrap().display());
    }
});
```

### ColorDialog (GTK4.10+)
```rust
let dialog = gtk4::ColorDialog::builder().with_alpha(false).build();
dialog.choose_rgba(Some(&window), None, gio::Cancellable::NONE, move |result| {
    if let Ok(color) = result {
        println!("r={} g={} b={}", color.red(), color.green(), color.blue());
    }
});
```

---

## 11. GObject Properties & Values

### Setting a Value on a Breakpoint Setter
```rust
use gtk4::glib::value::ToValue;

breakpoint.add_setter(&widget, "property-name", Some(&true.to_value()));
breakpoint.add_setter(&widget, "width-request", Some(&400i32.to_value()));
breakpoint.add_setter(&widget, "label", Some(&"Narrow".to_value()));
```

### Reading a Property Dynamically
```rust
let val: bool = widget.property("visible");
let title: String = window_title.property("title");
```

---

## 12. Debugging Tips

### Print Widget Tree
```bash
GTK_DEBUG=interactive cargo run
```
Launches the GTK Inspector — inspect the live widget tree, CSS, and properties.

### Blueprint Compile Errors
```bash
blueprint-compiler compile --output /dev/null data/ui/window.blp
```
Shows syntax errors with line numbers before doing a full build.

### GResource Contents
```bash
gresource list target/debug/build/kopico-desktop-*/out/in.tl2a.kopico-desktop.gresource
```

### Runtime Warnings
Set `G_MESSAGES_DEBUG=all` to see GLib/GTK deprecation and binding warnings:
```bash
G_MESSAGES_DEBUG=all cargo run
```

### Adwaita Version Check
```bash
pkg-config --modversion libadwaita-1
```
The installed version must be **at least** the `v1_x` feature flag in `Cargo.toml`
(e.g. `v1_9` needs libadwaita ≥ 1.9), otherwise the build fails.

---

## 13. Background Apps & System Tray

Implemented in `src/tray.rs` + `setup_app()` in `src/main.rs`.

### Keep Running With No Visible Window
```rust
// Hide instead of destroy — window state survives, app keeps running
window.connect_close_request(|window| {
    window.set_visible(false);
    glib::Propagation::Stop
});

// In connect_startup: keep the process alive even with zero windows.
// Dropping the guard (on shutdown) releases it.
let hold_guard = app.hold();
```

### Reuse One Window on Activate
```rust
fn show_window(app: &Application) {
    let window = app.windows().into_iter().next()
        .unwrap_or_else(|| build_ui(app).upcast());
    window.present();
}
app.connect_activate(show_window);
```

### Stateful Action (checkbox / toggle)
```rust
let action = gio::SimpleAction::new_stateful("enabled", None, &true.to_variant());
action.connect_activate(|action, _| {
    let on = !action.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
    action.set_state(&on.to_variant());
});
app.add_action(&action);   // usable as "app.enabled" from menus, buttons, Blueprint
```

### Tray Menu → App Actions (thread-safe)
```rust
MenuEvent::set_event_handler(Some(|event: MenuEvent| {
    let action = event.id.0;                    // String — Send
    glib::idle_add_once(move || {               // runs on the GTK main loop
        if let Some(app) = gio::Application::default() {
            app.activate_action(&action, None);
        }
    });
}));
```
Never capture widgets in the handler: on Linux (ksni) it runs on a worker thread.

### Making the Tray Optional
Keep the tray behind a stateful action (`app.tray`) and hold it in
`Rc<RefCell<Option<Tray>>>`: `Some(Tray::new(..))` to show, `None` to remove (this also
drops the hold guard). The close handler must check the same state — hide only while the
tray exists, otherwise return `Propagation::Proceed` so the app can exit.

### Quitting for Real
`app.quit()` stops the main loop → `connect_shutdown` fires → drop the `TrayIcon` and
hold guard there → `app.run()` returns and the process exits. No background instance remains.

### Platform Gotchas
| Platform | Note |
|---|---|
| Linux | Use `tray-icon` with `default-features = false, features = ["ksni"]` — the default backend loads GTK 3 and crashes a GTK 4 process |
| GNOME | Needs the *AppIndicator and KStatusNotifierItem Support* extension to show tray icons |
| macOS | Create the tray on the main thread (startup). Template icons follow menu bar light/dark but are always monochrome — use `set_icon` (non-template) for a coloured state such as green = enabled |
| macOS notifications | `gio::Notification` has no GLib macOS backend — `notification.rs` uses `notify-rust` (UNUserNotificationCenter, async on the GTK loop); the app must run from a signed `.app` (`cargo run` does this via `scripts/macos-app.sh`) |



---

## 14. Background Threads → GTK

GTK objects may only be touched on the main thread. Send plain data from a worker thread
over `async-channel` and handle it in a local future — it wakes only when a message
arrives (used by `src/receiver.rs`):

```rust
let (tx, rx) = async_channel::unbounded::<Event>();

std::thread::spawn(move || {
    // … blocking work …
    let _ = tx.send_blocking(Event::Done(result));   // Event must be Send
});

glib::spawn_future_local(async move {               // main thread; may capture widgets
    while let Ok(event) = rx.recv().await {
        toast_overlay.add_toast(adw::Toast::new("Done"));
    }
});                                                  // ends when all senders are dropped
```

Don't poll a `std::sync::mpsc` receiver from `glib::idle_add` — an idle source that
returns `ControlFlow::Continue` runs continuously and pins a CPU core.
`glib::MainContext::channel` no longer exists in current gtk-rs.
