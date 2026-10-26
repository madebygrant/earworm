# Wave plan: earworm as a Tauri desktop app (option B)

Written 2026-10-26 against `main` at `fcc7edd`. Line counts and names are
hints; re-check before acting. On approval this is saved as
`docs/plans/2026-10-26-native-app-plan.md`, beside the other wave plans.

## Context

earworm is a 40k-line Rust TUI. The request is a real desktop GUI, chosen
over wrapping the TUI in a terminal window. Decisions made: **the TUI stays**
as a supported front end on a shared core; the GUI is **Tauri 2 with Svelte
and TypeScript**; the first release targets **macOS and Linux**.

**Already true, found while planning:**

- The worker never touches the terminal. It talks to the UI only through
  `Cmd` (38 variants) and `Msg` (40), both in `src/app.rs`. That is the seam
  a second front end plugs into.
- The protocol types live in `app.rs`, so non-UI modules import from the UI
  module: `tag.rs` (`Asker`, `Status`), `custom.rs`, `export.rs`,
  `discover.rs` (`AlbumRow`, `TopRow`, `RelatedRow`), `artists.rs`
  (`ArtistRow`), `picker.rs` (`Shelf`), `player.rs` (`Playback`, `Player`).
- `Msg::Ask(Prompt, Sender<Reply>)` (`app.rs:769`) carries a channel, which
  cannot cross to a webview. `Asker` (`app.rs:878`) blocks the worker on it.
- Much screen logic is in `App`, not the worker: `rows()`, `shelf_rows()`,
  `found_hits`, sort and kind filters, the pick gate's marks, the picker's
  `nucleo` matching, the settings rows, flashes and their queue.
- Tools are run by bare name (`yt-dlp`, `ffmpeg`, `fpcalc`, `cliamp`) and
  `deps.rs` reports them "on PATH". A Finder-launched macOS app gets a minimal
  `PATH` without `/opt/homebrew/bin`, so every tool would read as missing.
- `serde` is already a dependency.

## 1. Move the protocol out of `app.rs`

Why first: every later wave depends on the core not importing the TUI, and
it is a pure move with the suite as its check.

- New `src/protocol.rs`: `Msg`, `Cmd`, `Track`, `Status`, `Shelf`, `Prompt`,
  `Reply`, `Escape`, `Asker`, the row structs (`ArtistRow`, `AlbumRow`,
  `TopRow`, `RelatedRow`, `PlaylistRow`), `Playback`, `Player`. `app.rs`
  re-exports them so the TUI's own code changes little.
- Every non-UI module imports from `protocol`, never `app`.
- Read `docs/rules/threads-and-messages.md` first; its rules move with the
  types.

Exit: `cargo test` passes and `grep -n "crate::app" src/{worker,tag,lookup,custom,export,discover,artists,picker,player,playlists}.rs` finds nothing.

## 2. Split into a workspace

- `crates/core` (lib): worker, config, tag, lookup, ytdlp, manifest, artists,
  discover, custom, playlists, export, deps, update, cheats, achievements,
  theme data, protocol.
- `crates/tui` (bin `earworm`): main, app, ui, icons, the intro.
- Tests move with their modules. Fixtures shared through a `test-support`
  feature on core rather than copied.
- The binary name, flags and `cargo install` stay the same.
- Update CLAUDE.md's module list and the rules table paths.

Exit: `cargo test --workspace` passes with the same count as before the
split, and `cargo build -p earworm-tui` produces `earworm`.

## 3. A serialisable protocol with question ids

Why here: the GUI cannot exist without it, and the TUI moving onto the same
path is what keeps one protocol.

- `Serialize` on `Msg` and the row types, `Deserialize` on `Cmd` and `Reply`.
- `Msg::Ask(Prompt, Sender<Reply>)` becomes `Msg::Ask { id, prompt }`. Core
  holds a `Questions` registry (id to `Sender<Reply>`); a front end answers
  with `answer(id, reply)`. `Asker` registers before it sends, so an answer
  can never arrive for an id not yet registered.
- The pick gate's reply channel (`App.picking`) goes the same way.
- The TUI answers through the registry, so the pty and key tests exercise the
  path the GUI will use.
- Risk: the tests in `worker.rs` that destructure `Msg::Ask` to reply (about a
  dozen) all change; do them in one pass.

Exit: `cargo test --workspace` passes; a test round-trips every `Msg` and
`Cmd` variant through JSON from an exhaustive `match`, so a new variant that
cannot serialise fails to compile or fails the test.

## 4. Find tools without the shell's PATH

- `core::tools::bin(name)`: an explicit override from the config, then
  `PATH`, then `/opt/homebrew/bin`, `/usr/local/bin`, `~/.local/bin`. Every
  `Command::new("yt-dlp" | "ffmpeg" | "fpcalc" | "cliamp")` and `deps.rs` go
  through it.
- Bundling yt-dlp and ffmpeg is out of scope for the first release: yt-dlp
  needs frequent updates and ffmpeg's licence depends on the build. The GUI
  says which tool is missing and how to install it, as `--check` does.
- Read `docs/rules/dependencies-and-updates.md` and `ui-details.md`
  (`run_bounded`) first.

