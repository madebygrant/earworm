#!/usr/bin/env bash
# Installs earworm from this clone: the terminal app, the desktop app, or both.
# Usage: scripts/install.sh [--cli | --desktop | --both] [--dry-run]
# Runs on macOS's stock bash 3.2, so no associative arrays or fractional read timeouts.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="${TMPDIR:-/tmp}"
log=""
dry=0
choice=""

pick() {
  if [ -n "$choice" ] && [ "$choice" != "$1" ]; then
    echo "install: choose one of --cli, --desktop and --both" >&2
    exit 2
  fi
  choice=$1
}

for arg in "$@"; do
  case "$arg" in
    --cli) pick cli ;;
    --desktop) pick desktop ;;
    --both) pick both ;;
    --dry-run) dry=1 ;;
    -h | --help)
      sed -n '2,3p' "$0" | sed 's/^# //'
      exit 0 ;;
    *) echo "install: unknown option $arg (try --help)" >&2; exit 2 ;;
  esac
done

tty=0
[ -t 1 ] && [ -t 0 ] && tty=1
if [ "$tty" = 1 ] && [ -z "${NO_COLOR:-}" ]; then
  bold=$'\033[1m' dim=$'\033[2m' reset=$'\033[0m'
  accent=$'\033[38;5;213m' green=$'\033[32m' red=$'\033[31m' yellow=$'\033[33m'
else
  bold="" dim="" reset="" accent="" green="" red="" yellow=""
fi

cursor_on() { [ "$tty" = 1 ] && printf '\033[?25h'; return 0; }
cursor_off() { [ "$tty" = 1 ] && printf '\033[?25l'; return 0; }
trap 'cursor_on' EXIT
trap 'cursor_on; printf "\n"; exit 130' INT TERM

width() {
  local cols
  cols=$(stty size 2>/dev/null </dev/tty | cut -d' ' -f2) || true
  [ "${cols:-0}" -ge 40 ] 2>/dev/null || cols=80
  echo "$cols"
}

ok() { printf '  %s✓%s %s\n' "$green" "$reset" "$1"; }
bad() { printf '  %s✗%s %s\n' "$red" "$reset" "$1"; }
warn() { printf '  %s!%s %s\n' "$yellow" "$reset" "$1"; }

banner() {
  printf '\n  %s%s♪ earworm%s %sinstaller%s\n' "$bold" "$accent" "$reset" "$dim" "$reset"
  printf '  %sYouTube playlists to tagged music files%s\n\n' "$dim" "$reset"
}

labels=("Terminal app" "Desktop app" "Both")
hints=(
  "earworm in ~/.cargo/bin, for the terminal and SSH"
  "$([ "$(uname -s)" = Darwin ] && echo "earworm.app in /Applications" || echo "earworm-desktop in ~/.local/bin, with a menu entry")"
  "the terminal app and the desktop app"
)
values=(cli desktop both)

draw_menu() {
  local i
  for i in 0 1 2; do
    if [ "$i" = "$1" ]; then
      printf '\033[2K  %s❯ %s%s%s  %s%s%s\n' "$accent" "$bold" "${labels[$i]}" "$reset" "$dim" "${hints[$i]}" "$reset"
    else
      printf '\033[2K    %s\n' "${labels[$i]}"
    fi
  done
  printf '\033[2K  %s↑↓ move · enter choose · 1-3 pick · q quit%s\n' "$dim" "$reset"
}

menu() {
  local at=2 key rest
  printf '  %sWhat would you like to install?%s\n\n' "$bold" "$reset"
  cursor_off
  draw_menu "$at"
  while true; do
    IFS= read -rsn1 key </dev/tty || key=q
    if [ "$key" = $'\033' ]; then
      # An arrow is Esc [ A; a lone Esc's next key is handled as itself.
      if ! IFS= read -rsn1 -t 1 rest </dev/tty; then
        rest=$'\033'
      fi
      if [ "$rest" = "[" ]; then
        IFS= read -rsn1 rest </dev/tty || rest=""
        case "$rest" in
          A) key=k ;;
          B) key=j ;;
          *) key=$'\033' ;;
        esac
      else
        key=$rest
      fi
    fi
    case "$key" in
      k) at=$(((at + 2) % 3)) ;;
      j) at=$(((at + 1) % 3)) ;;
      1 | 2 | 3) at=$((key - 1)); printf '\033[4A'; draw_menu "$at"; break ;;
      "") break ;;
      q | Q) cursor_on; printf '\n  Nothing installed.\n\n'; exit 0 ;;
    esac
    printf '\033[4A'
    draw_menu "$at"
  done
  cursor_on
  printf '\033[6A\033[J  %sWhat would you like to install?%s %s%s%s\n\n' "$bold" "$reset" "$accent" "${labels[$at]}" "$reset"
  choice="${values[$at]}"
}

ask_plain() {
  local i answer
  echo "  What would you like to install?"
  for i in 0 1 2; do printf '    %d) %s  %s\n' $((i + 1)) "${labels[$i]}" "${hints[$i]}"; done
  printf '  Choose [3]: '
  read -r answer || answer=""
  case "${answer:-3}" in
    1) choice=cli ;; 2) choice=desktop ;; 3) choice=both ;;
    *) echo "  Pick 1, 2 or 3." >&2; exit 2 ;;
  esac
  echo
}

frames=(⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏)

