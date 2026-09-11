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
  (what Esc on a prompt does), `picking` (the reply channel for the pick gate),
  the `marked` set behind `targets()`, and the `filter` behind `rows()`
- `config.rs` — `Cli`, the TOML `FileConfig`, and `Config::build`, which merges
  them
- `worker.rs` — `ask_start` (library, resync or the URL prompt, which loops
  until the URL validates) then `pipeline` then `serve`, which serves both the
  post-run commands and the library screen. `tag_tracks` is the
  identify-and-write pass, shared by the run and by retry; `ask_which` is the
  pick gate between the scan and the download; `open_shelf` builds the same
  rows from the manifest and the tags on disk, with no network
- `ytdlp.rs` — `scan` (flat-playlist listing), `run` (the real download)
- `manifest.rs` — the `.earworm` sidecar: `#url` and `#synced` headers plus
  video id to current filename
- `tag.rs` — lofty reads and writes, `identify` decides a track's status
- `lookup.rs` — AcoustID, Deezer, Cover Art Archive, all through one `ureq`
  agent
- `player.rs` — the optional cliamp handoff: import the `.m3u8`, then `load`
- `ui.rs` — every draw function
- `theme.rs` — the palette, plus `background()`, the diagonal gradient painted
  behind every frame

## Things that will bite you

### Threads and messages

- **A worker panic must not hang the UI.** The event loop treats
  `TryRecvError::Disconnected` as fatal, not as "no messages yet".
- **`Status` must be `settled()` for the run to finish.** A terminal status
  missing from that list hangs the UI forever. `settled()` is an exhaustive
  match, so a new variant fails to compile there; `App::counts()` is a
  hand-kept array and does not, which is exactly where `Skipped` was first
  missed. A settled status absent from `counts()` is counted in the header's
  total and left out of the tally beside it, and the two disagree with nothing
  saying why.
- **`Msg::Quit` exists so cancelling the first URL prompt ends the tool.** The
  worker drops its sender straight after, so `main`'s disconnect branch has to
  check `app.quit` before deciding the worker died.
- **The pick gate blocks the worker on a channel only the UI can answer.**
  The reply `Sender` lives in `App.picking` and nothing else holds a clone, so
  every path that takes it (`confirm_pick`, `cancel_pick`) must send. Quitting
  mid-pick deliberately does not: `main` never joins the worker, so the blocked
  thread goes with the process. `handle_pick_key` sends no `Cmd`, or the work
  would queue behind a question nothing is going to answer.
- **`confirm_pick` reads `marked`, never `targets()`.** `targets()` falls back
  to the cursor when nothing is marked, which here turns "I want none of these"
  into "download the row I happen to be on".
- **A skipped track stays in `tracks`.** `write_playlist` builds the `.m3u8`
  from the tracks it is handed, so dropping the unpicked ones would rewrite a
  whole playlist file from the handful this run fetched. `Status::Skipped` is
  what keeps them present and untouched.
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
- **`pipeline` takes `gate` rather than reading `cfg.pick`, and `resync` passes
  `false`.** The gate is on by default, so a question per folder is the one
  thing that would stop an unattended walk of the whole library: the user asked
  for the sync, not for each playlist in it. Nothing tests this. It is a single
  literal at one call site, and switching it to `cfg.pick` still compiles and
  still passes the suite, so treat that line as load-bearing.
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
- **`Skipped` is the one status the archive reads.** It is how a gated run
  tells yt-dlp not to fetch a track, and it has to work with no file behind it.
  `--playlist-items` would do the same job and silence `note_departures`, since
  narrowing the listing makes every other track absent; narrowing only the
  download leaves `Listing::complete` meaning what it says.
- **`tag_tracks` skips `Gone` and `Skipped` together.** Neither has a file this
  run may touch, and falling through marks both `Failed` for having none, which
  reports a deliberate choice as something that went wrong.
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

### Playing through cliamp

- **`cliamp playlist import` refuses a name it already holds,** exit 1, without
  replacing or duplicating. So a refresh is delete-then-import, and the delete
  failing is the ordinary first-time case rather than an error worth reporting.
- **The stored name is `earworm - <folder> (<tag>)`, never the bare folder
  name.** That delete is unconditional, so a bare name would destroy a
  same-named playlist the user built in cliamp by hand, and cliamp's store is
  flat, so two `--dir` roots holding a `Focus` would otherwise share one entry.
  An ASCII colon is rejected by cliamp as an invalid playlist name, which is
  why the separator is a dash.
