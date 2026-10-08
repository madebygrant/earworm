# Custom playlists


- **A custom playlist is a library row whose `path` is a file, so no folder
  command may ever see one.** `library()` returns folders only and is what
  resync, `--list`, `--check` and the duplicate checks walk: a list is none of
  those, and that is also why `--resync` skips them with no "no URL" line.
  `library_all()` is the folders plus the lists, and only the calls that feed
  the screen use it. The keys route a custom row by name (`App::custom_row`),
  and `not_custom` in `remove_shelf`, `rename_shelf` and `open_row` is the
  second lock: `remove_dir_all` on the store would take every list.
  `no_folder_command_will_act_on_a_custom_playlists_path` pins the second lock
  and `a_custom_row_is_routed_by_name_and_never_sent_as_a_folder` the first.
- **`Kind::Custom` is made by the library and read from nowhere.** No folder
  name or `#kind` header can claim it, or a hand edit would turn a download
  folder into a row that cannot sync. It gets its own icon and pill, its own
  stop on `a`, and is left out of `found_rows`: its tracks are already found
  in the folders they came from, so listing them again shows each twice.
- **An entry is a video id, resolved through the library, with the last path as
  a hint.** A path breaks on the first edit, because `write_track` renames the
  file. A `~` id survives that (it is made once and never changes) but two
  folders can hold the same one, so it only resolves in the folder its hint
  names, and `rename_shelf` calls `playlists::rehome` to keep the hint true. A
  hint is untrusted: the file is hand-editable and travels, so an absolute path
  or a `..` is dropped on read and write and never resolved (`safe_hint`); an
  absolute path replaces the root in a `join`. Resolution reads hidden folders
  too (`Others::everything`): hiding takes a row off the library and the
  duplicate checks, and a list that goes on playing the folder's files is what
  the hide promises. `refreshed` runs whenever a list is opened or played and
  gives each entry that resolves the hint it has now and the id its folder's
  sidecar knows it by, because a sync that matches an adopted file to a video
  drops the `~` line and a hint that is never refreshed then points at the old
  name after the next edit
  (`opening_a_list_moves_a_dropped_local_id_and_a_stale_hint_onto_what_the_sidecar_now_says`,
  `a_hidden_folders_track_still_resolves_after_it_is_renamed`).
- **A `~` id is unique within its folder.** `id_for` counts up (`~Song.opus
  (2)`) when the id is taken, because a renamed adopted file keeps its old id
  and a new file may be named like the one it was made for; two lines with one
  id made the list resolve to whichever file came first
  (`a_new_file_named_like_a_renamed_one_gets_an_id_of_its_own`). `contents()`
  in `worker.rs` still builds its ids from filenames, which is the same
  collision on the read side and is left as it was.
- **Adding a track adopts its folder.** A file in a folder earworm never edited
  has no sidecar entry, so the add writes one (`~file`) before the list holds
  it, and says so in the log: a hidden file appearing in somebody's music
  folder is not something they should find out by looking. Without it the
  entry would resolve only by its path and the first rename would lose it.
- **Every command on the screen carries an id, never a position.** `J` and `K`
  reorder rows under the cursor, so `Msg::Custom` carries a `focus` id and the
  cursor follows the entry; a refresh with none keeps the entry it was on. The
  list holds each id once, so an id names one row.
- **`View::Playlist` has its own keys and none of the track list's.** `e` there
  would write tags into a folder the list does not own, and `s`, `r` or `c`
  would act on tracks that are not in the worker's list at all. Enter jumps to
  the source folder, which is where edits are made, and says so for a missing
  track. `can_playlist` is exclusive with `can_command`, `can_browse` and
  `can_find` on `View`. `the_playlist_screen_takes_its_own_keys_and_none_of_the_track_commands`.
- **`N` ends in the track picker, `View::Pick`, and it is UI state.** After
  the name the worker sends `Msg::PickTracks` and carries on: nothing blocks on
  the picker, so a UI that never answers strands nothing. Enter on a new list
  sends `Cmd::AddToPlaylist(picks, Some(name))`, which skips the chooser and
  opens the list; Esc keeps the empty list, since deleting it would throw away
  the name just typed. `can_pick` is exclusive with the other four screens, and
  `calm()` is false there, because the box is always typing and an achievement
  popup would swallow a keystroke of the query
  (`the_picker_is_exclusive_with_every_other_screens_keys`,
  `an_achievement_waits_while_the_picker_is_open`,
  `a_new_list_is_saved_empty_and_opens_the_picker`).
