# Audio format


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
- **The `.m3u8` moves with the sidecar, per track, or a half-converted folder
  reads as a folder full of departures.** `write_playlist` runs once at the
  end of the pass and builds the file from the tracks that are `listed`,
  which nothing is until the tagging pass sets it, so it cannot simply be
  called earlier. Quitting part-way through a conversion therefore left the
  playlist naming the old filenames, and `mark_departed` matches names: every
  converted track failed while the unconverted ones still matched, which is
  exactly enough to get past the "nothing matched, believe none of it" guard,
  so each converted track was called `Gone`. Renumbered past the playlist,
  never tagged again, refused a rename, dropped from the next playlist
  written. `repoint_playlist` renames the one entry, between the rename and
  the removal of the old file so the playlist never names a file that is not
  there. Reproduced on a real folder; the counts are in docs/history.md.
- **`mark_departed` matches the stem, not the whole filename.** A conversion
  changes a track's extension and nothing else, and the playlist cannot be
  kept in step atomically: `repoint_playlist` narrows the gap to one track and
  one instant, and this is what makes even that harmless. Two files differing
  only by extension both match one entry, which errs towards calling nothing
  departed, and that is the safe answer here for the same reason the guard
  below it gives.
- **`D` deletes only departures a listing proved, and `Track::departure_proven`
  is the whole distinction.** Two paths set `Status::Gone` and they are not
  equal. `note_departures` has just seen a complete, unnarrowed listing with
  the video missing from it, and marks the row proven. `mark_departed` reads
  the same fact off the last playlist file when a folder is opened offline,
  and leaves it false: a narrowed run writes a short `.m3u8`, so an offline
  read can call half a folder departed, and a deletion resting on that takes
  music. The key is not drawn for an unproven `gone`, and the worker's refusal
  says that `S` is what would prove it. The old test comment "reported as
  gone, a later prune deletes music" was written before the prune existed,
  and this flag is the answer to it.
- **`purge` asks with every filename in the note, checks every path is under
  `--dir`, and re-checks the file is there.** The header clips, so the names
  go where they wrap; a question about deleting music that does not say which
  music is not one anybody can answer. The paths come from a sidecar that a
  hand could have edited, and one outside the library stops the whole purge
  before the question, not just that file. The manifest then drops the
  entries by itself, since it keeps one only while the file exists, and the
  `.m3u8` never listed them; the save is still needed at once, or the library
  screen counts the deleted files as missing until the next sync. The test
  for that reads `manifest::entries`, because `manifest::read` filters on the
  file and passed against the missing save.
- **`Msg::Dropped` takes rows off the list; `Msg::Tracks` does not mean
  that.** `Tracks` restarts the estimate's clock and says the scan is over. A
  dropped row also takes its mark with it, and the cursor is clamped rather
  than snapped: the filter's snap moves a cursor sitting on a hidden row, and
  one past the end is not hidden, it is nowhere.
- **A narrowed run writes a short `.m3u8`.** `note_departures` is silenced by
  `cfg.extra` but `write_playlist` is not, so `--playlist-items 4` leaves the
  playlist file naming one track. Found while deciding what a purge may trust;
  not fixed, and the reason an offline `gone` is never proof.
- **`lookup::stop` is `ytdlp::stop` for everything else, and it latches.** A
  transcode is minutes of ffmpeg writing into the user's playlist folder, so
  quitting without it left an orphan running after the terminal was restored.
  The registration lives inside `run_bounded`, with the waiting, rather than
  at each call site. The latch is the half that is not obvious: killing the
  child makes the call return a failure, and a failure is what makes
  `transcode` try the next encoder in its list, so the shutdown killed one
  ffmpeg and the next line spawned another that nothing was left to kill. It
  also closes the gap between a loop's cancel check and the spawn a few lines
  later. Verified by hand, like `ytdlp::stop`, which has no test either; the
  timings are in docs/history.md. **Nothing in the suite may call it.** It is process-wide and never cleared,
  so one call makes every bounded subprocess after it in that binary return
  `None`, which is the trap the colour depth already documents.
