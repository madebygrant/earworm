# Loudness tags


- **`loudness` is off until the file turns it on.** It goes through `opt_in`
  like `convert`, because the pass runs on tracks already on disk and rewrites
  their tags: on by default, the next `--resync` would touch every file in
  the library. The flag only ever switches it off.
- **The two reference levels are different on purpose.** Opus is
  `R128_TRACK_GAIN`, a Q7.8 integer against -23 LUFS, and nothing else.
  Everything else is `REPLAYGAIN_TRACK_GAIN` in dB against -18 plus a peak.
  Opus players ignore ReplayGain, and writing both would give them two
  answers. `the_two_reference_levels_are_not_swapped` pins the numbers.
- **`write_loudness` skips a file whose track gain reads back.** A resync then
  costs a tag read and no ffmpeg, and a value another tool wrote stays. One that
  does not parse (`-3,2 dB`) is measured and replaced: left, it kept the track
  out of every album mean for good and logged the skip on every sync
  (`a_gain_that_reads_back_is_kept_and_one_that_does_not_is_measured_again`).
  Album gain is the opposite: it is rewritten when the computed value differs,
  because adding a track changes it for every file. That includes an album gain
  another tool wrote, which earworm cannot tell from its own; the alternative
  is an album gain that goes stale when a track is added, and this was
  chosen. The album peak is written only when every track has a peak, since
  the largest of a few understates it
  (`an_album_peak_is_written_only_when_every_track_has_one`).
- **Album gain is read back from the track tags, not measured again.**
  `stored_loudness` inverts the gain, so the album pass decodes nothing. It
  depends on the track pass having run first, which is why it sits after
  `tag_tracks` in `pipeline` and `retry`, and only when the folder is an
  album: a retried track changes the mean for every file. Nothing tests the
  `retry` call, which needs a live download.
- **Opus albums have no peak.** R128 has no peak tag to write, and inventing
  one in a ReplayGain tag would give opus players a second answer.
- **`measure` writes ffmpeg's stderr to a file.** The summary is on stderr,
  and `run_bounded` drains no pipe, for the reason `transcode` documents.
  Silence prints `-inf`, which parses as a number, so `parse_loudness` refuses
  non-finite values or a silent file would be written a gain of infinity.
- **A measuring failure is a log line, never a status.** It costs one tag.
  Marking the track `Failed` would report a downloaded, tagged file as broken.
- **`Extra::key` picks the key from the tag type.** lofty has no `Lyrics`
  mapping for ID3v2, only `UnsyncLyrics`, and `set_extra` reports a key the
  container cannot hold instead of dropping it.
- **`lyrics` is off until the file turns it on,** for the reason `loudness`
  is, and because it is the one setting that makes a request to a third party
  for every track.
- **`enrich_with` asks for lyrics only after reading the tag.** A track that
  has words never costs a request, a rerun is free, and lyrics the user typed
  or another tool wrote are never replaced. The fetch is a closure so the test
  counts requests without a network, as `tag::sleeve` does.
- **LRCLIB's `get` needs an album and a duration, and most playlist tracks
  have no album.** `lookup::lyrics` tries `get` only when there is an album and
  otherwise searches by artist and title. A search result has to be within two
  seconds of the track's length *and* by the same artist and title, because
  length alone lets a cover of the same song through and lyrics are never
  overwritten once written. No duration means no lookup at all. An exact match
  that says instrumental ends the lookup: searching on could attach another
  record's words. The album sent is the file's own: `real_album` blanks it
  when it equals the playlist name, which is the album fill and would only
  ever miss. An error on `get` falls through to the search unless it was the
  strike that paused the breaker.
- **A miss and an outage are different answers, and `Lookup` says which.**
  `lrclib_json` reads a 404 as `Missing` and anything else as `Down`;
  `get_json` cannot, which is why the lyrics calls do not use it. Both leave
  the tag empty and the next sync asks again, and nothing records that a track
  was tried, deliberately: LRCLIB grows. The summary tells them apart, since
  "nothing there" and "did not answer" call for different reactions.
- **`Breaker` stops asking after three failed requests in a row, for five
  minutes.** Every track asks up to twice and the lyrics timeout is eight
  seconds, so a down service would otherwise cost a library the length of the
  sync in timeouts, with the cancel check only reached between tracks. A 404
  and a success both reset the count, or a library of tracks LRCLIB lacks would
  trip it. `lyrics_via` takes the request as a closure so the tests stand in
  for the network and count what was asked; the pause is checked against an
  `Instant` the test supplies. `Lookup::Skipped` (no length, no name) is not
  counted at all, or the summary would claim searches that never happened.
- **A miss is counted, never flagged.** `tag_tracks` returns an `Extras` tally
  and `pipeline` and `retry` append `said()` to the summary. A track with no
  lyrics is a track LRCLIB lacks, so marking it `Failed` would report a good
  file as broken. Tracks that already had words are not in the count, or "9 of
  12" would claim searches that never happened. `retry` appends it too, since
  it rebuilds the summary rather than adding to the run's. Nothing tests that
  the two call sites append it: `pipeline` needs a live scan.
- **Plain text only.** `lyrics_of` prefers `plainLyrics` and strips the
  timestamps from `syncedLyrics` when that is all there is, because a plain
  tag shows `[01:23.45]` as text in any player that does not read LRC. An
  instrumental writes nothing.
