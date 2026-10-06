# What proved it

Evidence behind rules in CLAUDE.md, and trade-offs taken with eyes open. It
lives here rather than there because CLAUDE.md is read at the start of every
session and this is read when somebody doubts a rule, which is rarer and worth
the extra click. Nothing here is a rule. Every rule stayed where it was.

## Decomposed filenames in the `.m3u8`

Confirmed in VLC on iOS with one Korean track listed four ways in one file:
decomposed plays, composed does not, and percent-encoding changes neither,
which is what rules out the `.m3u8` URI rules as the cause. APFS ignores the
difference on lookup, so nothing about this reproduces on the machine that
writes the file.

The cost is the opposite case. On a byte-exact filesystem holding composed
files, Linux or Android, decomposed is the wrong form and the entries name
nothing there. Chosen rather than overlooked, on one user who is on Apple
hardware, and the point at which this becomes a setting rather than a
constant.

## The half-converted folder

Reproduced on a real folder: six of twelve converted, six dead entries, six
departures.

## `lookup::stop` and the latch

Verified by hand, like `ytdlp::stop`, which has no test either: 417ms and a
SIGTERM against a 60s bound, and the retry refused in microseconds.

## `counts()` and the hand-kept `TALLY`

The first attempt kept a hand-kept `TALLY` array and read it from `counts()`,
which still compiled with a variant missing and still dropped a whole column.
Proved by adding one.

## Measuring every palette

The first version of `theme()` ran `unreadable()` only on the `[colors]` path,
so a `[themes.*]` palette, just as unmeasurable by the suite, could be
illegible with nothing said.

## `save_key` and the first table

Shipped with `[colors]`, and found by a round-trip test written for something
else.

## The conversion table

The first version of it had `opus` and `m4a` both wrong, on the assumption
that a container YouTube serves is a container yt-dlp picks. `yt-dlp -f
bestaudio --print %(acodec)s` is what settled it.

## Splitting a video by its chapters

Measured on yt-dlp 2026.08.19 against a 757s album upload with six chapters
and against a three-chapter, nineteen-second video. Nothing here is built
yet. It is what a design has to start from.

- `--split-chapters` keeps the whole-video file and writes the chapters beside
  it, under the `chapter:` output template (`section_number`, `section_title`).
  The full file was 12.5MB beside five tracks of 2 to 3MB.
- No hook reports the chapter files. `--print after_move:`, `--exec` at
  `after_move` and `--exec` at `post_process` each fired once, for the whole
  video, and `after_video:` printed `NA`. The `section_*` fields are `NA` in
  that line. earworm would have to predict the names, and yt-dlp sanitises
  them: `<Untitled Chapter 1>` became `＜Untitled Chapter 1＞`, full-width.
- `%(chapters)j` is in the `--flat-playlist` listing for a single video, so no
  second, non-flat probe is needed. A playlist entry prints `NA`, and so does
  a video with no chapters. The JSON has no tab or newline, so it fits the
  scan's tab-separated `--print` as one more field.
- yt-dlp invents an `<Untitled Chapter 1>` for any gap before the first
  chapter. The album started its first chapter at 1s, so the split wrote a
  1s, 17KB first track and numbered the real tracks from 02.
- A video with two or more chapters is not an album. `Me at the zoo`, a
  nineteen-second clip, has three. A rule of "chapters means split" would cut
  every tutorial and podcast with timestamps into tracks, so splitting has to
  be asked for.
- ffmpeg stream copy (`-ss`, `-to`, `-c copy`, `-map_metadata -1`) cut a
  179s span to within 40ms in opus, mp3 and m4a. In flac it ignored the range
  and wrote all 757s, because a file with no seek table cannot be cut by copy.
  A flac or alac cut has to re-encode, which is lossless there. alac was not
  tried.
- ffmpeg warns "Referenced QT chapter track not found" on an m4a that carries
  chapter metadata. The cut still came out right.

## Library icons

- Not measured: the glyph table was chosen from the plan's candidates without
  rendering them in a terminal. `●`, `▤` and `◆` are one cell wide by
  `unicode-width`, which `every_glyph_is_one_cell_wide` checks, and that is
  not the same as a font drawing them one cell wide.
- Not measured: `tb-tui-common`'s `font_probe`, so there is no detection.
- The `nerd` tier was added after `●▤◆` still drew small. Codepoints are
  `md-album` f0025, `md-playlist_music` f0cb8 and `md-folder_music` f1359,
  read from Nerd Fonts' glyphnames.json. Not yet looked at in a patched font.
- `◉`, `≡` and `⌂` drew small in the first real terminal, so they became `●`,
  `▤` and `◆`, bold. `☰` was tried and is two cells wide by `unicode-width`.

## Songs on more than one playlist

- Deezer's `/search` hit carries `isrc`: `Autobahn (2009 Remaster)` by
  Kraftwerk came back with `GB01A0900374`, on a live request on 2026-10-06. No
  extra request is needed. iTunes and AcoustID were not checked and carry none.
- A third column on a sidecar entry would be read by an older binary as part of
  the filename, so per-track data went in `#isrc` lines, which `load` skips by
  the `#` rule.
- Not done: the Art Track versus video investigation, the `OLAK5uy_` album page
  check and the `fpcalc` thresholds. The first two are only needed for the
  Art Track preference, the last for the fingerprint comparison, and neither of
  those was built.
- Not tried in a live sync: `copy_known` has only run against fixtures.

## Art Track versus music video

Measured 2026-10-06 on `Kraftwerk Autobahn` search results, against
`vkOZNJYAZ7c` (the Art Track, channel `Kraftwerk`) and `iukUMRlaBBE` (a
fan upload of the video).

- The flat listing tells them apart by nothing. `media_type` and `availability`
  print `NA` for both, and the channel is the bare artist name on one and an
  ordinary channel on the other, which is what CLAUDE.md already says of the
  uploader.
- A full per-video extraction does. The Art Track has `track`, `album` and
  `artist` set and a description opening `Provided to YouTube by Parlophone UK`;
  the video has all of them null. That is one request per video, so it is only
  affordable for the rows `mark_twins` has already flagged, never for a
  whole playlist.
