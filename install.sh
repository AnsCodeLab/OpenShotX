#!/usr/bin/env bash
#
# OpenShotX installer.
#
# Builds the release binary (unless --no-build) and installs the binary, icon,
# and desktop entry. Defaults to a per-user install under ~/.local; pass
# --system (or PREFIX=/usr/local) for a system-wide install (needs root).
#
# Usage:
#   ./install.sh                 # user install to ~/.local
#   ./install.sh --no-build      # install an already-built target/release binary
#   sudo ./install.sh --system   # system install to /usr/local
#   PREFIX=/opt/openshotx ./install.sh
#
set -euo pipefail

BUILD=1
SYSTEM=0
for arg in "$@"; do
    case "$arg" in
        --no-build) BUILD=0 ;;
        --system)   SYSTEM=1 ;;
        -h|--help)  grep '^#' "$0" | sed 's/^# \?//'; exit 0 ;;
        *) echo "unknown option: $arg" >&2; exit 1 ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Binary tarballs have no Cargo project — nothing to build.
if [ ! -f "$SCRIPT_DIR/Cargo.toml" ]; then
    BUILD=0
fi

# Resolve install prefix.
if [ "$SYSTEM" -eq 1 ]; then
    PREFIX="${PREFIX:-/usr/local}"
else
    PREFIX="${PREFIX:-$HOME/.local}"
fi

BIN_DIR="$PREFIX/bin"
ICON_DIR="$PREFIX/share/icons/hicolor/scalable/apps"
APP_DIR="$PREFIX/share/applications"
METAINFO_DIR="$PREFIX/share/metainfo"

echo "==> Installing OpenShotX to $PREFIX"

# Build.
if [ "$BUILD" -eq 1 ]; then
    if ! command -v cargo >/dev/null 2>&1; then
        echo "error: cargo not found. Install Rust (https://rustup.rs) or pass --no-build." >&2
        exit 1
    fi
    echo "==> Building release binary (cargo build --release)"
    cargo build --release
fi

# Locate the binary: source tree (target/release) or binary tarball (alongside).
if [ -x "$SCRIPT_DIR/target/release/openshotx" ]; then
    BIN_SRC="$SCRIPT_DIR/target/release/openshotx"
elif [ -x "$SCRIPT_DIR/openshotx" ]; then
    BIN_SRC="$SCRIPT_DIR/openshotx"
else
    echo "error: openshotx binary not found (looked in target/release/ and ./). Build first." >&2
    exit 1
fi

# Install files.
install -Dm755 "$BIN_SRC"                     "$BIN_DIR/openshotx"
install -Dm644 "$SCRIPT_DIR/data/openshotx.svg" "$ICON_DIR/openshotx.svg"

# The desktop entry's Exec must point at the installed binary.
DESKTOP_TMP="$(mktemp)"
sed "s#Exec=openshotx#Exec=$BIN_DIR/openshotx#g" \
    "$SCRIPT_DIR/data/openshotx.desktop" > "$DESKTOP_TMP"
install -Dm644 "$DESKTOP_TMP" "$APP_DIR/openshotx.desktop"
rm -f "$DESKTOP_TMP"

# AppStream metainfo (powers the "App Details" panel in GNOME Software).
install -Dm644 "$SCRIPT_DIR/data/io.github.anscodelab.openshotx.metainfo.xml" \
    "$METAINFO_DIR/io.github.anscodelab.openshotx.metainfo.xml"

# Refresh caches (best-effort).
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APP_DIR" 2>/dev/null || true
command -v gtk-update-icon-cache  >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true

echo "==> Installed:"
echo "    binary:   $BIN_DIR/openshotx"
echo "    icon:     $ICON_DIR/openshotx.svg"
echo "    desktop:  $APP_DIR/openshotx.desktop"
echo "    metainfo: $METAINFO_DIR/io.github.anscodelab.openshotx.metainfo.xml"

# Warn if the bin dir isn't on PATH (common for ~/.local/bin on some setups).
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "note: $BIN_DIR is not on your PATH; add it to use 'openshotx' directly." ;;
esac

echo "==> Done. Try: openshotx capture area    (or launch OpenShotX from your app menu)"