Exit: `env -i HOME=$HOME PATH=/usr/bin:/bin earworm --check` finds the
Homebrew tools on a machine that has them.

## 5. The view model both front ends share

Why before the GUI: these rules have tests and were each learned the hard
way; the GUI should call them, not reimplement them in TypeScript.

- Move into core, as pure functions over state passed in:
  - `rows()` and the track filter, review mode;
  - `shelf_rows()`, sort, the kind filter;
  - `found_hits`;
  - the pick gate's marks;
  - the picker's matching;
  - the artist query.
- Cursors, scroll, flashes and drawing stay in each front end.
- The tests for these move to core unchanged.
- Read `docs/rules/lists-and-search.md` and `threads-and-messages.md` first.

Exit: the moved tests pass in core, the TUI calls core for each, and no
filter or sort logic is left in `crates/tui/src/app.rs`.

## 6. The Tauri shell

- `crates/gui`: Tauri 2, Svelte and TypeScript through Vite.
- The backend:
  - spawns `worker::run` on a thread;
  - forwards every `Msg` as an `earworm://msg` event;
  - exposes `send(cmd)`, `answer(id, reply)` and the view-model functions as
    commands;
  - passes `stop` and `cancel` as the TUI does.
- Config, `--dir` and the YouTube Music unlock behave as in the TUI. The cheat
  console is a hidden key chord, so the unlock stays a secret.
- A Svelte store applies `Msg` the way `App::apply` does, in one reducer.
- Reducer tests run in Vitest against `Msg` JSON recorded from the Rust tests.

Exit: the app opens on a fixture `--dir` and lists its folders from
`Msg::Library`; the reducer tests pass.

## 7. Library, runs and questions

- Screens:
  - the library: sort, kind filter, filter box, sync, new URL;
  - the track list, with live statuses and the log pane;
  - the pick gate;
  - every `Prompt` kind (choice, input, form) as a dialog;
  - quit while a download runs.
- Keyboard shortcuts match the TUI's where they apply, from
  `docs/keys.md`.
- Read `questions-and-forms.md` and `keys-and-screens.md` first.

Exit: with a stub `yt-dlp` on the resolved path, syncing a fixture playlist
from the GUI writes its `.m3u8` and `.earworm` (wait on the files), and an
answered pick gate downloads only what was marked.

## 8. The remaining screens

- Search (`t`), custom playlists and the picker, export, tag edit and undo,
  covers (plain `<img>`, no kitty graphics), settings, help.
- Artists, the artist page, the YouTube Music fetch, hidden until unlocked.
- Each screen checked against the same fixture library, one at a time;
  any can slip to a later release except settings.

Exit: every TUI screen has a GUI counterpart, and the discovery rules hold
in the GUI (the locked page offers no download).

## 9. Packaging for macOS and Linux

- macOS: `.app` and `.dmg`, **unsigned**: no Developer ID, no notarisation.
  - Apple Silicon refuses to run code with no signature at all, so the build
    keeps the linker's ad-hoc signature (`codesign -s -`), which Tauri's
    bundler applies when no identity is set. Check it survives bundling.
  - Gatekeeper blocks a downloaded unsigned app on first launch. On macOS 15
    and later right-click Open no longer bypasses it: the user opens it once
    from System Settings > Privacy & Security > Open Anyway, or runs
    `xattr -dr com.apple.quarantine /Applications/earworm.app`. The install
    docs say both, with the reason.
  - No auto-updater: Tauri's updater needs signed update bundles. The app
    can point at the GitHub release page, as the TUI's update check does.
- Linux: AppImage and `.deb` on WebKitGTK; check `xdg-open` in
  `player::reveal` and the cliamp handoff.
- CI builds both on tags. The TUI keeps its own release.

Exit: the `.dmg`, downloaded through a browser so it carries the quarantine
flag, opens for a fresh macOS user after Open Anyway and finds the Homebrew
tools; `codesign -dv` shows the ad-hoc signature; the AppImage runs on a
stock Ubuntu LTS.

## 10. Docs

- README: a GUI section and install steps, including the unsigned-app
  first launch on macOS.
- New `docs/gui.md`.
- New `docs/rules/gui.md`, covering the reducer, question ids and tool
  resolution, with a row in CLAUDE.md's rules table.
- Update CLAUDE.md's architecture to describe two front ends on one core.

Exit: every new rule names its test or says nothing tests it.

## Why this order

Waves 1 to 5 change no behaviour and leave the TUI better structured on their
own, so the project can stop after any of them. The GUI starts only once the
seam is serialisable and the rules it needs are callable. The screens go
from the most-used down, and packaging comes last because it depends on what
the app ships.

## What can slip

Wave 8's less-used screens (export, covers, cheat console) can ship in a
later release. Bundling yt-dlp and ffmpeg is deferred by design, and so is signing. Windows is
out of scope.

## Verification

- `cargo test --workspace` after every wave, same count or more.
- From wave 6: `npm test` (Vitest) for the reducer, and `cargo tauri dev`
  against a fixture `--dir` with a stub `yt-dlp`, waiting on files rather than
  the screen, as CLAUDE.md requires for the TUI.
- Before packaging: launch from Finder, not a shell, to prove tool resolution.