- **`tag` is FNV-1a written out, not `DefaultHasher`,** whose output is
  explicitly not stable between Rust releases. A changed hash orphans every
  playlist already in cliamp under the old name, so the value is pinned in a
  test against an independent implementation.
- **Two timeouts, not one.** `load` and `--version` reach the socket and answer
  at once or never; importing reads the tags off every file and is genuinely
  slow, and killing it part-way would leave cliamp's store torn.
- **`player::installed()` is cached and `player::running()` must not be.** What
  is on PATH cannot change within a session; whether cliamp is up changes from
  another window while earworm watches, including while the finish menu sits
  open, which is why that menu re-probes on every pass.
- **The finish row and the `p` key answer the same question.** Both require
  `running`, not merely installed. They drifted apart once because `finish`
  runs before `serve` and so cannot read `App.player`, and the result was the
  same action hidden on one screen and offered on the next.
- **`space` is library-only.** On the track list it marks a row, and that is
  what `targets()` and every bulk command read, so the transport key had to go
  where `space` was still free. That is also the screen showing what cliamp is
  doing.
- **The stub tests set their own timeouts.** macOS charges for the first exec
  of a newly written executable, which is what every stub is, and borrowing
  cliamp's 2s `IPC` bound made them fail about one run in three. Those tests
  check argv and control flow, never latency.
- **cliamp names the track, never the playlist.** `status --json` carries
  `track.path` and no playlist name at all, so the library row is matched by
  the folder that path sits in. A radio stream's path is a URL, which is why
  only a leading slash counts as a folder.
- **`status --json` is parsed, not exit-code checked.** A stopped cliamp prints
  its "not running" line instead of JSON, so a failed parse is the same answer
  and one probe covers both whether it is up and what it is playing.
- **`serve` polls cliamp between commands rather than on a thread of its own.**
  `recv_timeout` is what makes that free, and `serve` idling is exactly when
  `p` is live: during a run the keys are dead, so a fresher answer would be one
  nobody could act on. `Watch` is what keeps that to one `Msg::Player` per
  change rather than one every three seconds for the life of the session.
- **A key that can only fail is worse than no key.** `p` is drawn and handled
  only while `can_play()`, and the status bar carries `♪ cliamp off` so its
  disappearance has a reason on screen rather than looking like a bug.
- **`cliamp load` needs a running instance** and exits 1 with "cliamp is not
  running" otherwise. That is not a failure: the import has already happened,
  so `player::load` reports where the tracks landed instead. earworm never
  launches cliamp, because cliamp is a TUI and starting one would mean handing
  over the terminal earworm owns.
- **The `.m3u8` is the input, never the folder.** It is earworm's own answer
  about what the playlist holds and in what order, departures already left out;
  `playlist create` over the directory would lose the order and sweep up
  `cover.jpg`.
- **`finish`'s menu rows carry their answers.** The cliamp row is only there
  when cliamp is, so a bare index would mean a different thing depending on
  what is installed.

### Audio format

- **A format name is not an extension.** `vorbis` writes `.ogg` and `alac`
  writes `.m4a`, so `FORMATS` carries both. `run` hands the name to
  `--audio-format`, `scan` predicts filenames with the extension, and
  `scan_template` is what stops the two drifting apart. A scan predicting the
  wrong extension makes every track look absent and re-downloads a folder that
  is already complete.
- **`Config::build` checks the format rather than leaving it to yt-dlp.** An
  unknown name gets as far as the download, by which time the scan has already
  predicted filenames nothing is going to write.
- **wav and aac are left out on purpose.** Neither carries tags or cover art,
  so earworm would do a third of its job and report it as finished.
- **`with_tag` takes the tag type from the container.** lofty refuses a tag
  type the format cannot hold, so assuming Vorbis comments left every untagged
  mp3 and m4a failing with "no writable tag". ffmpeg stamps a tag of its own on
  almost anything, so the test that covers this asks for an mp3 with
  `-id3v2_version 0 -write_id3v1 0` and asserts at least one fixture arrived
  untagged.