- **A killed transcode's scratch file is swept, not overwritten.** The kill
  comes from the shutdown path, so the loop never reaches the cleanup in its
  own error arm. Writing the same name again covers only a repeat of the same
  track into the same format, and changing format in between strands a
  multi-megabyte file in the music folder for good. `sweep_scratch` matches
  earworm's own hidden `SCRATCH` prefix, so it can never take a file somebody
  else put there, and one constant names it for both the writer and the sweep.
- **Changing the format converts nothing. `convert` is what does, and only a
  sync.** The manifest records each track by the filename it has, so those
  tracks stay downloaded and a folder switched part-way holds both. That is
  the guarantee `format` makes and it cannot be the one that breaks it: a
  plain menu pick that rewrote the library would turn one keystroke into a
  rewrite of every folder under `--dir` at the next `--resync`, which is one
  more keystroke. So `convert` is a second key, off by default, and the one
  setting here that does not go through `off`: `opt_in` is the same rule
  about the command line only ever switching something off, defaulting the
  other way. `set_format` asks about it straight after a format pick, since
  that is the one moment the question is in the user's head, and never when
  it is already on.
- **The conversion lives in `pipeline`, so `--resync` and a URL run get it
  the same way.** Opening a folder from the library converts nothing: that
  path is offline and read-only by design. `to_bring` is empty whenever
  `convert` is off, which is the single point the promise above rests on.
- **`bring` is decided before the download and nowhere else.** That is the
  last moment at which every track either holds the file an earlier run left
  or holds nothing at all; the `@D` line overwrites `track.path` for anything
  yt-dlp fetches, so the old path has to have been remembered by then. Only
  `Have` is considered: `Gone` holds no playlist position and its index is
  synthetic, `Skipped` was never fetched, `Pending` is about to arrive in the
  target format anyway.
- **`bestaudio` on YouTube is the Opus stream, and the whole conversion table
  follows from that.** Measured with `yt-dlp -f bestaudio --print
  %(acodec)s`: itag 251, Opus in WebM, with the AAC stream (itag 140) present
  and not chosen. So `--audio-format opus` copies what it already has, which
  `ffprobe` confirms on a real download: the file tags `ENCODER=Lavf`, the
  muxer, where a re-encode names the encoder instead, and the bitrate is
  YouTube's itag rather than an ffmpeg default. Every other format is ffmpeg
  re-encoding that same Opus, except a download to `m4a`, which asks for
  `bestaudio[ext=m4a]/bestaudio` (`NATIVE_M4A`): YouTube's own AAC is then
  copied (measured: itag 140 lands at exactly 128k, where the Opus route
  re-encodes to about 170k), and a video without one falls back to the Opus.
  So an `.m4a` downloaded before that change, or converted from another format, is Opus
  transcoded to AAC, and one downloaded since is usually one generation. `mp3`
  and `vorbis` on disk are always two generations deep, and an `m4a` file is
  refetched rather than converted because nothing on disk says which kind it is. It is also why
  an `opus` or `m4a` *target* is refetched from anything: a download copies
  YouTube's own stream, so it costs one generation against the two any
  conversion would (a video with no AAC stream costs two either way). `alac`
  to `m4a` is the exception and converts: both are `.m4a`, so the refetch
  lands on the path the old file holds and yt-dlp skips or rewrites it. The first
  version of this table had both wrong; docs/history.md says on what
  assumption.
- **`alac` and `m4a` share an extension, so the codec has to be read.**
  Without `tag::mp4_format` an AAC file sits untouched in a folder being
  brought to alac and an alac file does the same going the other way, because
  both are already `.m4a` on disk. `None` from it is a real "cannot tell" and
  the track is left alone: falling back to the extension would pick one of
  the two at random and re-encode on no evidence. The library's `off_format`
  count *does* fall back to the extension, and says so, because counting from
  a filename is the whole reason it costs no disk.
- **A refetch is left out of the archive, not given a `Status`.**
  `write_archive` names every id whose file exists, so an mp3 being converted
  is exactly the track yt-dlp would skip. A `Status::Refetch` would have to be
  added to `counts()` by hand, which is how `Skipped` came to be counted in
  the header total and missing from the tally beside it.
