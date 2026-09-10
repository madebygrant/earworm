# earworm

Rust/ratatui TUI that downloads a YouTube playlist as tagged opus files. See
README.md for user-facing behaviour.

## Architecture

Two threads, one channel each way. This split is the point of the design: the
UI thread owns the terminal and nothing else, the worker owns every subprocess,
network call and file write. Nothing that blocks touches the render loop.

- `main.rs` — terminal setup, event loop, key handling, shutdown
- `app.rs` — `App` state, `Track`, `Status`, `Msg` (worker to UI), `Cmd` (UI to
  worker), `Asker` (worker asks the UI a question and blocks on the reply)
- `worker.rs` — `ask_url` then `pipeline` then `serve`, the post-run loop that
  handles edit and cover commands
- `ytdlp.rs` — `scan` (flat-playlist listing), `run` (the real download)
- `tag.rs` — lofty reads and writes, `identify` decides a track's status
- `lookup.rs` — AcoustID, Deezer, Cover Art Archive, all through one `ureq`
  agent
- `ui.rs` — every draw function
- `theme.rs` — the palette; background is deliberately never set, so the
  terminal's own background shows through

## Things that will bite you

- **Never `bail!` out of the pipeline for a download failure.** yt-dlp exits
  non-zero if any single entry fails, and aborting throws away the tags, cover
  and playlist for every track that succeeded. Carry it into the summary.
- **`Status` must be `settled()` for the run to finish.** Adding a terminal
  status without adding it there hangs the UI forever.
- **A worker panic must not hang the UI.** The event loop treats
  `TryRecvError::Disconnected` as a fatal error, not as "no messages yet".
- **Byte indexing over a lowercased copy of a string is wrong.**
  `to_lowercase()` doesn't preserve byte length (U+212A shrinks, U+0130 grows).
  Work in chars.
- **`Asker::input` distinguishes empty from cancelled.** `None` is Esc; an
  empty string is a real answer. Collapsing them silently discards edits.
- **`std::process::Command` has no timeout.** `lookup::run_bounded` polls
  `try_wait` and kills at a deadline. Use it for anything external.
- **ureq's global timeout defaults to `None`.** The shared agent in
  `lookup::agent()` sets it. Don't build a bare `Agent`.
- **Never embed an AcoustID key.** It comes from `ACOUSTID_API_KEY` only.

## Testing the TUI

ratatui diffs cells, so a pty capture of a changed line emits only the changed
run of characters. Asserting on screen text needs either a forced repaint
(resize) or a replay of the output into a grid. Waiting on a substring is also
unreliable, since it matches stale buffer contents from an earlier frame.

ratatui installs its panic hook process-globally via `std::panic::set_hook`.

## Conventions

Comments explain why, not what. Multi-line comments use `/* */`. Doc comments
`///` on the declaration, and they carry the reasoning for anything non-obvious
in the function below.
