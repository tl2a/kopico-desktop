# Kopico Desktop — Agent Instructions

> This file provides structured instructions for AI coding agents (OpenAI Codex,
> GitHub Copilot Workspace, etc.). For a deeper reference, see `CLAUDE.md`.

## Repository Purpose

A GNOME desktop application (GTK4 + libadwaita) written in Rust, using Blueprint
markup for UI and Meson as the primary build system.

---

## Key Rules for Agents

### 1. Never Edit Generated Files
- `*.ui` files under `$OUT_DIR` are **generated** from `.blp` sources — do not edit them.
- The `.gresource` binary is also generated. Only source files under `data/ui/*.blp` and
  `data/*.gresource.xml` should be modified.

### 2. UI Changes Go in `.blp` Files
- All widget layout, properties, and CSS classes belong in `data/ui/window.blp`
  (or new `.blp` files for additional windows/dialogs).
- Signal handlers stay in `src/main.rs` — Blueprint does not wire up Rust closures.

### 3. Validate Blueprint Before Committing
```bash
blueprint-compiler compile --output /dev/null data/ui/window.blp
```

### 4. Verify Build After Every Change
```bash
cargo build 2>&1 | tail -5
```

### 5. Widget ID Contract
Every widget accessed in Rust via `builder.object("id")` **must** have a matching
`id` attribute in the `.blp` file. Keep these in sync — mismatches panic at runtime.

### 6. Import Discipline
```rust
use adw::prelude::*;          // always — enables property setter methods
use adw::gtk::SomeWidget;     // GTK widgets via adw re-export
use adw::SomeAdwWidget;       // Adwaita-specific widgets
use gtk4::gio;                // for Resource / resources_register
```
Do **not** mix `gtk4::Widget` and `adw::gtk::Widget` for the same type.

---

## Adding Features — Checklist

### New Widget in Existing Window
- [ ] Add widget with an `id` in `data/ui/window.blp`
- [ ] Fetch it in `build_ui()` via `builder.object("the_id")`
- [ ] Wire signal handlers using the clone pattern

### New Window or Dialog
- [ ] Create `data/ui/my_window.blp`
- [ ] Register it in `data/in.tl2a.kopico-desktop.gresource.xml`
- [ ] Add the `.blp` path to `blueprints = files(…)` in `meson.build`
- [ ] Load with `gtk4::Builder::from_resource("/in/tl2a/kopico-desktop/ui/my_window.ui")`

### New Dependency
- [ ] Add to `[dependencies]` in `Cargo.toml`
- [ ] If it's a system library, add a `dependency(…)` call in `meson.build`
- [ ] Run `cargo tree --target all -i gtk` — it must find nothing. Any crate that pulls in
      GTK 3 (`gtk 0.18`) crashes the GTK 4 app on Linux.
- [ ] Don't add `gio`/`glib` crates directly — use `gtk4::gio` / `gtk4::glib`.

### Changing the croc Receiver
- [ ] Read “Receiving Files” in `CLAUDE.md` — the parser depends on croc's exact output
- [ ] `cargo test` (parser) and `cargo test -- --ignored --test-threads=1` (real croc)
- [ ] After a croc upgrade, re-check its stderr/stdout wording against those notes

### New Tray Menu Item
- [ ] Register an `app.<name>` action in `setup_app()` (`src/main.rs`)
- [ ] Add a menu item with id `"<name>"` in `Tray::new()` (`src/tray.rs`)
- [ ] Nothing else — the `MenuEvent` handler already dispatches ids to `app.*` actions

---

## File Map

| File | Purpose | Edit? |
|---|---|---|
| `README.md`, `docs/*.md` | Human docs — keep in sync with behaviour changes | ✅ Yes |
| `src/main.rs` | App entry, app actions, window + signal handlers | ✅ Yes |
| `src/receiver.rs` | croc receive loop (background thread, polling) | ✅ Yes — see CLAUDE.md “Receiving Files” first |
| `src/tray.rs` | System tray icon + menu → `app.*` actions | ✅ Yes |
| `src/notification.rs` | System notifications — gio on Linux, notify-rust on macOS | ✅ Yes |
| `dev-install.sh` | Registers the dev build for notifications (Linux `.desktop`, macOS `.app`) | ✅ Yes |
| `data/macos/Info.plist` | macOS bundle metadata (bundle id must equal `APP_ID`) | ✅ Yes |
| `scripts/macos-app.sh` | Builds, signs and registers `Kopico Desktop.app`; `cargo run` runner on macOS | ✅ Rarely |
| `.cargo/config.toml` | Sets that runner for macOS | ✅ Rarely |
| `data/style.css` | App CSS, auto-loaded by `AdwApplication` | ✅ Yes |
| `data/ui/window.blp` | Window UI layout | ✅ Yes |
| `data/*.gresource.xml` | GResource manifest | ✅ When adding `.ui` files |
| `meson.build` | Meson build config | ✅ When adding `.blp` files |
| `build.rs` | Cargo build script | ✅ Rarely |
| `Cargo.toml` | Rust dependencies | ✅ Yes |
| `Cargo.lock` | Locked dependency versions | ❌ Never manually |
| `builddir/` | Meson build output | ❌ Generated |

---

## Adwaita Widget Quick Reference

| Need | Widget |
|---|---|
| Top-level window | `Adw.ApplicationWindow` |
| Window with header bar | `Adw.ToolbarView` + `Adw.HeaderBar` |
| Responsive sidebar layout | `Adw.OverlaySplitView` |
| Top-level tabs | `Adw.ViewStack` + `Adw.ViewSwitcher` / `Adw.ViewSwitcherBar` |
| Multi-page navigation | `Adw.NavigationView` / `Adw.NavigationPage` |
| Empty / placeholder state | `Adw.StatusPage` |
| In-app notifications | `Adw.ToastOverlay` + `Adw.Toast` |
| Settings page | `Adw.PreferencesPage` + `Adw.PreferencesGroup` + `Adw.ActionRow` |
| Expandable row | `Adw.ExpanderRow` |
| Combo row | `Adw.ComboRow` |
| Switch row | `Adw.SwitchRow` |
| Spinner row | `Adw.SpinRow` |
| Banner (top notice) | `Adw.Banner` |
| About dialog | `Adw.AboutDialog` |
| Alert dialog | `Adw.AlertDialog` |

---

## Blueprint Syntax Quick Reference

```blueprint
using Gtk 4.0;
using Adw 1;

// Top-level object (no variable needed for root)
Adw.ApplicationWindow window {
  title: _("My App");          // _() marks translatable strings
  default-width: 800;

  // Inline breakpoint — no Rust code required
  Adw.Breakpoint {
    condition ("max-width: 600sp")
    setters { split_view.collapsed: true; }
  }

  content: Adw.ToolbarView {
    [top]                      // child type annotation
    Adw.HeaderBar {
      [start]
      Gtk.Button btn {         // id = "btn" → builder.object("btn")
        label: _("Click");
        styles ["suggested-action", "pill"]
      }
    }
  };
}
```

### Property value types
| Blueprint | Example |
|---|---|
| String | `label: _("Hello");` |
| Integer | `margin-top: 12;` |
| Boolean | `visible: true;` |
| Enum | `halign: center;` |
| Object ref | `title-widget: Adw.WindowTitle { … };` |