- **A converted track goes back to `Have`, never through `identify`.** The
  tags come off the old file and are written onto the new one, so sending it
  to the lookup afterwards would replace a hand-typed correction with a guess.
  `tag_tracks`'s `Have` branch keeps a `source` the same pass already set, or
  "on disk" overwrites the one word saying this run rewrote the file.
- **Nothing is deleted until the replacement has been read back.** A video
  pulled from YouTube since the first download cannot be fetched again. The
  read-back is load-bearing only when the old file has no readable tags:
  otherwise writing them onto a bad replacement fails first and the swap ends
  there anyway, which is what made the first version of that test pass against
  its own bug.
- **The replacement has to be the format that was asked for, not merely
  readable.** On the refetch path it is whatever yt-dlp wrote, so a
  postprocessor falling back to the source container would be renamed,
  recorded in the sidecar and reported as converted while being nothing of
  the kind, and `format_on_disk` would then say `Leave` about it on every
  later sync: the row claims success for good. The target name takes its
  extension from `config::extension` for the same reason.
- **`swap_in` hands the old file back rather than removing it, and the caller
  moves the sidecar first.** Killed in between, the manifest names the
  replacement, which is on disk, and the next sync sees a track already in the
  target format and leaves it alone. Removed first, the manifest names a file
  that has gone.
- **A name already taken is this pass's own leftover unless a track holds
  it.** An interrupted swap renames the replacement into place and dies before
  the sidecar moves, so refusing every collision wedged the track: every later
  sync planned the same conversion, hit the guard, and on the refetch path
  fetched a file and threw it away, for good. `files_held` is what tells the
  two apart, the replacement is logged when it happens, and the refusal names
  the next step because the reader otherwise has to work out that renaming one
  of them is the answer.
- **The pick gate governs the conversion too.** It only ever marks unpicked
  `Pending` tracks `Skipped`, which says nothing about what is already on
  disk, so a run where somebody asked for two new tracks also re-downloaded
  every mp3 in the folder with nothing on that screen mentioning it.
  `ask_which` returns its answer and `to_bring` honours it. `--resync` passes
  no gate and so converts the whole folder, which is the point of it.
- **Both halves report what they could not do.** A folder where every
  transcode failed for want of an encoder reported as a clean sync, because
  only the successes were counted. The refetch half separates "never arrived"
  from "arrived and could not be used": one message covering both told the
  reader the wrong one half the time.
- **The swap renames over the old file rather than removing it first.**
  `fs::rename` replaces atomically, so the `alac`-over-`m4a` case never has a
  moment with no file in it. An earlier version removed the old one first for
  no gain.
- **Tags go on through lofty, never ffmpeg's `-map_metadata`.** `with_tag`
  knows which tag type each container takes; ffmpeg's field mapping across
  containers is inconsistent in exactly the way this codebase has already paid
  for once. `-vn` drops the picture for the same reason and `set_cover` puts
  it back.
- **`tag::Kept` is the one list of what survives a file swap.** It reads the
  identity fields, the lyrics, the ISRC and the picture before the file can
  change, and `write` puts them on the replacement; `swap_in` and export's
  `reencode` both go through it, so a tag added to the tool is carried by
  adding it there. Loudness is left out on purpose: R128 and ReplayGain are
  different tags against different reference levels, and the next sync
  measures the new file. Before it, a conversion carried artist, title, album,
  year and the picture and nothing else: lyrics somebody typed were gone for
  good, which breaks "lyrics are never replaced", and on the refetch path the
  ISRC went too, leaving a `#isrc` line the file no longer backed. `swap_in`
  also drops that line if the replacement still has no ISRC. Pinned by
  `a_swap_carries_the_lyrics_and_the_isrc_too` and
  `a_converted_export_keeps_the_lyrics_and_the_isrc`; the sidecar-drop line has
  no test, since `Kept::write` keeps the ISRC whenever there was one.
- **A lossy source never becomes a 24-bit lossless file.** ffmpeg decodes
  opus to float and then writes 24-bit flac or alac, which is twice the size
  to store the decoder's own rounding: on a real folder that was 436MB against
  235MB for the same audio. Forced only when the source format is known and
  lossy, so a genuinely deeper file is not quietly flattened. The two encoders
  disagree about the spelling and each refuses the other's, `s16` for flac and
  `s16p` for alac.
