# earworm

Rust/ratatui TUI that downloads a YouTube playlist as tagged opus files, built
on a core library it shares with a Tauri desktop front end. See
README.md for user-facing behaviour.

## Architecture

Two threads, one channel each way, and that split is the design. The UI thread
owns the terminal and nothing else; the worker owns every subprocess, network
call and file write. Nothing that blocks touches the render loop. The one
exception is `^t` writing the theme to the config, and the reason it is one is
in docs/rules/themes-and-colour.md.

A Cargo workspace. `crates/core` (`earworm-core`, a library) holds everything
that is not drawing; `crates/tui` (package and binary `earworm`) is the
terminal front end on top of it. The core never depends on the TUI, and
`main.rs` re-exports the core's modules at the TUI's root so `crate::config`
and friends resolve there too. Test helpers the TUI's tests need from the core
sit behind core's `test-support` feature, which only the TUI's dev-dependency
turns on.

`crates/core/src`:

- `protocol.rs` — what the worker and a front end exchange: `Msg` (worker to
  UI), `Cmd` (UI to worker), `Track`, `Status`, `Shelf` (one library row),
  `Prompt`, `Reply`, `Asker` (worker asks the UI a question and blocks on the
  reply), `Escape` (what Esc on a prompt does), the row structs and
  `Playback`, all serde so a front end can speak JSON. A question is an id:
  `open_question` registers it, `answer(id, reply)` is the way back. Non-UI
  modules import these from here, never from `app`, and `app` keeps its
  imports private so the compiler holds that line
- `config.rs` — `Cli`, the TOML `FileConfig`, and `Config::build`, which merges
  them; `SWITCHES` and `Config::rows`, what the settings screen lists
- `worker.rs` — `ask_start` (library, resync or the URL prompt, which loops
  until the URL validates) then `pipeline` then `serve`, which serves both the
  post-run commands and the library screen. `tag_tracks` is the
  identify-and-write pass, shared by the run and by retry; `ask_which` is the
  pick gate between the scan and the download; `open_shelf` builds the same
  rows from the manifest and the tags on disk, with no network
- `ytdlp.rs` — `scan` (flat-playlist listing), `run` (the real download)
- `manifest.rs` — the `.earworm` sidecar: the `#url`, `#synced`, `#name`,
  `#m3u8`, `#kind` and `#hidden` headers, plus video id to current filename
- `tag.rs` — lofty reads and writes, `identify` decides a track's status
- `lookup.rs` — AcoustID, Deezer, Cover Art Archive, all through one `ureq`
  agent
- `player.rs` — the optional cliamp handoff: import the `.m3u8`, then `load`
- `cheats.rs` — the cheat console's codes and `Unlocked`, the session's
  unlocks, shared by the UI and the worker
- `achievements.rs` — the `Achievement` table, a pure `earned(Facts)`, and the
  state file that remembers which were announced
- `theme.rs` — `Palette` and the five themes it ships, the WCAG measurements
  that keep them legible, `background()` (the diagonal gradient painted behind
  every frame) and the 256-colour quantiser. Colours are `ratatui_core`'s, the
  one ratatui crate the core depends on
