#!/usr/bin/env bash
# Builds the desktop app and installs it for the current user.
# macOS: earworm.app into /Applications, or ~/Applications when that is not writable.
# Linux: the AppImage as ~/.local/bin/earworm-desktop, with a menu entry.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
gui="$root/crates/gui"
bundle="${CARGO_TARGET_DIR:-$root/target}/release/bundle"

for tool in cargo pnpm; do
  command -v "$tool" >/dev/null || { echo "install-desktop: $tool is not on PATH" >&2; exit 1; }
done

case "$(uname -s)" in
  Darwin) bundles=app ;;
  Linux) bundles=appimage ;;
  *) echo "install-desktop: only macOS and Linux are supported" >&2; exit 1 ;;
esac

(cd "$gui" && pnpm install --frozen-lockfile && pnpm tauri build --bundles "$bundles")

if [ "$bundles" = app ]; then
  apps=/Applications
  [ -w "$apps" ] || { apps="$HOME/Applications"; mkdir -p "$apps"; }
  # Copied beside the old one and swapped in, so a failed copy never leaves no app at all.
  rm -rf "$apps/earworm.app.new"
  ditto "$bundle/macos/earworm.app" "$apps/earworm.app.new"
  rm -rf "$apps/earworm.app"
  mv "$apps/earworm.app.new" "$apps/earworm.app"
  echo "Installed $apps/earworm.app"
else
  image="$(ls -t "$bundle"/appimage/*.AppImage | head -n 1)"
  bin="$HOME/.local/bin"
  icons="$HOME/.local/share/icons/hicolor/128x128/apps"
  menu="$HOME/.local/share/applications"
  mkdir -p "$bin" "$icons" "$menu"
  install -m 755 "$image" "$bin/earworm-desktop"
  install -m 644 "$gui/src-tauri/icons/128x128.png" "$icons/earworm.png"
  cat >"$menu/earworm.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=earworm
Comment=Downloads a YouTube playlist as a tagged music library
Exec="$bin/earworm-desktop"
Icon=earworm
Categories=AudioVideo;Audio;
EOF
  echo "Installed $bin/earworm-desktop"
fi
