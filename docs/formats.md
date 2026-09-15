# Audio formats

## Choosing one

Opus by default. `--format` takes `opus`, `m4a`, `mp3`, `flac`, `vorbis` or
`alac`. wav and aac are left out because neither carries tags or cover art.

Typing a URL offers the format before the download starts. The choice is
written to the config, so the next playlist arrives the same way, and `f` on
the library changes it later.

## Converting what you already have

Changing the format converts nothing already on disk, so a folder you switch
format on ends up holding both. Turn `convert` on and a sync brings the old
tracks across too. That is deliberately a second setting. Picking a format is
one keystroke, and if it alone rewrote what was already on disk, the next
`--resync` would run over every folder under `--dir` a keystroke later.
earworm asks about it once, right after you pick a format.

With it on, each playlist comes across the next time it syncs. Tracks already
in the target format are left alone, as are ones whose video has left the
playlist. Most of the rest are re-encoded in place.

Three aren't. YouTube hands yt-dlp an Opus stream, so `m4a`, `mp3` and
`vorbis` on disk have already been re-encoded once, and converting them again
would cost a third generation where a fresh download costs two. Those get
downloaded. So does anything on its way *to* `opus`, since opus is the one
format a download copies rather than re-encodes.

Tags, cover art and any name you typed all come across, and nothing is deleted
until the replacement is written and read back. A track whose download fails
keeps the file it had and says so.

Converting to `flac` or `alac` recovers no quality, because YouTube never
served any. It buys you a format your player can read, at three times the
size.
