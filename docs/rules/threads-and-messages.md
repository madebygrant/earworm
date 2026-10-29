# Threads and messages


- **A worker panic must not hang the UI.** The event loop treats
  `TryRecvError::Disconnected` as fatal, not as "no messages yet".
- **`Status` must be `settled()` for the run to finish.** A terminal status
  missing from that list hangs the UI forever, and `settled()` is an
  exhaustive match so a new variant fails to compile there. The tally beside
  the header used to be the other half of this trap, read off a hand-kept
  array that a new variant did not break: that is where `Skipped` was first
  missed, counted in the header's total and absent from the tally with nothing
  saying why. `counts()` is built from the tracks now and `tally()` is
  exhaustive too, so both ends fail to compile rather than going quiet.
- **`Msg::Quit` exists so cancelling the first URL prompt ends the tool.** The
  worker drops its sender straight after, so `main`'s disconnect branch has to
  check `app.quit` before deciding the worker died.
- **A question is an id, and a front end answers every one it takes.**
  `Msg::Ask` and `Msg::Pick` carry an id rather than a channel, because a
  channel cannot cross into a webview; `protocol::answer(id, reply)` is the
  way back. The registry holds the reply channel, so nothing on the UI side
  can drop it, and a question taken and never answered leaves the worker
  waiting where a dropped channel used to wake it with `Cancel`. Abandoning a
  question is answering `Reply::Cancel`; a front end that reloaded or is
  closing, and has lost the ids, calls `cancel_open_questions`
  (`a_question_is_answered_once_by_its_id`,
  `a_front_end_that_lost_its_ids_can_cancel_everything_open`, its own test
  binary because the call reaches every question in the process).
- **Cancel at the pick gate takes none, as Esc does.** The gate returning
  `None` means there was no gate, and the run downloads the whole playlist,
  so only a question nothing can answer any more returns it
  (`abandoning_the_pick_gate_downloads_nothing`).
- **The pick gate blocks the worker on a question only the UI can answer.**
  Its id lives in `App.picking`, so every path that takes it (`confirm_pick`,
  `cancel_pick`) must answer. Quitting mid-pick deliberately does not: `main`
  never joins the worker, so the blocked thread goes with the process.
  `handle_pick_key` sends no `Cmd`, or the work would queue behind a question
  nothing is going to answer.
- **Every message and command survives JSON.** The desktop front end sees
  only JSON, so `every_message_and_command_survives_json` round-trips one
  sample of each and checks the samples against the variants serde itself
  lists, read off its error for one that does not exist, so nothing is
  counted by hand. The derive refuses to compile a variant that cannot
  serialise; the test fails a variant with no sample.
- **A path goes out lossily.** serde_json refuses a path that is not UTF-8,
  so one oddly named file on Linux would sink a whole library message. The
  `lossy` serialisers in `protocol.rs` cost that row a replacement character
  instead (`a_path_that_is_not_utf8_still_goes_out`). Commands carry back
  what they were given, so a lossy path fails for its own row alone.
- **Thumbnails are made only when a front end asks.** `Cmd::Thumbs` names
  the files and `Msg::Thumb` answers each with a cached JPEG's path. The
  terminal never asks, so it never pays the ffmpeg run each new one costs,
  and the image bytes stay out of the message channel. They are named for
  the cover's bytes, so a tag write reuses one and an album's tracks share
  one, and a batch stops at `stop` or `cancel` rather than holding up the
  next command (`a_thumbnail_is_small_cached_and_follows_the_cover`).
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
