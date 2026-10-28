# Wave plan: earworm as a Tauri desktop app (option B)

Written 2026-10-26 against `main` at `fcc7edd`. Line counts and names are
hints; re-check before acting. On approval this is saved as
`docs/plans/2026-10-26-native-app-plan.md`, beside the other wave plans.

## Context

earworm is a 40k-line Rust TUI. The request is a real desktop GUI, chosen
over wrapping the TUI in a terminal window. Decisions made: **the TUI stays**
as a supported front end on a shared core; the GUI is **Tauri 2 with Svelte
and TypeScript**; the first release targets **macOS and Linux**. How it
looks is set in `docs/plans/2026-10-28-desktop-ux-plan.md`, with a mockup of
every screen in `docs/mockups/`: the TUI's look in one monospace face, the
open folder as a sheet of cover tiles, and a new `graphite` palette as the
default. Waves 2, 3 and 6 to 10 carry what that plan needs.

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

## 1. Move the protocol out of `app.rs` [done 2026-10-28]

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

## 2. Split into a workspace [done 2026-10-28]

- `crates/core` (lib): worker, config, tag, lookup, ytdlp, manifest, artists,
  discover, custom, playlists, export, deps, update, cheats, achievements,
  theme data, protocol.
- `crates/tui` (bin `earworm`): main, app, ui, icons, the intro.
- Tests move with their modules. Fixtures shared through a `test-support`
  feature on core rather than copied.
- Three `worker.rs` tests run the worker, feed its messages to `App` and draw
  the screen (one near `can_purge`, two checking the `≈` twin marker). Core
  cannot reach `App`, so move them to the TUI crate on core's
  `test-support`, or assert on the `Msg` values instead. Left from wave 1.
- `Cmd::Icons` carries `icons::Icons`, and `config.rs` uses it too, but
  `icons.rs` is listed for the TUI. Move the `Icons` enum (the choice, not
  the glyph tables) into core, or `Cmd` will not compile there.
- `Palette` holds `ratatui::style::Color`, so theme data in core still pulls
  in ratatui. Either core keeps ratatui as a dependency for the colour type
  alone, or palettes become plain RGB triples with the TUI converting. Decide
  before the move; wave 6's CSS generation wants the RGB form anyway.
- The binary name, flags and `cargo install` stay the same.
- Add the `graphite` palette (UX plan, colour table) beside the four in
  `theme.rs`. If the TUI ships it too, it goes in `BUILT_INS` and
  `every_palette_is_legible_on_its_own_ground` covers it; if not, it is a
  GUI-only constant that test iterates as well. Decide before the move, since
  it is the UX plan's one open question that touches the TUI. Read
  `docs/rules/themes-and-colour.md` first.
- Update CLAUDE.md's module list and the rules table paths.

Exit: `cargo test --workspace` passes with the same count as before the
split, and `cargo build -p earworm` produces `earworm`.

## 3. A serialisable protocol with question ids [done 2026-10-28]

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
- A new `Msg` for cover thumbnails, which the GUI's tiles need. The worker
  writes a 256px JPEG of each track's embedded art into a cache folder when a
  track settles or a folder opens, and sends its path; the webview loads it
  through Tauri's asset protocol. Image bytes stay out of the JSON channel and
  the UI side never reads an audio file, which
  `docs/rules/threads-and-messages.md` forbids. The TUI ignores the message.

Exit: `cargo test --workspace` passes; a test round-trips every `Msg` and
`Cmd` variant through JSON from an exhaustive `match`, so a new variant that
cannot serialise fails to compile or fails the test.

## 4. Find tools without the shell's PATH [done 2026-10-28]

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

## 5. The view model both front ends share [done 2026-10-28]

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

## 6. The Tauri shell [done 2026-10-28]

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
- Tokens and styles come from `docs/mockups/tokens.css` and `base.css`; the
  look is set in `docs/plans/2026-10-28-desktop-ux-plan.md`.
- The CSS variables are generated at start from core's palettes, not copied
  by hand, so `[themes.*]` and `[colors]` from the config apply in the GUI as
  they do in the TUI. The role names already match `Palette`'s fields.
- `^t` and the theme dropdown save the theme the way the TUI's `^t` does;
  `themes-and-colour.md` says why that one write skips the worker.
