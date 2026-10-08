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
