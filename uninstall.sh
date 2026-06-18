#!/usr/bin/env bash
#
# OpenShotX uninstaller. Removes the binary, icon, desktop entry, and the
# tray autostart entry. Mirrors install.sh's prefix handling.
#
# Usage:
#   ./uninstall.sh               # remove from ~/.local
#   sudo ./uninstall.sh --system # remove from /usr/local
#
set -euo pipefail

SYSTEM=0
for arg in "$@"; do
    case "$arg" in
        --system) SYSTEM=1 ;;
        -h|--help) grep '^#' "$0" | sed 's/^# \?//'; exit 0 ;;
        *) echo "unknown option: $arg" >&2; exit 1 ;;
    esac
done

if [ "$SYSTEM" -eq 1 ]; then
    PREFIX="${PREFIX:-/usr/local}"
else
    PREFIX="${PREFIX:-$HOME/.local}"
fi

rm -fv "$PREFIX/bin/openshotx"
rm -fv "$PREFIX/share/icons/hicolor/scalable/apps/openshotx.svg"
rm -fv "$PREFIX/share/applications/openshotx.desktop"
# Tray autostart entry is always per-user.
rm -fv "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/openshotx-tray.desktop"

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
command -v gtk-update-icon-cache  >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true

echo "==> OpenShotX uninstalled from $PREFIX (config at ~/.config/openshotx left intact)."