- **`a` on a list's screen reopens the picker, and its tracks start marked.**
  `PickView.held` is what the list held when the picker opened and `marks`
  starts as a copy, so unmarking a held track is a removal. Enter sends
  `Cmd::EditPlaylist { add, remove }`, both lists being the difference between
  `marks` and `held` (`pick_changes`); a removal drops the entry and never the
  audio. Held tracks are read off the list screen's rows, so one whose file is
  missing is not held and can be added again. The rows are `•` kept, `−`
  leaving and `✓` new, so no state rests on colour. Enter with no difference
  says so and stays, and the Esc question is asked only for a difference
  (`tracks_the_list_holds_start_marked_and_unmarking_one_takes_it_off`,
  `an_edit_takes_unmarked_tracks_off_the_list_and_never_off_disk`,
  `leaving_an_open_list_unchanged_does_not_ask_but_a_removal_does`).
- **Esc on the picker asks first when it would lose something.** A stray Esc
  after forty marks across several queries is unrecoverable, so it raises
  `Confirm::Discard`; Esc on that question means keep picking, as it does on the
  quit question. A new list asks for any mark and an open list for any change
  (`esc_with_marks_asks_first_and_only_a_yes_discards_them`).
- **The picker lists every track on an empty query, the one exception to "an
  empty query is no results".** That rule exists so the search screen is not the
  library screen; here the whole library is the point.
  `an_empty_query_lists_every_addable_track_in_order`.
- **Picker marks are `(folder, filename)` in the order marked, never row
  positions.** Ranking reorders rows on every keystroke and a refresh shifts
  them all; the mark order is the order the tracks join the list. The cursor is
  a position and goes back to the first row on a new query, as in fzf. Tab
  toggles in place and does not move the cursor. `Marks` keeps a set of composed
  keys beside the ordered list: a linear scan that composed every name made `^a`
  over 5,000 tracks take three seconds on the UI thread
  (`marks_keep_their_order_and_survive_a_new_query_and_a_refresh`,
  `marks_made_under_two_queries_go_in_the_order_they_were_marked`,
  `marking_five_thousand_tracks_is_not_quadratic`).
- **`F2` filters to the marked rows and is applied in `pick_refresh`, not by
  rebuilding `hits`.** A query hides earlier marks and nothing else shows them.
  Marks change while it is on, so the toggle paths refresh too, or an unmarked
  row stays on screen
  (`f2_shows_only_the_marked_rows_and_keeps_them_across_a_query`).
- **A row is the title and its folder, matched together.** The label is the
  filename without extension or playlist number (`strip_number`, so `1979` keeps
  its digits), because the order is by relevance and the number says nothing.
  The query runs against `title folder`, so `focus creep` finds a track by its
  folder. nucleo's indices count graphemes and the highlight walks chars, so
  `char_positions` converts them; indices past the joining space belong to the
  folder
  (`the_playlist_number_is_dropped_but_a_title_that_opens_with_one_keeps_it`,
  `the_folder_is_matched_too_alone_or_with_the_title`,
  `a_multi_codepoint_grapheme_does_not_shift_the_highlight`).
- **The row scrolls the first match into view.** A plain truncation cuts a long
  title at the edge, which can hide the only characters that matched. `fit`
  starts the text about a third of the width before the match
  (`a_match_late_in_a_long_title_is_scrolled_into_view`).
- **Letters type in the picker, so help is `F1`, the marked view is `F2` and
  `^c` quits.** `j`, `k`, `q` and `h` are part of the query there, and the
  status bar drops `h keys` for the same reason. `rows` runs on the UI thread at
  each keystroke and not per frame (`pick_refresh`); the 5,000-file test bounds
  it. The bar's Enter hint carries the count it will act on
  (`the_picker_bar_does_not_offer_h_for_keys_and_says_esc_cancels_on_an_open_list`,
  `enter_says_what_it_will_do`).
- **`D` on a custom row deletes the list and never audio.** It asks first, names
  the count, and the flash says no audio was touched. The exit test deletes a
  list built from two folders and checks every file, both sidecars and the
  library row.
- **The in-library `.m3u8` is the one playlist file allowed `../`.** Its entries
  are `../Folder/file`, relative to the store, so it holds wherever `--dir` is
  and every other `.m3u8` still names bare filenames. It is for playback inside
  the library: a car stereo or a phone will not follow `../`, which is what
  export is for. `save` refuses a file that does not parse, since `say_broken`
  tells the user it is hidden and a new list of that name would otherwise write
  over its entries
  (`saving_over_a_file_that_does_not_parse_is_refused_and_leaves_it_alone`). It
  is written when the list is played, and a list with nothing
  on disk refuses rather than handing cliamp an empty file. `play` itself has no
  test: it reaches the real cliamp store.
