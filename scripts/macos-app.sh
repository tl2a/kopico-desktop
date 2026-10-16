#!/usr/bin/env bash
# macos-app.sh — run / install Kopico as a real macOS app bundle.
#
# macOS notifications (UNUserNotificationCenter) only work for a process started from a
# signed .app bundle whose bundle id is known to LaunchServices. A bare `cargo run`
# binary has no bundle, so:
#
#   scripts/macos-app.sh install            build "~/Applications/Kopico Desktop.app"
#                                           from target/debug/kopico-desktop
#   scripts/macos-app.sh <binary> [args…]   cargo runner (see .cargo/config.toml):
#                                           refresh the bundle with <binary> and run it
#                                           from inside the bundle
#
# The bundle is ad-hoc signed (no Apple account needed). Notification permission is
# tied to the bundle id, so it survives rebuilds.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_ID="in.tl2a.kopico-desktop"
APP="$HOME/Applications/Kopico Desktop.app"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

refresh_bundle() {
    local binary="$1"
    mkdir -p "$APP/Contents/MacOS"
    cp "$REPO_ROOT/data/macos/Info.plist" "$APP/Contents/Info.plist"
    # A copy, not a symlink: the bundle must contain the real executable to be signed
    rm -f "$APP/Contents/MacOS/kopico-desktop"
    cp "$binary" "$APP/Contents/MacOS/kopico-desktop"
    # Unsigned apps are refused notification permission; ad-hoc signing is enough
    codesign --force --sign - --identifier "$APP_ID" "$APP" 2>/dev/null
    "$LSREGISTER" -f "$APP"
}

case "${1:-}" in
    install)
        binary="$REPO_ROOT/target/debug/kopico-desktop"
        [ -x "$binary" ] || { echo "Build first: cargo build" >&2; exit 1; }
        refresh_bundle "$binary"
        echo "Installed: $APP (bundle id $APP_ID, ad-hoc signed)"
        ;;
    "")
        echo "usage: $0 install | <binary> [args…]" >&2
        exit 2
        ;;
    *)
        binary="$1"
        shift
        # Only the app itself goes into the bundle; test binaries etc. run as usual
        if [ "$(basename "$binary")" != "kopico-desktop" ]; then
            exec "$binary" "$@"
        fi
        refresh_bundle "$binary"
        exec "$APP/Contents/MacOS/kopico-desktop" "$@"
        ;;
esac