- JetBrains Mono at 400 and 700 is bundled as local files, never fetched.

Exit: the app opens on a fixture `--dir` and lists its folders from
`Msg::Library` in the rail; the reducer tests pass; `^t` walks every palette
and a `[colors]` override in the fixture config shows on screen.

## 7. Library, runs and questions

- Screens, each with its mockup:
  - first launch, with `--check`'s tools table (`first-launch.html`);
  - the library in the rail: sort, kind filter, filter box, sync, new URL
    (`library.html`);
  - the open folder as tiles, with live statuses, the cover settling, follow
    mode and the log pane, and the list one key away on `w` (`run.html`);
  - the pick gate, always as rows (`pick.html`);
  - every `Prompt` kind (choice, input, form) as a dialog (`prompts.html`);
  - the detail pane in every status (`detail.html`);
  - quit while a download runs (`overlays.html`).
- Keyboard shortcuts match the TUI's where they apply, from
  `docs/keys.md`; `w` is the one new key.
- Until the thumbnail message lands, a settled tile shows its number; the
  layout does not wait on it.
- Read `questions-and-forms.md` and `keys-and-screens.md` first.

- Carried from wave 6's review: the page stores `Msg::Ask` and `Msg::Pick`
  but draws and answers neither, so a first run with an empty library sits
  at the URL prompt.

Exit: with a stub `yt-dlp` on the resolved path, syncing a fixture playlist
from the GUI writes its `.m3u8` and `.earworm` (wait on the files), and an
answered pick gate downloads only what was marked. Each screen, captured at
1440 wide in graphite, matches its picture in `docs/mockups/shots/` in layout
and colour.

## 8. The remaining screens

- Search (`t`), custom playlists and the picker, export, tag edit and undo,
  covers (plain `<img>`, no kitty graphics), settings, help.
- Artists, the artist page, the YouTube Music fetch, hidden until unlocked.
- Mocked already: artists and the artist page (`artists.html`), settings
  (`settings.html`), search, the key sheet, the achievement toast and the
  console (`overlays.html`). Not yet mocked: custom playlists, the picker,
  export and the cover chooser. Mock each in `docs/mockups/` on the shared
  shell before building it, so it is reviewed as a picture first.
- Each screen checked against the same fixture library, one at a time;
  any can slip to a later release except settings.

- Carried from wave 6's review: the reducer does not mark an artist page's
  new releases or record the page as read, which `App::apply` does with
  `artists::key`. Move that marking into core before the page uses it.

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
- The bundled JetBrains Mono ships with its OFL licence file. Check the font
  renders on WebKitGTK with no system copy installed.
- CI builds both on tags. The TUI keeps its own release.

Exit: the `.dmg`, downloaded through a browser so it carries the quarantine
flag, opens for a fresh macOS user after Open Anyway and finds the Homebrew
tools; `codesign -dv` shows the ad-hoc signature; the AppImage runs on a
stock Ubuntu LTS.

## 10. Docs

- README: a GUI section and install steps, including the unsigned-app
  first launch on macOS.
- New `docs/gui.md`, illustrated with the shots in `docs/mockups/shots/`
  retaken from the real app.
- `docs/keys.md`: `w`, tiles or list, marked as the GUI's alone.
- New `docs/rules/gui.md`, covering the reducer, question ids, tool
  resolution, the palette-to-CSS generation and the thumbnail message, with a
  row in CLAUDE.md's rules table.
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
later release. Real covers in tiles can slip too: without the thumbnail
message, tiles show their numbers and the list view is the better default.
Bundling yt-dlp and ffmpeg is deferred by design, and so is signing. Windows
is out of scope.

## Verification

- `cargo test --workspace` after every wave, same count or more.
- From wave 6: `npm test` (Vitest) for the reducer, and `cargo tauri dev`
  against a fixture `--dir` with a stub `yt-dlp`, waiting on files rather than
  the screen, as CLAUDE.md requires for the TUI.
- From wave 2: `node docs/mockups/contrast.mjs` exits 0, and the Rust
  contrast test covers `graphite`.
- From wave 7: each screen compared with its picture in
  `docs/mockups/shots/`, and checked with `prefers-reduced-motion` on, where
  the cover arrives without stepping and the scan line is gone.
- Before packaging: launch from Finder, not a shell, to prove tool resolution.
