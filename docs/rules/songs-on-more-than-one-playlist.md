# Songs on more than one playlist


- **A video another folder already holds is copied, not downloaded.**
  `library_index` reads the sidecars of every other folder under `--dir` for
  this pass's `Pending` ids, and `copy_known` runs straight after the pick gate
  so an unpicked track is not copied. It is built per pass and not per
  `--resync`: a song on two playlists arrives in the first folder's pass and the
  second folder's pass has to see it. Folders are read in name order, so the
  same video in three folders always copies from the same one. The rest of the
  library is read once per pass through `Others`, lazily: the id index, the ISRC
  check and the reopened folder all ask it, where each reading every sidecar
  itself made one pass cost the square of the library. `Others` is still built
  per pass, so a `--resync` of N folders reads N sidecars N times: a second
  folder has to see what the first just fetched, and the reads are small, so
  it was not worth a cache that has to know when to go stale. Nor does
  `library` stop statting every file whenever a pass has a download the id
  index could not account for. A source counts
  only when `format_on_disk` matches `cfg.format`, or the copy would put a
  second format in the folder for `convert` to undo, and `~` and chapter ids
  are skipped for the reason `write_archive` skips them. `--resync` passes no
  gate and still copies, which is the point of an unattended sync.
- **A copy, never a hardlink.** A tag edited in one folder must not change the
  other. On APFS `fs::copy` clones, so it costs no space until one side is
  edited. The copy is named after the source file with this playlist's number in
  front, not the path the scan predicted: the prediction is yt-dlp's raw title
  and the source already holds the name its lookup or an edit gave it, and a
  `Have` track is never renamed. It goes through a `SCRATCH` name and a rename,
  so a copy killed part-way cannot leave a truncated file that the next scan
  calls downloaded. A name already taken is left alone and the track downloads
  as usual. `write_archive` then names the id, because the file exists, and
  yt-dlp skips it: that join is `a_video_another_folder_holds_is_copied_and_the_archive_skips_it`.
- **The copy keeps the source's tags, album included, and `retry` does not
  copy.** Decided, not overlooked: if the source's lookup found no album, its
  album tag holds the other playlist's name (the `cfg.album` fill) and the copy
  keeps that. Rewriting it would mean telling that fill apart from a real album
  that happens to share the folder's name, which nothing records, and a wrong
  guess overwrites a real tag. The user can set it with `A`. A retry's downloads
  are tracks that already failed once, so skipping them there costs little.
- **A duplicate is `Track.twin`, not a `Status`.** A status has to pass
  `settled()` and `tally()`, and it would replace the word that says how the
  tagging went, which a duplicate is not. `Msg::Twin` sets or clears it and the
  row says `≈` and the words in `warn`, so nothing rests on colour. It has
  a column of its own ahead of the tail, reserved on every row while any row has
  one and capped at `TWIN_MAX`, so the tail is what clips on a narrow screen and
  the marker never leaves it: after the tail it disappeared at 70, 90 and 120
  columns on a long title. The detail pane carries it whole. A row the sync will copy says `≈ copy from Focus`, because
  unpicking it saves a copy and not a download, and `copy_known` clears it. It is a
  warning only, never a skip: a live version and the studio one share a title.
- **The pick gate compares titles with `reconcile`'s rules, and the library
  half is title only.** Two ids in one listing match on the normalised title and
  durations within two seconds. Against files already in the library only the
  title is compared, since a duration would cost a tag read per file and this
  runs for a screen. A title matches whole to whole, or whole to the part after
  the last ` - `, because one side often carries the artist and the other does
  not; two such tails never meet, or every `Intro` would match every other. An
  id the library holds is copied, so it says so instead of guessing. `library`
  stats every file, so it is only asked when some row is a download the id
  index could not account for.
- **The ISRC lives in the tag and in `#isrc` header lines, never in a third
  column on the entry line.** An older binary splits an entry on its first tab,
  would read `01 - A.opus\tGB01A…` as a filename, call every file missing and
  download the library again. `read` skips `#` keys, which is what an old
  reader does with these. `set_isrcs` merges and drops an id with no entry,
  and is called from `write_manifest` after the entries are written, since that
  rewrite decides which ids still exist. The file's tag is the truth and the
  sidecar follows it: `tag::apply` removes an ISRC only when the hit has none
  *and* its title differs from the file's, because an AcoustID or Apple hit
  never carries one and would otherwise strip a correct tag. "Differs" is
  `same_title`: composed, lowercased, and a trailing bracket group ignored,
  since Deezer says `Autobahn (2009 Remaster)` where the others say `Autobahn`
  and the right ISRC went with that. The identify and
  `T` paths then read the file back and call `drop_isrc` if the line is stale. A track already on disk reads its ISRC
  off the tag in `tag_tracks` and `open_shelf`, so an old file is compared and
  a reopened folder shows what a sync did. Pinned by
  `isrcs_ride_in_headers_beside_an_old_format_entry_and_vanish_with_it`.
- **Only Deezer gives an ISRC.** `lookup::deezer` reads `isrc` off the search
  hit it already makes, which costs no request; Apple and AcoustID matches
  carry `None`. `flag_isrc_twins` runs after tagging and in `open_shelf`, and
  compares against other folders' `#isrc` lines, so it costs no tag read. A
  track with an ISRC drops its title-based marker first, since the ISRC is the
  stronger answer and a contradicted guess should not outlive it. The same video in two folders
  is what a copy makes and is not a twin. An ISRC match replaces a title
  guess, and a guess is dropped without one only when the other side is known
  to hold a different ISRC (`title_guess_contradicted`): most files have none,
  and a missing ISRC contradicts nothing. A `copy from` row is never a guess
  (`a_title_guess_survives_an_isrc_that_nothing_contradicts`).
- **Three call sites in `pipeline` carry this and nothing tests them.**
  `library_index` and `mark_twins` before `Msg::Tracks`, `copy_known` after
  `ask_which`, and `flag_isrc_twins` before `sync_manifest`. Reaching them needs
  a live scan, as with the lines in the albums section. What is tested is one
  level down. Moving `copy_known` above the gate would copy tracks nobody picked.
  `copy_known` writes the manifest itself once it has copied anything: the
  copy is named after the source, not the scan's prediction, so a run quit
  during the download left a file no sidecar line named, and the next scan
  downloaded the track again beside it
  (`a_copy_is_recorded_in_the_sidecar_as_soon_as_it_is_made`).
