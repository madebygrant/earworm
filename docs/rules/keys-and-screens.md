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
- **A letter means one thing on every screen.** `o` sorted the library and
  narrowed to an artist's folders, `f` was the format on the library and
  follow on the track list, and `a` filtered by kind and added tracks to a
  custom playlist, so each screen had to be learned again. The library sorts on
  `s`, the format is a settings row, and the playlist screen adds on `P` as
  the track list and search do. Two pairs stay, because each is one idea:
  `n`/`N` is next/previous on the track list and new URL/new list on the
  library, and `e` edits whatever the row is. The pick gate's `a` is "all",
  which no other screen offers. A new key checks `docs/keys.md` for the letter
  first (`p_on_a_list_reopens_the_picker_and_esc_comes_back_to_the_list`,
  `the_library_reveal_key_sends_the_folder_under_the_cursor`).
- **`,` opens the settings ahead of every screen's keys, and never while
  typing.** After the prompts, the help and the console, and before any list:
  the filter box, the artists query and the picker's query take it as a
  comma. Esc goes back to the screen it came from, which `settings_back` holds
  (`settings_open_from_any_list_and_esc_returns_to_it`,
  `the_filter_box_takes_the_keys_on_the_library_too`).
- **A setting changes only through the worker, and only while it is idle.**
  The worker owns `Config` and does not read commands during a run, so a
  change sent then would wait for the run to end while the screen still
  showed the old value, and `send` would leave `busy` set behind it. The
  first run is not started by `send`, so `busy` is clear through it and
  `done` is what says a run is going. The switch is saved before the session adopts it, as the
  format is. A switch a flag turned off is refused, since the flag would turn
  it off again at the next start; the theme and icons are drawn by the UI and
  change at once. Icons are applied from the rows the worker sends back, so a
  failed save leaves screen and file agreeing
  (`enter_changes_a_setting_unless_a_flag_or_the_file_holds_it`,
  `icons_and_the_bell_follow_the_rows_the_worker_sends`,
  `a_switch_is_saved_before_the_session_takes_it`).
- **The bell is read from the settings rows, not a copy taken at startup.**
  `notify` was a local in `run`, so turning it off on the settings screen
  changed nothing until the next start. `App::notify` reads the rows the
  worker re-sends (`icons_and_the_bell_follow_the_rows_the_worker_sends`).
- **Every switch is one key in two places.** `SWITCHES` carries what the
  screen says, and `switches!` generates `FileConfig::switch`,
  `Config::switch` and `set_switch` from one list of field names. The test
  checks both name the same keys and that each flag is a real `Cli` argument
  (`every_switch_is_read_set_saved_and_flagged_by_its_key`). The AcoustID key
  is shown as `set` or `not set` and never as itself
  (`a_setting_says_whether_the_file_a_flag_or_the_default_set_it`). The theme
  row says `config` once `^t` has saved it, from `App.theme_saved`, since that
  save never passes the worker.
