# What a key is allowed to do


- **Esc never ends the session.** It used to quit whenever `leave_tracks`
  found no library behind the run, which is every first run from start to
  finish, so one Esc aimed at a filter or a closed prompt took the download
  with it. It now reports what it cannot do, because a key that goes silent
  reads as a broken key. `q` and ^c are the only ways out, and the help drops
  the `back Esc` row when there is nothing behind the run rather than
  documenting an exit that no longer exists.
- **`App.say` is the UI talking, not the worker.** It is `Msg::Flash` without
  a message, for the keys the UI answers by itself.
- **Esc unwinds filter, then marks, then the library.** The order lives on
  `escape_tracks` rather than as match arms in `main`, so the three steps stay
  one decision. Marks sit in the middle because they survive a cleared filter;
  anything that later inserts a step before `leave_tracks` keeps the screen
  while it unwinds, or one Esc starts navigating mid-cleanup.
- **The caret is a char index, `caret_byte` converts.** A byte index lands
  inside a multi-byte character the first time a title carries an accent, and
  `String::insert` panics on it. Every input edit goes through the `input_*`
  methods so the caret and the string cannot disagree.
- **The prompt block is drawn between `input_parts`.** Appending it to the end
  of the line put the cursor where the next letter was not going.
- **`App.viewport` is set by the draw, read by `page`.** Only the layout knows
  how many rows the list got, and a page key that moves by anything else
  scrolls past what the user was reading.
- **The status bar's hints yield to the tally, and `extra` is what yields.**
  Every count in the tally is unrecoverable once it is off the row, where
  every hint is one `h` away. `TALLY_FLOOR` is the room the counts keep.
- **A status legend only draws when the whole popup still clears the frame by
  two rows.** It grew tall enough to cover the status bar once, which hid the
  tally it exists to explain. The keys are what the overlay is for, so the
  legend is what gives way.
- **Colour is confidence, the word is provenance.** `ok` and `manual` share
  teal because both are checked; `guessed`, `unsure` and `no match` are sand
  or amber because none of them are. `kept` sat in dim beside four states with
  nothing to check at all, so an unverified guess looked as settled as a
  skipped track.
- **`log_color` exists because a clean run still warns.** Painting all of
  stderr red made every run look broken and hid the one ERROR in it, and the
  `l` hint burns amber only when `log_errors()` finds a real one.
