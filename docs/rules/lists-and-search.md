# Lists, narrowing and search

## Lists and messages

- **`ListState` is built fresh every frame, so the offset has to be ours.**
  Left to itself ratatui scrolls the least it can to make the selection
  visible, which pins the cursor to the last row for the whole of a long list.
  `scroll_to` keeps `SCROLLOFF` rows of context and only moves when the cursor
  comes within that of an edge; `App.scroll` and `App.shelf_scroll` carry it
  between frames.
- **A flash is timed by its length and a second one queues.** One fixed four
  seconds fitted "saved track 12" and not a wrapped cliamp error, and a bulk
  action's outcomes used to arrive and leave inside a single blink with only
  the last readable. `QUEUE` is three deep, and `Msg::Stage` clears it: the
  run talking outranks a backlog of outcomes from before it started.
- **`wrap` keeps the caller's leading spaces on the first line.** Splitting on
  spaces ate them, which left every unselected option in a menu two columns
  left of the one carrying the marker.
- **A digit answers a choice only when that option is there.** The finish
  menu's last row is quit, so an unbounded digit is one keypress from ending
  the session by accident.
- **earworm refuses a terminal it cannot draw in.** Both ends are checked:
  crossterm's raw mode otherwise fails with an OS error naming a device rather
  than saying `--list` and `--check` work without one.

## Narrowing, ordering and undoing

- **The review is a predicate beside the filter, not a string in the box.**
  `kept`, `unsure`, `no match` and `failed` share no word to type, so
  `Status::wants_a_look` is what `v`, `n`/`N` and the finish menu's first row
  all read. `keeps()` is where the two narrowings meet, and `narrowed()` is
  what every key that clears them asks; clearing one without the other leaves
  a band saying the list is short with nothing to turn off.
- **`n` and `N` do not wrap.** A jump that quietly starts again at the top
  reads as a key that did nothing. Reaching the end says which end it was.
- **The library order is a view, never a sort of `library` itself.** `shelf`
  is a position in the vector the worker built, so re-sorting that vector
  would move the folder out from under the cursor every time a sync changed a
  count. `shelf_rows()` is the order and the narrowing together, and every
  library key walks it, exactly as the track keys walk `rows()`.
- **One filter box, two screens.** `snap()` puts both cursors back on a row
  that is showing, because a message can change either list. The band asks
  the view which counts to print.
- **Every screen's keys yield to the filter box.** `handle_key` checked
  `View::Library` before `typing_filter`, so `/` on the library raised the
  band and then sent every letter to the library's own keys: typing "no"
  cycled the sort and `e` opened a rename. The box could be opened and never
  filled. The `Found` and `Library` branches both carry `!app.typing_filter`
  now, which is what the track list always had by sitting after it.
- **`t` is library-only, like `space` and `f`, and the track list names the
  route instead.** The search is about the library rather than the playlist on
  screen, and `T` on the track list already means "look this one up again":
  two keys a letter apart meaning different searches is the `O`/`o` problem
  again. So the help there carries an `Esc t` row rather than a key, and only
  when there is a library behind the run for it to reach.
- **`View::Found` is the library's filter asked of the tracks.** The rows say
  which folders hold a match and the preview says which tracks, one folder at
  a time and only above `PREVIEW_FROM`; the search screen is the whole library
  at once, which is the case neither of those covers. Same box, same query:
  `t` switches between them. The matchers differ on purpose: the library rows
  and the track list are a substring narrowing of a list already on screen, and
  `t` is the picker's fuzzy matcher (`picker::find`), so a query like `autbn`
  can show no folders and still find the track on `t`. It keeps missing files,
  which the picker leaves out because they cannot be added, and lists them
  with a `!`; `found_hits` is what the header, the band and the draw all ask, so
  they cannot answer three different questions
  (`the_search_screen_is_fuzzy_ranked_and_matches_the_folder_like_the_picker`,
  `the_search_screen_underlines_what_matched`).
