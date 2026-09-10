# earworm

Rust/ratatui TUI that downloads a YouTube playlist as tagged opus files. See
README.md for user-facing behaviour.

## Architecture

Two threads, one channel each way, and that split is the design. The UI thread
owns the terminal and nothing else; the worker owns every subprocess, network
call and file write. Nothing that blocks touches the render loop.

- `main.rs` — terminal setup, event loop, key handling, shutdown
- `app.rs` — `App` state, `Track`, `Status`, `Shelf` (one library row), `View`
  (which screen has the keys), `Msg` (worker to UI), `Cmd` (UI to worker),
  `Asker` (worker asks the UI a question and blocks on the reply), `Escape`
  (what Esc on a prompt does), the `marked` set behind `targets()`, and the
  `filter` behind `rows()`
- `config.rs` — `Cli`, the TOML `FileConfig`, and `Config::build`, which merges
  them
- `worker.rs` — `ask_start` (library, resync or the URL prompt, which loops
  until the URL validates) then `pipeline` then `serve`, which serves both the
  post-run commands and the library screen. `tag_tracks` is the
  identify-and-write pass, shared by the run and by retry; `open_shelf` builds
  the same rows from the manifest and the tags on disk, with no network
- `ytdlp.rs` — `scan` (flat-playlist listing), `run` (the real download)
- `manifest.rs` — the `.earworm` sidecar: `#url` and `#synced` headers plus
  video id to current filename
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
- **`Msg::Quit` exists so cancelling the first URL prompt ends the tool.** The
  worker drops its sender straight after, so `main`'s disconnect branch has to
  check `app.quit` before deciding the worker died.
- **`Asker::input` distinguishes empty from cancelled.** `None` is Esc; an
  empty string is a real answer. Collapsing them silently discards edits.
- **`App.busy` is set by `send`, not by a message from the worker.** The gap
  between a keypress and the worker reporting it started is long enough for a
  second press to queue a duplicate, and a duplicated swap silently undoes the
  first. `serve` sends `Msg::Idle` last, after everything else for that
  command.
- **`Msg::Flash` is a stage message that expires; `Msg::Stage` is one that
  stays.** Post-run outcomes flash, and `main` calls `expire_flash` every
  frame because whatever set the message has already finished. A `Stage` sent
  after `done` is set deliberately does not become the resting message, or
  cancelling a command would revert the header to a step already over.
- **The filter is a predicate, never a rebuild of `tracks`.** `cursor` stays a
  position in `tracks` and `marked` stays a set of track indices, so no
  command means anything different under a filter. `rows()` is the view and
  `row_of_cursor(&rows)` maps the cursor onto it, taking the view rather than
  rebuilding it so the highlight and the pane cannot answer two different
  questions; `shown()` counts without collecting, for the header. Note that
  the `▌` marker comes from `pos == cursor` rather than from `ListState`, so
  the marker cannot drift from what a command acts on: only the scroll offset
  can, which is what the filter's scroll test pins down.
- **`selected()` returns `None` for a filtered-out cursor,** so `e` and `c`
  cannot reach a row that is off screen, and `snap()` puts the cursor back on
  a visible row after every change to the filter.
- **`apply` snaps after every message, not just after a keypress.** `Update`
  carries a new name and it and `Progress` change the status word, so the
  worker can filter the cursor's own track out from under it: editing the
  track you are on does exactly that. Without the snap no `▌` is drawn
  anywhere and `e`, `c` and `space` go quiet until the next `j`.
- **`App.marked` holds track indices, not row positions.** A `Cmd` carrying
  rows would mean something different to the worker than it does on screen.
- **Moving the cursor by hand clears `follow`, `g` and `G` included.** A jump
  that keeps following reads as a broken key: the next progress message drags
  the cursor straight back to whatever is downloading.
- **`mark_like_cursor` stops at the filter.** `/` then `m` is how you mark one
  artist's bad rows, and reaching past the filter marks the whole run instead.
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
- **An absence proves nothing unless the scan asked for everything and got
  it.** `Listing::complete` covers a truncated listing; `cfg.extra` covers a
  narrowed one, because `--playlist-items 4` makes every other track absent
  while yt-dlp still exits 0. Which arguments filter is not knowable, so any
  of them silences `note_departures`. Anything that later deletes rather than
  reports must keep both halves.
- **`finish` holds the run's `Result` and only clones it.** `main` reports the
  exit status from the last `Msg::Done`, so rebuilding the result from the text
  on screen turned a failed run into a successful one and `$?` came back 0.
- **A `Prompt::Choice` header is a `Block::title`, which clips.** Anything
  long enough to need wrapping goes in `note`, rendered above the options.
- **`finish` returning false must skip `serve`.** It is the one answer that
  means quit, and falling through would leave the worker waiting on a command
  channel the UI has already stopped feeding.