- `icons.rs` — the glyph sets behind `icons = "..."`
- `tools.rs` — `command`, how every external tool is started: from where
  `bin` finds it (the config's `[tools]` path, PATH, then Homebrew's and
  `~/.local`'s bin folders) and with those folders on its PATH
- `view.rs` — the list rules both front ends share, as pure functions over
  the state a front end holds: what the track filter and review keep, the
  library's narrowings and `Sort`, the `t` search's hits, the artist query,
  which tracks a marking key takes and what the pick gate opens with
- `artists.rs` — the artist index read off the tags, `key` (one artist under
  any spelling), and the `.artists` and `.artists-seen` stores
- `discover.rs` — Deezer's artists, releases, top tracks and related artists,
  `album_key`/`song_key`, the owned/partial match and the new-release check

`crates/tui/src`:

- `main.rs` — terminal setup, event loop, key handling, shutdown
- `app.rs` — `App` state, `View` (which screen has the keys), `picking` (the
  pick gate's question id), the `marked` set behind `targets()`, and the
  `filter` behind `rows()`, which asks `view` and keeps only the cursors
- `ui.rs` — every draw function

`crates/gui` is the desktop front end: Tauri 2 with Svelte and TypeScript
through Vite. `src-tauri/src/main.rs` (package `earworm-gui`, binary
`earworm-desktop`) starts the worker, sends every `Msg` to the page on the
`earworm://msg` event, and exposes `send`, `answer`, the theme and `view`
functions as commands. `src/lib/reducer.ts` is the page's `App::apply`, `src/lib/keys.ts` its
`handle_key` and `src/lib/screens.svelte.ts` the cursors and actions of the
screens past the run and the library;
`src/lib/msgs.json` is recorded from core's samples for its tests. Run it
with `pnpm dev` in `crates/gui` and `cargo run -p earworm-gui`, or
`pnpm tauri dev`; `pnpm test` runs the reducer tests.

## Rules

Each rule has the reason it exists, because the reason is what stops the next
person undoing it. They live in `docs/rules/`, one file per area, and are **not**
loaded with this file. Before changing code in an area below, read its file in
full: the rules there are the ones that have already bitten, and several say a
change that compiles and passes the suite is still wrong. A change that crosses
areas reads each. The measurements behind the rules are in docs/history.md,
which is read when somebody doubts a rule rather than at the start of a session.

| Area | File | Read when |
|---|---|---|
| Threads and messages: the UI/worker split, `Msg` and `Cmd`, the pick gate, `busy`, the filter predicate and cursors | `docs/rules/threads-and-messages.md` | touching `main.rs`'s event loop, `Msg`/`Cmd`, `Status`, the pick gate or the filter |
| Files and re-runs: the manifest sidecar, resyncs, renames, departures, `.m3u8` names, `D` deleting | `docs/rules/files-and-reruns.md` | touching `manifest.rs`, `scan`, renaming, `write_playlist`, `mark_departed` or purge |
| Cover art: embedding, capping and restoring covers, folder images, pacing artwork downloads | `docs/rules/cover-art.md` | touching cover fetching, `cap_cover`, `c` or `cover.jpg` |
| Folders earworm didn't download: adopted folders, `~` ids, `#hidden`, `D` menus, attaching a URL, `reconcile` | `docs/rules/folders-not-downloaded.md` | touching `contents`, `open_shelf`, adoption, hide/forget or attaching a URL |
| Albums and playlists: kind detection, shared sleeves, the pill and icons, the `a` kind filter | `docs/rules/albums-and-playlists.md` | touching `manifest::kind_of`, `tag::sleeve`, `Shelf.kind` or library icons |
| Songs on more than one playlist: copying known videos, twins, ISRC | `docs/rules/songs-on-more-than-one-playlist.md` | touching `copy_known`, `library_index`, `Track.twin` or `#isrc` |
| Custom playlists: the playlist store, ids and hints, the track picker, export, the in-library `.m3u8` | `docs/rules/custom-playlists.md` | touching `custom.rs`, `playlists.rs`, `picker.rs`, `export.rs` or `View::Playlist`/`View::Pick` |
| Splitting a video by its chapters: `settle_split`, `fetch_split`, chapter ids and spans | `docs/rules/chapter-splitting.md` | touching chapter splitting or `tag::cut` |
| Loudness tags: ReplayGain/R128, LRCLIB lookups, the breaker | `docs/rules/loudness-and-lyrics.md` | touching `write_loudness`, lyrics or `Breaker` |
| Playing through cliamp: importing playlists, `player.rs`, the `▶` marker's inputs | `docs/rules/cliamp.md` | touching `player.rs`, `p` or `space` |
| Audio format: formats and conversion, `convert`, config writes, `D` purge safety, `lookup::stop` | `docs/rules/audio-format.md` | touching `FORMATS`, `convert`, `transcode`, `save_key`/`save_format` or `Config::build` |
| What a key is allowed to do: Esc, `q`, `App.say`, the status bar and help rules, one meaning per letter, the settings screen | `docs/rules/keys-and-screens.md` | adding or changing a key, Esc behaviour, a status-bar hint, or a setting on the `,` screen |
| Cheat codes and achievements: `cheats.rs`, `achievements.rs`, the popup and its state file | `docs/rules/cheats-and-achievements.md` | touching either file or the `^z ^x` console |
| Questions, forms and quitting: `App.confirm`, `App.fields`, `Confirm` and quit questions, the header and estimate | `docs/rules/questions-and-forms.md` | touching prompts, forms, `Confirm` or the quit flow |
| Dependencies, `--check` and the update check: `DEPS`, `--check`, the GitHub probe | `docs/rules/dependencies-and-updates.md` | touching `deps.rs`, `update.rs` or `--check` |
| Themes and colour depth: `Palette`, `[themes.*]`, `[colors]`, `^t`, truecolor vs 256, `NO_COLOR` | `docs/rules/themes-and-colour.md` | touching `theme.rs`, `recolour`, theme config or any colour in `ui.rs` |
| Lists, narrowing and search: scrolling, flashes, the filter, review, the library order, `t` search, undo | `docs/rules/lists-and-search.md` | touching `view.rs`, `rows()`, `shelf_rows()`, `found_hits`, the filter box or undo |
| Artists and discovery: the artist index, stars, the artist page, getting albums and top tracks, new releases | `docs/rules/discovery.md` | touching `artists.rs`, `discover.rs`, `find_album`/`find_song`, `View::Artists`/`View::Artist` or `by_artist` |
| The desktop app: what the page may keep, the worker's start, the recorded messages, the palette as CSS | `docs/rules/desktop.md` | touching anything in `crates/gui` |
| Verbs, text, subprocesses and drawing: status verbs, the bell, column widths, `run_bounded`, the intro, the gradient | `docs/rules/ui-details.md` | touching text widths, subprocess calls, the intro or `draw_*` layout |

A rule that mentions "the albums section" or a similar section name means the
file for that area.

New rules go in the file for their area, with the test that pins them. A new
area gets a new file and a row above.

## Testing the TUI

**A skipped fixture test is a pass.** Dozens of tests print "no ffmpeg,
skipping" and return when ffmpeg cannot make their file, so on a machine
without the encoders the suite is green and proves nothing.
`the_encoders_the_fixture_tests_skip_without_are_all_there` fails there
instead, naming what is missing; `EARWORM_NO_FFMPEG=1` says the machine is
meant to be without.

Render into `TestBackend` and assert on the buffer, as the gradient,
status-bar and spinner tests do. Cheaper than a pty for anything reachable by
building an `App`.

For the paths that need a real process, drive a pty, but **never wait on screen
text.** ratatui diffs cells, so a word that changed emits only the characters
that differ: waiting for `finished` hangs while the screen shows `finishe` and
a `d` written elsewhere. A substring match also hits stale buffer contents from
an earlier frame. Wait on the filesystem instead, the `.m3u8` appearing or
`.earworm` gaining a line, and force a repaint with a resize when you must read
the screen.

**Rebuild the fixture for every pty run.** A run that edits tags changes the
folder it ran against, so a second run with the same keys starts from
different data and reports something else. Two runs that disagree are the
harness, not the code, until proven otherwise.

**Check the binary is newer than the source.** `cargo build` has reported
`Finished` here without recompiling while the source was newer, which means
verifying a fix against a binary that predates it.
`cargo clean -p earworm -p earworm-core` forces it.

**Break the fix and watch the test fail.** Three tests written this way passed
against the bug they described. Two specific traps: `ListState::select` with an
out-of-range index clamps, so a fixture whose right answer is the last row
passes either way, and the `▌` marker comes from `pos == cursor` rather than
from the list state, so it cannot catch a view-mapping bug at all.

ratatui installs its panic hook process-globally via `std::panic::set_hook`.

## Conventions

Comments explain why, not what. New comments are `//` lines, three at most;
existing `/* */` blocks stay until the code under them changes. Doc comments
`///` go on the declaration and carry the reasoning for anything non-obvious
below.
