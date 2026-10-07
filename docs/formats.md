# Audio formats

## Choosing one

m4a by default. `--format` takes `m4a`, `opus`, `mp3`, `flac`, `vorbis` or
`alac`. wav and aac are left out because neither carries tags or cover art.

m4a plays everywhere, which is why it leads. Most YouTube videos carry an AAC
stream as well as the Opus one, and an m4a download takes that stream as it
is, with nothing re-encoded. A video without one falls back to the Opus,
re-encoded to AAC. The AAC stream is 128k (256k with a Premium account's
cookies), and Opus at a similar rate usually sounds a little better, so choose
`opus` for the copy closest to the source.

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

Three aren't. `mp3` and `vorbis` on disk have already been re-encoded once
from YouTube's Opus, and an `m4a` may have been, and converting them again
would cost a third generation where a fresh download costs two. Those get
downloaded. So does anything on its way *to* `opus` or `m4a`, since those are
the formats a download copies from YouTube rather than re-encodes.

Tags, cover art and any name you typed all come across, and nothing is deleted
until the replacement is written and read back. A track whose download fails
keeps the file it had and says so.

Converting to `flac` or `alac` recovers no quality, because YouTube never
served any. It buys you a format your player can read, at three times the
size.