- **`Msg::Restart` exists because a second playlist reuses the session.**
  Clearing tracks alone leaves `done` set, so the header would say finished
  while the next run was going and the post-run keys would stay live.
- **`--resync` runs one pipeline per folder, never one merged list.**
  `Track::index` is a position within its own playlist, so two playlists both
  have a track 1 and merging them makes every row, mark and command ambiguous.
- **The `#` headers must stay invisible to the entry parser.** `read` skips
  any key starting with `#`, and a manifest written before a header existed
  has to keep loading, or every folder downloaded so far re-downloads.
- **`write` keeps the recorded sync time and `write_synced` moves it.** A tag
  edit rewrites the entries without having met the playlist, and stamping it
  would make the library claim a sync that never happened.
- **`write_manifest` preserves entries the current tracks do not cover** when
  the file is still on disk. A pass over part of a playlist (`--playlist-items`,
  a retry, a folder opened from the library) would otherwise rewrite the whole
  manifest from that part, forgetting files that `scan` cannot then recognise
  by name, so the next sync downloads them again beside the copies on disk.
  Entries whose file has gone are dropped, so a churning folder does not
  collect dead lines for good.
- **`open_shelf` numbers tracks from the filename, not the row position.**
  Every file is written with its playlist number in front, and a folder with a
  gap in it would otherwise renumber everything after the gap the next time one
  of those tracks was edited. Two files can still carry the same number, since
  a departure renumbers the tracks after it and the departed file keeps its
  name: every command finds its track by index and takes the first match, so
  sharing one makes a row write tags to another row's file. Tracks the playlist
  still holds claim their number first and everything else is numbered past
  them.
- **The `.m3u8` names tracks by filename, never by full path.** It lives in
  the folder it lists, so a relative name resolves wherever the folder is
  copied to; an absolute one is valid only on the machine that wrote it, and
  the whole playlist breaks the moment it reaches a phone or another user's
  home. `last_playlist` matches on the filename for the same reason.
- **`mark_departed` reads the folder's `.m3u8` to decide what is still in the
  playlist,** because offline there is no listing to ask and that file is the
  last sync's answer already written down. It sets `Status::Gone` rather than
  just `listed = false`: `write_track` and `rename` are keyed on `Gone`, and
  without it the first edit re-lists the track and renames it to a playlist
  position it does not hold. A playlist file that names nothing on disk is not
  believed, since emptying a correct `.m3u8` is the costlier guess.
- **`write_track` keeps a departed track departed.** Overwriting the status
  with `manual` disarms the two guards it just consulted, so a second edit of
  the same row renames and re-lists it.
- **`Msg::Done` carries the header word for a success.** Opening a folder from
  the library settles without running anything, and "finished" would be a claim
  about work that never happened. A failure always reads `failed`, so only the
  success word varies.
- **`manifest::load` parses the whole sidecar in one read,** because `library`
  wants the URL, the sync time and the entries for every folder under `--dir`.
- **`Msg::Library` carries `show`,** because the same message refreshes the
  counts after a sync. Showing the library then would swap the screen out from
  under the run the user is watching.
- **`can_command` and `can_browse` are exclusive on `View`.** The two screens
  move different cursors, so a key live on the wrong one acts on a row that is
  not there.
- **`save_manifest` is keyed on the file, not on `listed`.** A departed track
  is unlisted but must stay in `.earworm`, or a video returning to the
  playlist downloads again beside the copy on disk.
- **`Status::Gone` rows are numbered past the playlist** so their indices
  cannot collide with a real track's, and `tag_tracks` skips them so a
  departed file is never re-tagged. That index is synthetic, so `write_track`
  does not rename a departed track either: it would stamp a playlist position
  the track no longer holds onto the file, next to the real holder.
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

**Rebuild the fixture for every pty run.** A run that edits tags changes the
folder it ran against, so a second run with the same keys starts from
different data and reports something else. Two runs that disagree are the
harness, not the code, until proven otherwise.

**Check the binary is newer than the source.** `cargo build` has reported
`Finished` here without recompiling while `src/` was newer, which means
verifying a fix against a binary that predates it. `cargo clean -p earworm`
forces it.

**Break the fix and watch the test fail.** Three tests written this way passed
against the bug they described. Two specific traps: `ListState::select` with an
out-of-range index clamps, so a fixture whose right answer is the last row
passes either way, and the `▌` marker comes from `pos == cursor` rather than
from the list state, so it cannot catch a view-mapping bug at all.

ratatui installs its panic hook process-globally via `std::panic::set_hook`.

## Conventions

Comments explain why, not what. Multi-line comments use `/* */`. Doc comments
`///` go on the declaration and carry the reasoning for anything non-obvious
below.
