# earworm

Rust/ratatui TUI that downloads a YouTube playlist as tagged opus files. See
README.md for user-facing behaviour.

## Architecture

Two threads, one channel each way. This split is the point of the design: the
UI thread owns the terminal and nothing else, the worker owns every subprocess,
network call and file write. Nothing that blocks touches the render loop.

- `main.rs` — terminal setup, event loop, key handling, shutdown
- `app.rs` — `App` state, `Track`, `Status`, `Msg` (worker to UI), `Cmd` (UI to
  worker), `Asker` (worker asks the UI a question and blocks on the reply),
  `Escape` (whether Esc on a prompt skips or quits)
- `config.rs` — `Cli`, the TOML `FileConfig`, and `Config::build`, which merges
  them
- `worker.rs` — `ask_url` (loops until the URL validates) then `pipeline` then
  `serve`, the post-run loop that handles edit and cover commands
- `ytdlp.rs` — `scan` (flat-playlist listing), `run` (the real download)
- `tag.rs` — lofty reads and writes, `identify` decides a track's status
- `lookup.rs` — AcoustID, Deezer, Cover Art Archive, all through one `ureq`
  agent
- `ui.rs` — every draw function
- `theme.rs` — the palette, plus `background()`, the charcoal-to-black vertical
  gradient painted behind every frame

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
- **`Msg::Quit` exists so cancelling the URL prompt ends the tool.** The
  worker drops its sender straight after, so `main`'s disconnect branch has to
  check `app.quit` before deciding the worker died.
- **`youtube_url` parses the authority, not the whole string.** A link only
  passes if the host itself is one of `HOSTS`, or
  `evil.com/youtube.com/watch?v=x` would.
- **`std::process::Command` has no timeout.** `lookup::run_bounded` polls
  `try_wait` and kills at a deadline. Use it for anything external.
- **ureq's global timeout defaults to `None`.** The shared agent in
  `lookup::agent()` sets it. Don't build a bare `Agent`.
- **A `bool` clap flag can't tell "off" from "unset".** `Config::build` takes
  the raw `ArgMatches` and checks `value_source`, which is why `main` parses
  through `Cli::command().get_matches()` rather than `Cli::parse()`. Adding a
  flag without a matching config key silently makes the file unable to set it.
- **`draw_background` must run first and stay first.** Every other widget
  styles only its foreground, which is what lets the gradient survive
  underneath. A widget that sets a background punches a hole, and `Clear`
  under a popup resets to the terminal's own colour, which is why `popup`
  repaints with `SURFACE`.
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
