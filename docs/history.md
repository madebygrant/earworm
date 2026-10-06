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