- **The search screen marks tracks with Tab and `P` adds them.** `found_marks`
  is the picker's `Marks` and the add goes through the same chooser, so nothing
  about the worker differs. The marks clear when the add succeeds
  (`Msg::FoundUnmark`, sent from `append` only when something was written) and
  not when `P` is pressed, for the reason a bulk action clears on completion;
  a new search clears them too. A file that is gone is not markable, since the
  add would only fail on it. `from_found` is set by Enter on a result so `Esc`
  from that folder comes back to the results; the library's and the list's
  open paths clear it
  (`tab_marks_search_results_and_p_adds_them_in_the_order_marked`,
  `a_file_that_is_gone_is_not_marked`,
  `esc_from_a_folder_opened_from_a_result_goes_back_to_the_results`).
- **One or two letters are a substring match, in the picker and on `t`.** As a
  fuzzy query they match almost any title and the list stops meaning anything;
  from three letters on, gaps are allowed
  (`one_or_two_letters_must_appear_as_typed_where_three_may_have_gaps`). The
  library's empty pane says when `t` would find tracks that no folder shows
  (`a_library_with_no_matching_folder_points_at_t_when_it_would_find_tracks`). `draw` computes it once into `App.found_cache`
  and the header, band and list read that, and the list builds only the visible
  rows. A call is about 7 ms at 5,000 files in release, which is the number to
  watch if a library grows past that.
- **The search cursor is the `(shelf, file)` pair, never a row number.**
  `found_rows` is recomputed from the filter like `rows()` and `shelf_rows()`,
  so a sync rebuilding the library, or a keystroke in the box rebuilding the
  list, would leave a row number pointing at a different track. `found_snap`
  falls to the first result rather than the nearest: editing the query is a
  new question, not a move within the old answer.
- **An empty query is no results, not every track.** A search screen listing
  the whole library is the library screen, which is where `t` was pressed.
  `find_tracks` opens the box instead, and refuses to leave the library at
  all when the query matches nothing: a screen of nothing with the box already
  closed is a dead end.
- **`Cmd::Open` carries the filename to land on, and the worker resolves it.**
  `open_shelf` numbers tracks from the filename and numbers departures past
  the playlist, so the index a search result would compute is not the one the
  row ends up with. `Msg::Focus` carries the answer back and clears `follow`,
  or the next progress message drags the cursor off the track that was asked
  for.
- **The library filter matches a folder's name or a track inside it.**
  `Shelf.files` is already in memory from the stat pass `library` does, so
  answering "which playlist holds that track" costs no disk and no worker
  round trip, and without it the answer was opening folders one at a time.
  `shelf_matches` short-circuits on the name; the filenames are lowercased per
  call rather than kept twice, since a second copy of every filename in the
  library is the costlier half. The preview is what says *which* track
  matched, so below `PREVIEW_FROM` a folder can be on the list with nothing on
  screen saying why: `t` is the answer to that, and the reason it is offered
  in the status bar rather than only under `h`.
- **Undo holds the values, not a diff, and only one level.** The edit worth
  taking back is the one just seen landing on the row; a stack would be a
  second history to keep in step with the files on disk. It clears itself
  after one use, or `u` is a redo wearing the name of an undo.
- **`Msg::Undoable` is what makes `u` a key at all.** The worker holds the old
  values, so the UI cannot work out whether there is anything to put back, and
  a key that can only fail is worse than no key. The bar names what it holds,
  since one level of undo is only usable if you can see which level.
- **An undone track reads `undone`, not the status it had.** Somebody typed
  those values back. Restoring `ok` would claim a lookup that is no longer
  what is in the file.
- **A bulk undo collects each track's values as its write succeeds.** A track
  that could not be written is not one `u` may claim to put back.
- **The log offset counts from the tail.** That is where a running job writes,
  so a line arriving while someone is reading further up has to grow the
  offset or the window moves under them.
- **The help overlay gives way in order: the legend, then the settings tail.**
  The keys are what it is for, and the popup must clear the frame by two rows
  or it covers the status bar. Adding rows to that table is what makes the
  rule bite, and the failure is a test somewhere else entirely.
- **The window title is written only when it changes.** Every frame otherwise
  writes an escape sequence nobody asked for. It says `0/0` never: a scan that
  is working would look like one that is stuck.
