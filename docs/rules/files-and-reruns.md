# Files and re-runs


- **Never `bail!` out of the pipeline for a download failure.** yt-dlp exits
  non-zero if any single entry fails, and aborting throws away the tags, cover
  and playlist for every track that succeeded. Carry it into the summary.
- **An edit survives a resync through three links, and the guarantee is the
  join.** `manifest::read` points the scan at the renamed file so it comes
  back `Have` rather than being predicted absent; `write_archive` names every
  id whose file exists so yt-dlp skips it rather than writing over it; and
  `tag_tracks`'s `Have` branch reads the tags off disk and neither identifies
  nor renames. Each link has its own test, and
  `a_resync_keeps_the_name_an_edit_gave_a_track` is the three in a row,
  driven through a stub yt-dlp printing the filename the template would have
  predicted. Note what it does *not* claim: the number in the filename does
  not follow a reordered playlist, because `rename` runs only in the identify
  arm that `Have` skips. The `.m3u8` is written in the new order with the
  current filenames and `open_shelf` renumbers from those, so it stays
  self-consistent while drifting from the playlist position.
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
- **The `.m3u8` names them decomposed, and every comparison composes first.**
  macOS stores these names composed and APFS ignores the difference on lookup,
  so the bug is invisible here: copying the folder to an iPhone hands the
  filenames over decomposed, iOS compares bytes, and the playlist names
  nothing. Measured in VLC on iOS; docs/history.md has the four spellings and
  what they ruled out. The second half is what makes the first safe. `mark_departed` matches stems by
  string equality, so decomposed entries against composed filenames miss every
  Hangul or accented name while the ASCII ones still match, which is exactly
  enough to clear the "believe none of it" guard and call the rest `Gone`: the
  half-converted folder's bug reached by a different road, and `stem_of` is the
  one place it is closed. `repoint_playlist` matches composed for the same
  reason and because every playlist written before this holds composed entries.
  Relative names alone do not make a folder portable; this is the other half of
  that promise. It costs the opposite case, Linux or Android, which
  docs/history.md records as a choice rather than an oversight and as the
  point where this becomes a setting. `Playback::on` composes both sides too, since
  one path comes back out of cliamp and the other came off disk.
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
- **`can_command`, `can_browse` and `can_find` are exclusive on `View`.** The
  three screens move different cursors, so a key live on the wrong one acts on
  a row that is not there.
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
- **`run_with` takes the binary, like `scan_with`.** Asserting on a helper
  that built the download's arguments would leave the call site free to pass
  anything, which is exactly how the scan's format came to be right in the
  helper and hardcoded at the call.
- **The scan's `--print` carries duration and the playlist id, and the stubs
  have to agree.** `splitn(6)` with the filename last, because that is the one
  field that could hold a tab. Three stub yt-dlps print that line and all
  three break together when the format changes, which is the point: a stub
  still printing five fields would make every track look nameless rather than
  failing where the format is.
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