# Output goes to the log; the spinner shows its latest line.
step() {
  local label=$1 start=$SECONDS pid i=0 line cols elapsed
  shift
  echo "==> $label: $*" >>"$log"
  if [ "$dry" = 1 ]; then
    ok "$label ${dim}(dry run: $*)${reset}"
    return 0
  fi
  if [ "$tty" = 0 ]; then
    echo "  … $label"
    if "$@" >>"$log" 2>&1; then ok "$label"; return 0; fi
    bad "$label"; return 1
  fi
  "$@" >>"$log" 2>&1 &
  pid=$!
  cursor_off
  cols=$(width)
  while kill -0 "$pid" 2>/dev/null; do
    elapsed=$((SECONDS - start))
    line=$(tail -n 1 "$log" 2>/dev/null | tr -d '\r' | sed $'s/\033\\[[0-9;]*[A-Za-z]//g; s/^[[:space:]]*//')
    line=${line:0:$((cols - ${#label} - 16))}
    printf '\r\033[2K  %s%s%s %s %s%ds  %s%s' "$accent" "${frames[$i]}" "$reset" "$label" "$dim" "$elapsed" "$line" "$reset"
    i=$(((i + 1) % 10))
    sleep 0.1
  done
  elapsed=$((SECONDS - start))
  printf '\r\033[2K'
  cursor_on
  if wait "$pid"; then
    ok "$label ${dim}${elapsed}s${reset}"
    return 0
  fi
  bad "$label ${dim}${elapsed}s${reset}"
  return 1
}

fail() {
  printf '\n  %sThe install stopped.%s The last of its output:\n\n' "$red" "$reset"
  tail -n 15 "$log" | sed "s/^/    ${dim}/; s/\$/${reset}/"
  printf '\n  The full log is %s\n\n' "$log"
  exit 1
}

banner
if [ -z "$choice" ]; then
  if [ "$tty" = 1 ]; then menu; else ask_plain; fi
fi

want_cli=0 want_desktop=0
case "$choice" in
  cli) want_cli=1 ;;
  desktop) want_desktop=1 ;;
  both) want_cli=1 want_desktop=1 ;;
esac

printf '  %sChecking what it needs%s\n' "$bold" "$reset"
missing=0
need() {
  if command -v "$1" >/dev/null 2>&1; then ok "$1 ${dim}$2${reset}"
  else bad "$1 ${dim}$2${reset}"; missing=1; fi
}
need cargo "to build earworm (https://rustup.rs)"
if [ "$want_desktop" = 1 ]; then
  need node "for the desktop app's page (https://nodejs.org)"
  need pnpm "for the desktop app's page (https://pnpm.io)"
  if [ "$(uname -s)" = Linux ]; then
    need pkg-config "to find the system libraries Tauri builds against"
    if command -v pkg-config >/dev/null 2>&1; then
      if pkg-config --exists webkit2gtk-4.1; then ok "webkit2gtk-4.1 ${dim}the desktop app's web view${reset}"
      else
        bad "webkit2gtk-4.1 ${dim}the desktop app's web view${reset}"
        printf '    %ssee https://v2.tauri.app/start/prerequisites/#linux for the packages%s\n' "$dim" "$reset"
        missing=1
      fi
    fi
  fi
fi
if [ "$missing" = 1 ]; then
  printf '\n  Install what is marked %s✗%s and run this again.\n\n' "$red" "$reset"
  exit 1
fi

runtime=""
for tool in yt-dlp ffmpeg fpcalc; do
  command -v "$tool" >/dev/null 2>&1 || runtime="$runtime $tool"
done
if [ -n "$runtime" ]; then
  warn "earworm runs yt-dlp, ffmpeg and fpcalc, and${runtime} is not on your PATH"
  printf '    %sbrew install yt-dlp ffmpeg chromaprint, or your package manager'"'"'s equivalents%s\n' "$dim" "$reset"
fi

printf '\n  %sInstalling%s %s(the first build takes a few minutes)%s\n' "$bold" "$reset" "$dim" "$reset"
log=$(mktemp "${tmp%/}/earworm-install.XXXXXX")
if [ "$want_cli" = 1 ]; then
  step "Terminal app" cargo install --path "$root/crates/tui" --locked || fail
fi
if [ "$want_desktop" = 1 ]; then
  step "Desktop app" "$root/scripts/install-desktop.sh" || fail
fi

printf '\n  %s%sDone.%s\n' "$bold" "$green" "$reset"
if [ "$want_cli" = 1 ]; then
  bin="${CARGO_HOME:-$HOME/.cargo}/bin"
  printf '  Run %searworm%s in a terminal.\n' "$accent" "$reset"
  case ":$PATH:" in
    *":$bin:"*) ;;
    *) warn "$bin is not on your PATH, so add it to your shell's profile" ;;
  esac
fi
if [ "$want_desktop" = 1 ]; then
  where=$(grep -o 'Installed .*' "$log" | tail -n 1 | sed 's/^Installed //') || true
  if [ "$(uname -s)" = Darwin ]; then
    printf '  Open %searworm%s from %s.\n' "$accent" "$reset" "$(dirname "${where:-/Applications/earworm.app}")"
  else
    printf '  Start %searworm%s from your app menu, or run earworm-desktop.\n' "$accent" "$reset"
  fi
fi
printf '  %searworm --check says whether the tools it runs are all there.%s\n\n' "$dim" "$reset"
rm -f "$log"
