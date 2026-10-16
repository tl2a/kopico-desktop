#!/usr/bin/env bash
# dev-install.sh — registers the dev build with the desktop so that `cargo run` can show
# system notifications under Kopico's own name.
#
#   Linux: installs the .desktop file to ~/.local/share/applications/ (gio::Notification
#          needs it).
#   macOS: creates "~/Applications/Kopico Desktop.app" (signed bundle with the debug
#          binary) — UNUserNotificationCenter only works for a signed app bundle.
#          `cargo run` refreshes it automatically (scripts/macos-app.sh).
#
# Run once after cloning. Re-run if the repo moves.
# Safe to run repeatedly (just overwrites).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")" && pwd)"
BINARY="$REPO_ROOT/target/debug/kopico-desktop"
APP_ID="in.tl2a.kopico-desktop"

install_linux() {
    local desktop_src="$REPO_ROOT/data/$APP_ID.desktop"
    local dest_dir="$HOME/.local/share/applications"
    local dest="$dest_dir/$APP_ID.desktop"

    mkdir -p "$dest_dir"

    # Copy the template and patch Exec= to the actual debug binary path
    sed "s|Exec=.*|Exec=$BINARY|" "$desktop_src" > "$dest"

    echo "Installed: $dest"
    echo "  Exec= → $BINARY"

    # Refresh the GNOME app database so GAppInfo picks up the new entry immediately
    if command -v update-desktop-database &>/dev/null; then
        update-desktop-database "$dest_dir"
        echo "Desktop database updated."
    fi
}

install_macos() {
    # Same bundle `cargo run` uses (see .cargo/config.toml): copied + ad-hoc signed
    # executable, registered with LaunchServices
    "$REPO_ROOT/scripts/macos-app.sh" install
    echo ""
    echo "\`cargo run\` now starts Kopico from this bundle. On the first start macOS asks"
    echo "to allow notifications; afterwards Kopico Desktop is listed in"
    echo "System Settings → Notifications."
}

case "$(uname -s)" in
    Darwin) install_macos ;;
    *)      install_linux ;;
esac

if [ ! -x "$BINARY" ]; then
    echo ""
    echo "Note: $BINARY doesn't exist yet — run \`cargo build\` first."
fi

echo ""
echo "Done. System notifications will work once the app is running."
echo "(Re-run this script if you move the repo.)"