- **The `▶` follows the list that was played, not the folder its track is in.**
  cliamp reports a track and never a playlist, so `App.on_air` remembers the
  list `p` handed over and the folders it draws on (`Msg::OnAir`), and
  `row_on_air` marks the list while the track sits in one of them. Playing a
  folder sends `OnAir(None)`. Switching playlists inside cliamp is invisible
  from here, so the marker can stay on a list until its track leaves those
  folders. A rename sends `Msg::Renamed` before the library refresh, or the
  refresh sees the old name gone and clears the marker; a deleted list clears
  it from the refresh itself
  (`the_playing_marker_follows_a_renamed_list_and_goes_with_a_deleted_one`).
- **The library cursor follows its row by path.** `Msg::Library` looks the
  row up again after a refresh, since lists sort in among the folders and a
  list made or deleted shifts every row after it, and `Msg::Select` puts it on
  a list just made or renamed. A row that is gone keeps its index, clamped
  (`a_library_refresh_keeps_the_cursor_on_the_same_row_not_the_same_index`).
  The sync sorts leave lists out: they have no sync time and their dead
  entries are not something a sync fetches (`a_list_takes_no_part_in_the_sync_sorts`).
- **A move or a removal sends `Msg::Order`, never `Msg::Custom`.** `Custom`
  re-reads the tags of every entry, which on a long list made `J` and `K`
  sluggish. `Order` carries ids only and the screen reorders the rows it
  holds; an id it does not hold means the file changed behind it, so the
  message is ignored and the next open loads the list fresh. A move leaves the
  library alone, since its counts cannot change; a removal refreshes it once.
- **Export copies and never deletes, and names are mapped on the way out
  only.** `export::plan_*` build the whole plan (source, destination, playlist
  text) before anything is written, so the mapping, the collisions and the
  layout are tested without a device. `portable::name` then `unique` decide
  each filename: `clean()` stays as it is, or the library's files would be
  renamed on their next edit. A file is skipped when size and modification
  time (to 2s, FAT's resolution) match, so `put` sets the time after copying.
  Copies go through a hidden `SCRATCH` name and a rename, so a kill leaves
  nothing a later export reads as a track. The `._` file macOS writes on exFAT
  is removed only when this call caused it, checked before the write.
  The destination must exist and sit outside `--dir` (`check`), or a typo
  makes a new folder and a path inside the library re-reads its own export.
  `Msg::Stage` carries progress and `cancel` is the quit flag. Nothing tests
  the free-space check, which shells out to `df`, or the prompts in `run`.
- **A mirror offers only what this export recorded, and never replaces what
  it did not write.** Each folder on the device holds a hidden `.earworm-export`
  with an `owner<TAB>name` line per file earworm put there (`RECORD`; the owner
  is `folder:<name>` or `list:<name>`). `leftovers` offers a recorded name only
  when the plan no longer writes it and no other owner's line claims it, and
  `remove` re-checks the record and that the path is under the destination.
  `execute` leaves alone a file that exists and is not recorded under this
  owner, and `write_text` does the same for a playlist file, so an unrelated
  `Music/` already on the stick is neither offered nor overwritten. The
  folder-layout list can be mirrored for the same reason: it names a track as
  the folder's own export does (`folder_names`), so two exports never give one
  device name to two tracks, and a file both put there is claimed by neither's
  mirror. Every comparison goes through `key` (composed, lowercased), because
  APFS, FAT and exFAT call two spellings one file: compared byte for byte, a
  rename that only changed case offered the file just written, and answering
  yes deleted the only copy. The question is asked before the copy and acted
  on after, and not at all if the copy was stopped. The record is a file on
  the user's device that an older earworm cannot know about, and a stick moved
  to another machine keeps it.
  `a_rename_that_changes_only_case_or_accent_form_is_never_offered_for_removal`,
  `a_device_folder_earworm_did_not_write_is_neither_offered_nor_overwritten`,
  `a_list_and_a_folder_of_one_name_never_offer_each_others_files`.
- **A re-encoded copy is skipped on its time and its codec, not its size.** The
  size differs from the source by design, so `already_there` drops that test
  for it, and reads the codec instead because alac and m4a share an extension;
  breaking the first makes every second export convert again, and the second
  keeps an alac file for an m4a export
  (`a_second_export_in_another_codec_under_one_extension_is_redone`)
  (`a_converted_track_takes_the_new_extension_and_a_second_export_leaves_it`).
  `reencode` carries tags and the picture across itself, as `swap_in` does,
  and a track already in the target format is copied. The format list is
  `FORMATS`, so a codec ffmpeg lacks fails per track and is logged.
- **`P` is not in the track list's help table.** That table is 21 rows and a
  24-row terminal has no room for a 22nd, which dropped the play indicator and
  broke a test somewhere else. The key is offered in the status bar's hints
  instead, which give way.
- **Not covered:** driving the new keys through a pty. The harness lost keys in
  this environment, so the screens were checked once live and the rest through
  `TestBackend` and key tests.
