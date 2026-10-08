# Albums and playlists


- **A folder's kind is read off its name first, and the name is never
  written.** `manifest::kind_of` is the one derivation, asked by both the sync
  and `open_shelf` so the two cannot disagree. A bracket group whose whole
  contents are `album` or `playlist`, case ignored, anywhere in the name. The
  whole contents, because brackets in music folder names are usually a year, a
  format or an edition, and a substring match reads `[Album Version]` and half
  a library besides as albums. Two different markers in one name is no answer
  rather than the first. Re-read every time, so deleting the marker takes
  effect at once, which is the thing a stored answer cannot do.
- **A marked name records no `#kind` header.** Two records of one fact drift,
  and this drift is silent: a header left behind by a marker somebody has since
  deleted would go on deciding with nothing on screen saying why. Only
  detection writes the header, because detection has nowhere else to put its
  answer; the name does not need one.
- **Detection resolves before the download and records after it.**
  `settle_kind` answers and writes nothing; `record_kind` writes the header
  once there is a folder to write into. They were one function, and the bug
  that split them is that a brand new playlist's folder does not exist until
  yt-dlp makes it: the write failed with ENOENT on the one run that discovered
  the album, so the header was never recorded, the library showed no album
  until somebody synced a second time, an error line landed in the log pane on
  every first album download, and `r` in that same session re-resolved
  `playlist` and deleted the `cover.jpg` the run had just deliberately kept.
  `record_kind` returns quietly when the folder is still missing, because that
  is a run where nothing downloaded at all and there is nothing to report.
- **Detection only ever promotes to `album`, never demotes.** The two mistakes
  are not equal. Calling an album a playlist leaves today's behaviour in place;
  calling a playlist an album stamps one sleeve across a dozen unrelated
  records, which is the whole bug. So only the `OLAK5uy_` prefix on
  `Listing.playlist_id` promotes a folder and silence means playlist. The
  uploader was going to be the second signal and is not: yt-dlp reports an Art
  Track's channel as `Kraftwerk`, not `Kraftwerk - Topic`, so one shared
  channel cannot tell an album from a single-artist playlist. Measured, not
  assumed. The flat listing carries no `album` or `release_year` field at all,
  so there is nothing else in it to read.
- **An album's tracks share one sleeve, and `tag::sleeve` is where that
  happens.** `lookup::cover_bytes` is keyed on each track's own `Match` with no
  cache, so twelve tracks of one record resolve twelve times and a track that
  matched a single, a reissue or a compilation comes back with that sleeve: one
  album, three covers. The first answer is kept on `CoverState.shared` and
  reused. It takes the fetch as a closure rather than calling `lookup`, which
  is the only reason the sharing is testable without a network: the test counts
  how many times the closure runs, and "the files all match" cannot see the
  difference between one request and twelve. A miss stores nothing, so the next
  track still tries and an obscure opening track does not cost the folder its
  art.
- **An album shares its album tag as well as its sleeve, and `tag::album_name`
  is the other half of `tag::sleeve`.** The two are read together, so sharing
  one and not the other is the same bug wearing different clothes: track five
  matching a compilation wore that compilation's name beside the record's own
  sleeve, which is worse than the twelve-covers case it replaced, because the
  file now contradicts itself. Kept apart from `sleeve` rather than folded
  into it, because this answer costs no request: it is first-found rather than
  first-fetched, so a track whose art failed still settles the name.
- **An album keeps its folder image; a playlist drops one.** On a playlist
  `cover.jpg` is whichever track resolved art first wearing the folder's name,
  which is what `drop_folder_cover` was written to clear up. On an album it is
  that album's sleeve and matches every file in the folder. The album flag is a
  parameter rather than a check at the call site so the rule is testable where
  it lives.
- **Three lines in `pipeline` carry these answers and nothing tests them.**
  `one_sleeve: album` on the `CoverState`, the `album` argument to
  `drop_folder_cover`, and `written: !theirs.is_empty()`. All three still
  compile and the whole suite still passes with any of them flipped, because
  reaching them needs a live scan and, for the last two, a live lookup. Same
  trap as the `gate` literal `resync` passes, and the same answer: treat those
  three lines as load-bearing. What can be tested is tested one level down:
  `an_albums_files_all_read_back_the_same_album_tag` builds a `Match` by hand
  with the cover off, which is the only way to reach `tag::apply` without a
  network.