- **`FORMATS` carries the encoders, and the fixtures read the same column.**
  The name is not the same on every ffmpeg build, which is not hypothetical:
  this machine has no `libvorbis` and falls through to the native `vorbis`.
  A second copy of that list in the test file is one that goes stale.
- **`transcode` writes its stderr to a file, not a pipe.** `run_bounded` polls
  `try_wait` and drains nothing until the child has exited, so a pipe that
  fills its buffer hangs ffmpeg until the deadline kills it. Its stdin is
  `null` as well, and so is `shrink`'s: an inherited one lets ffmpeg read the
  terminal earworm is holding in raw mode and eat the keys meant for the UI.
- **`counts()` is built from the tracks, not from a list of statuses.**
  `tally()` is exhaustive and decides both whether a status is counted and
  where it sits, so a new one is either given a column or deliberately left
  out and neither can be forgotten. The first attempt read a hand-kept array
  here and dropped a whole column with nothing failing, which docs/history.md
  records. `TALLY` is now
  `#[cfg(test)]` and nothing in the running tool reads it, so forgetting an
  entry costs coverage rather than correctness.
- **`save_format` edits one line and re-parses before writing.** The config is
  hand-edited, so rebuilding it from the parsed struct would drop every
  comment. A file earworm can no longer read is worse than one that never
  recorded the choice, since the next start fails outright.
- **Only `ErrorKind::NotFound` means the config is empty.** `unwrap_or_default`
  on the read turned a permission denial or a stray non-UTF-8 byte into "the
  file was empty", and the save then replaced a whole hand-written config with
  one line while reporting success. An absent file is the one case that writes
  a one-line config.
- **`extension` never guesses.** It used to fall back to `opus`, so a typo in
  `FORMATS` would predict the wrong filename for every track in a folder: the
  scan would call all of them absent and download the lot again beside the
  copies on disk. `Config::build` refuses anything not in the table and
  `set_format` picks out of it, so an unknown name here is a bug in this file
  and says so.
- **The format is saved before the run adopts it.** A failed save left the
  session on the new format while the flash said nothing had been remembered,
  so the next download agreed with neither the message nor the file.
- **A quoted key is the same key.** TOML allows `"format" = …`, which
  `save_format` read as some other key and appended a second `format` to. That
  is a duplicate, the re-parse refused it, and every later save failed the same
  way: the setting could never be written again.
- **`save_format` re-parses the file before editing it too.** A config that was
  already broken is not something the edit did, and "the edited config no
  longer parses" sends the reader to the wrong line.
- **One replace for every file earworm owns.** `config::write_atomically` is
  what both the config and the update cache go through. A torn cache costs
  only an extra probe, so it could have kept `fs::write`, but two answers to
  "how does earworm replace a file" is how the weaker one gets copied into
  the next place that cannot afford it.
- **The write is a temp file renamed over the resolved path.** `fs::write`
  truncates first, so a crash between the truncate and the write leaves a
  config that will not load. The rename follows the symlink, because a config
  kept in a dotfiles repo would otherwise be replaced by a plain file and every
  later change in the repo would stop arriving; and it copies the old file's
  mode, because a fresh temp file is world-readable and this file holds an
  AcoustID key in plain text.
- **The scan's format is tested through a stub yt-dlp, not through
  `scan_template`.** Asserting on the helper left the call site free to pass
  anything: reverting `.arg(scan_template(cfg))` to a hardcoded `opus` kept the
  suite green while every track in a flac folder looked absent. `scan_with`
  takes the binary so a test can read back the arguments the scan actually
  ran with, the same way `player`'s `*_with` functions do.
- **`f` is library-only, like `space`.** It follows the active track on the
  track list, and the setting is about the next playlist rather than the one on
  screen. `--no-config` leaves `Config.config_file` as `None`, and the flash
  then says the choice lasts one session.
- **`Msg::Settings` exists because `App.settings` is built once at startup.**
  Change a setting from inside the tool without re-sending it and the help
  overlay keeps reporting the format the run began with.
