# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build
cargo build --release

# Install after build
install -Dm755 target/release/openshotx ~/.local/bin/openshotx

# Test
cargo test

# Run a single test
cargo test test_name_substring

# Lint
cargo clippy

# Full pre-commit check
cargo test && cargo clippy && cargo build --release
```

## Architecture

OpenShotX is a native Rust screenshot/recording tool for Linux (X11 and Wayland), structured around a backend abstraction layer.

**Backend trait** (`src/backend/mod.rs`): `DisplayBackend` is the core abstraction. `CaptureData` wraps a raw pixel buffer with width/height and a `PixelFormat` enum that handles the many variants of pixel layout (RGB/BGR, 24/32-bit, LSB/MSB byte order) found in the wild. Both backends implement this trait so capture logic is backend-agnostic.

- **X11** (`src/backend/x11.rs`): Uses `x11rb` (pure Rust, no Xlib). GTK4 transparent overlay (`src/overlay.rs`) for drag-to-select area capture — goes straight to selection without a dialog.
- **Wayland** (`src/backend/wayland.rs`): Uses `ashpd` (xdg-desktop-portal). Area/window capture requires the portal's security dialog — this cannot be bypassed by design. Manual zbus calls handle portal features that ashpd doesn't expose.

**Capture pipeline**: `src/capture/mod.rs` converts `CaptureData` → PNG/JPEG (via `image` crate), composites the cursor, generates timestamped filenames (screenshots → `~/Pictures`, recordings → `~/Videos`), and copies to clipboard (`wl-copy` on Wayland, `xclip` on X11).

**OCR** (`src/ocr/mod.rs`): Tesseract integration with a preprocessing pipeline tuned for dark-mode UIs: 3× Lanczos upscale → color inversion → contrast enhancement. This pipeline achieves ~91% confidence on dark-mode chat text; don't remove steps without benchmarking.

**Recording** (`src/recording/mod.rs`): GStreamer pipeline with codec fallback (hardware → software → Theora). GIF recording goes through FFmpeg with palettegen for quality. Uses PipeWire for Wayland screen sharing.

**Scrolling capture** (`src/scrolling/mod.rs`): **BETA — has known quality issues.** Captures frames from a GStreamer/PipeWire stream, deduplicates by pixel diff, and stitches with overlap detection. Key problems: 67% duplicate frames (GStreamer at fixed 25fps), O(n²) overlap search, and Wayland portal captures the full monitor (not the selected region, unavoidable security limitation). See `dev-docs/SCROLLING_CAPTURE.md` for full analysis before touching this module.

## Dev Docs

`dev-docs/` is **gitignored** — do not commit it. It contains:
- `SCROLLING_CAPTURE.md` — detailed analysis of scrolling capture issues and attempted solutions (read this before working on `src/scrolling/`)
- `PROGRESS.md` — development history
- `NEXT_SESSION.md` — transient notes between sessions

Always update relevant `dev-docs/` files after completing a task, then commit only the source changes.