- **Changing the format converts nothing.** The manifest records each track by
  the filename it has, so those tracks stay downloaded and stay as they are,
  and a folder switched part-way holds both. Re-fetching them would be a far
  more expensive guess than leaving them alone.
- **`save_format` edits one line and re-parses before writing.** The config is
  hand-edited, so rebuilding it from the parsed struct would drop every
  comment. A file earworm can no longer read is worse than one that never
  recorded the choice, since the next start fails outright.
- **`f` is library-only, like `space`.** It follows the active track on the
  track list, and the setting is about the next playlist rather than the one on
  screen. `--no-config` leaves `Config.config_file` as `None`, and the flash
  then says the choice lasts one session.
- **`Msg::Settings` exists because `App.settings` is built once at startup.**
  Change a setting from inside the tool without re-sending it and the help
  overlay keeps reporting the format the run began with.

### Text, subprocesses and keys

- **Byte indexing over a lowercased copy of a string is wrong.**
  `to_lowercase()` doesn't preserve byte length (U+212A shrinks, U+0130 grows).
  Work in chars.
- **`youtube_url` parses the authority, not the whole string.** A link passes
  only if the host itself is one of `HOSTS`, or `evil.com/youtube.com/watch?v=x`
  would.
- **`std::process::Command` has no timeout.** `lookup::run_bounded` polls
  `try_wait` and kills at a deadline. Use it for anything external. It does not
  set the stdio: the caller does, because a piped stream nobody drains fills
  its buffer and blocks the child forever.
- **ureq's global timeout defaults to `None`.** The shared agent in
  `lookup::agent()` sets it. Don't build a bare `Agent`.
- **Never embed an AcoustID key.** It comes from `ACOUSTID_API_KEY` only.

### Drawing and config

- **A mode band deliberately punches through the gradient, and must fill the
  row.** `draw_band` backs the filter and the pick. `Paragraph` styles only the
  cells it writes, so the padding is what covers the gradient: pad to the full
  width and drop the right-hand hint when both halves will not fit. A band that
  stops mid-row reads as a rendering fault, where a missing hint reads as a
  narrow terminal. The pick band replaces the filter band, so it carries the
  filter text too, or `a` acts on a narrowed list with nothing saying so.
- **The library preview draws from `Shelf.files`, never from the disk.**
  Presence is resolved in `worker::library`, in the stat pass that already
  counts `missing`, because the draw loop must not stat and a tag read per
  cursor move would need a worker round trip. Filenames carry the number and
  tags already, so the pane needs no tag read at all.
- **One derivation of the selected shelf.** `draw_library` binds `selected`
  once and hands it to the pane, the `ListState`, the `▌` and the scrollbar.
  Recomputing the clamp per widget is how the highlight and the pane come to
  answer two different questions.
- **The intro runs off `started.elapsed()`, never `tick`.** A tick is whatever
  the poll timeout and the message traffic make it, so an animation keyed to it
  speeds up exactly when the worker gets busy. `main` shortens the poll to 33ms
  only while `app.intro()`, and nothing else on screen moves between messages.
- **`App::intro()` holds over the library but never over a prompt.** The
  library is ready in milliseconds on a bare start, so yielding to it left the
  intro unseen by anyone who runs `earworm` alone, and nothing is waiting on
  the user behind a list. A prompt is the opposite: the worker is blocked on an
  answer, and a question nobody can see is not polish. Tracks, a finished run
  and the help modal end it too. Those conditions are also what keeps the intro
  out of the other draw tests, so loosening one breaks them somewhere
  unrelated; the library tests now set `intro_done` to say so out loud.
- **The intro swallows the key that skips it.** Skipping an animation must not
  double as a command aimed at a screen the user never saw.
- **`draw_background` must run first and stay first.** Every other widget
  styles only its foreground, which is what lets the gradient survive
  underneath. A widget that sets a background punches a hole, and `Clear` under
  a popup resets to the terminal's colour, which is why `popup` repaints
  `SURFACE`.
- **A `bool` clap flag can't tell "off" from "unset".** `Config::build` takes
  the raw `ArgMatches` and checks `value_source`, which is why `main` goes
  through `Cli::command().get_matches()` rather than `Cli::parse()`. A flag
  with no matching config key silently makes the file unable to set it. Every
  flag is a negation and goes through `off`, so the command line can only
  switch a feature off and the file is the only way to hold one on.

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