- **The pill takes the marker out of the name, and only the album one.**
  `Autobahn [album]` beside a pill saying `album` says it twice.
  `manifest::without_marker` drops the group, in the library row alone: the
  folder is not renamed, `e` still opens on the name it really has, and the
  `/` filter still matches the text that is no longer on screen, which the
  preview already does for filenames. Only the kind the row is showing,
  because a `[playlist]` folder has no pill and the name is then the one
  place that answer is on screen. It reads a bracket group exactly as
  `kind_from_name` does, or the screen and the answer would disagree about
  what counts as a marker, and a name that is nothing but the marker comes
  back whole, since an empty name column says less than a repeated word.
- **The album pill's slot is reserved per draw, not per row.**
  `draw_library` asks whether any row on screen is an album and only then
  takes `KIND_WIDTH` off the name column, so a library with no album in it
  draws exactly as it did before the pill existed. Per row it would cost
  eight columns of name on every library; always on it would cost them on
  every library. The gap sits outside the fill, or the pill is not one and
  the counts after it step sideways between rows. `ink` on `accent` is the
  fill, reversed without colour, because that is the one pair every palette
  is measured on and `draw_band` is the precedent; the word `album` is what
  carries the state, so nothing here is told apart by colour alone. Only an
  album is marked: silence is the playlist answer everywhere else in this
  feature. Testing the fill is the part that bit. `bg.is_some()` is true of
  every cell, because the gradient paints one, and "different from the cell
  beside it" is true of plain text for the same reason: the assertion has to
  name `WARM.ink` and `WARM.accent`. A row comparison has to count columns
  rather than bytes, too, since the selected row opens with a three-byte
  `▌`.
- **`icons` picks how the library says a kind, and `text` is the layout above.**
  `symbols` is the config default and draws `●` for an album, `▤` for a
  playlist and `◆` for local, in `accent`, which every palette already
  measures against the gradient. The kind slot is `icons::SLOT` cells on every
  row, not per draw, so a `symbols` library with no album loses two columns of
  name where a `text` one loses none: a playlist has an icon, and a slot that
  came and went would move every column as albums were synced. `shown` strips
  `[playlist]` as well as `[album]` under `symbols`, since the icon now says
  both. The tier is a setting and not detected: a Private Use Area glyph is
  a box in a font without it and nothing reports that. Emoji were rejected,
  because colour emoji ignore the foreground, which breaks themes, `NO_COLOR`
  and the 256-colour quantiser, and are two cells wide. `nerd` draws the Material Design album,
  playlist_music and folder_music glyphs (codepoints checked against Nerd
  Fonts' glyphnames.json) and is opt-in because it needs a patched font. Its
  slot is a cell wider, `Icons::slot`, because those glyphs are drawn wider
  than the one cell `unicode-width` gives them and would touch the next column. `App::new` starts on
  `text` and only `main` sets the tier, so the pill tests are text-tier tests.
  Tested by `symbols_mark_every_row_and_keep_the_columns`; the legend rows in
  the library help are not.
- **`a` narrows the library by kind, and it is a predicate, not a query.**
  Neither word is in a folder's name, and a folder with no kind is a
  playlist, which no text in the box could say. So `App.only` sits beside the
  filter as `review` does on the track list, `kind_shows` counts a missing
  kind as a playlist, and `clear_filter` drops it with the rest. The library's
  Esc asks `shelf_narrowed()`, not `narrowed()`: the latter is the track
  list's and cannot see the kind, so Esc would answer "this is the top" with
  the list still short. `found_rows` and the search band's total ask
  `kind_shows` too, or `t` turns up tracks from folders the rows just hid,
  and emptying the search box clears the query alone rather than calling
  `clear_filter`, since the kind belongs to the library it returns to. The
  empty pane names the kind, because "nothing matches" has no query to quote.
- **The album tag fill was deliberately left alone.** `cfg.album` still writes
  the playlist's name into an empty album tag whatever the kind is. Gating it
  on `album` would be better tagging and is a silent change to the default path
  for every existing folder, which this feature promised not to make;
  `--no-album` is the control that already exists. Worth doing on its own, not
  smuggled in here.
