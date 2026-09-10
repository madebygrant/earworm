# earworm

Rust/ratatui TUI that downloads a YouTube playlist as tagged opus files. See
README.md for user-facing behaviour.

## Architecture

Two threads, one channel each way, and that split is the design. The UI thread
owns the terminal and nothing else; the worker owns every subprocess, network
call and file write. Nothing that blocks touches the render loop.

- `main.rs` — terminal setup, event loop, key handling, shutdown
- `app.rs` — `App` state, `Track`, `Status`, `Msg` (worker to UI), `Cmd` (UI to
  worker), `Asker` (worker asks the UI a question and blocks on the reply),
  `Escape` (whether Esc on a prompt skips or quits), plus the `marked` set
  behind `targets()`
- `config.rs` — `Cli`, the TOML `FileConfig`, and `Config::build`, which merges
  them
- `worker.rs` — `ask_url` (loops until the URL validates) then `pipeline` then
  `serve`, the post-run command loop. `tag_tracks` is the identify-and-write
  pass, shared by the run and by retry
- `ytdlp.rs` — `scan` (flat-playlist listing), `run` (the real download)
- `manifest.rs` — the `.earworm` sidecar mapping video id to current filename
- `tag.rs` — lofty reads and writes, `identify` decides a track's status
- `lookup.rs` — AcoustID, Deezer, Cover Art Archive, all through one `ureq`
  agent
- `ui.rs` — every draw function
- `theme.rs` — the palette, plus `background()`, the gradient painted behind
  every frame

## Things that will bite you

### Threads and messages

- **A worker panic must not hang the UI.** The event loop treats
  `TryRecvError::Disconnected` as fatal, not as "no messages yet".
- **`Status` must be `settled()` for the run to finish.** A terminal status
  missing from that list hangs the UI forever.
- **`Msg::Quit` exists so cancelling the URL prompt ends the tool.** The
  worker drops its sender straight after, so `main`'s disconnect branch has to
  check `app.quit` before deciding the worker died.
- **`Asker::input` distinguishes empty from cancelled.** `None` is Esc; an
  empty string is a real answer. Collapsing them silently discards edits.
- **`App.busy` is set by `send`, not by a message from the worker.** The gap
  between a keypress and the worker reporting it started is long enough for a
  second press to queue a duplicate, and a duplicated swap silently undoes the
  first. `serve` sends `Msg::Idle` last, after everything else for that
  command.
- **`App.marked` holds track indices, not row positions.** A `Cmd` carrying
  rows would mean something different to the worker than it does on screen.
- **A bulk action clears the marks with `Msg::Unmark` on completion,** not when
  the key is pressed, and only when something was written. Clearing on dispatch
  throws away a twenty-track selection the moment someone escapes the prompt.

### Files and re-runs

- **Never `bail!` out of the pipeline for a download failure.** yt-dlp exits
  non-zero if any single entry fails, and aborting throws away the tags, cover
  and playlist for every track that succeeded. Carry it into the summary.
- **Renaming and `scan` are coupled through `manifest.rs`.** `scan` decides a
  track is already downloaded by predicting yt-dlp's filename, which a rename
  invalidates. The sidecar closes that gap, so anything that moves a file must
  call `save_manifest`, or the next run re-downloads it and leaves a duplicate.
- **Only `Ok` and `Manual` tracks may be renamed.** Every other status is a
  guess, and a filename makes a guess look settled.
- **The yt-dlp archive is keyed on the file existing, not on a status.** By
  retry time the tracks that worked are `ok` or `manual`, so keying on
  `Status::Have` would leave them out, and yt-dlp remuxing them fails the whole
  run.
- **`is_file`, never `exists`, for a track's path.** A directory sitting where
  the audio should go satisfies `exists`, and `scan` then calls the track
  downloaded, so nothing fetches it or marks it failed.
- **`ytdlp::run` can run twice in one process.** Its scratch directory carries
  a counter as well as the pid. The `Download` guard removes the directory on
  drop and would take the other run's archive with it.
- **Bulk and single tag changes both go through `write_track`,** so the row,
  the filename and the playlist name cannot drift apart.
- **`summarise` feeds both the run and a retry.** `main` prints that summary on
  exit, so a retry building its own message drops the folder path.

### Text, subprocesses and keys

- **Byte indexing over a lowercased copy of a string is wrong.**
  `to_lowercase()` doesn't preserve byte length (U+212A shrinks, U+0130 grows).
  Work in chars.
- **`youtube_url` parses the authority, not the whole string.** A link passes
  only if the host itself is one of `HOSTS`, or `evil.com/youtube.com/watch?v=x`
  would.
- **`std::process::Command` has no timeout.** `lookup::run_bounded` polls
  `try_wait` and kills at a deadline. Use it for anything external.
- **ureq's global timeout defaults to `None`.** The shared agent in
  `lookup::agent()` sets it. Don't build a bare `Agent`.
- **Never embed an AcoustID key.** It comes from `ACOUSTID_API_KEY` only.

### Drawing and config

- **`draw_background` must run first and stay first.** Every other widget
  styles only its foreground, which is what lets the gradient survive
  underneath. A widget that sets a background punches a hole, and `Clear` under
  a popup resets to the terminal's colour, which is why `popup` repaints
  `SURFACE`.
- **A `bool` clap flag can't tell "off" from "unset".** `Config::build` takes
  the raw `ArgMatches` and checks `value_source`, which is why `main` goes
  through `Cli::command().get_matches()` rather than `Cli::parse()`. A flag
  with no matching config key silently makes the file unable to set it.

## Testing the TUI

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

ratatui installs its panic hook process-globally via `std::panic::set_hook`.

## Conventions

Comments explain why, not what. Multi-line comments use `/* */`. Doc comments
`///` go on the declaration and carry the reasoning for anything non-obvious
below.
