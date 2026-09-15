# earworm

**Playlist in, library out.**

A terminal app that turns a YouTube playlist into a folder of properly tagged,
properly named, properly illustrated audio files. Not 40 downloads with
`(Official Video)` baked into the artist name — a music library your player
trusts.

![A playlist open in earworm: the track list on the left, the selected track's
tags and file path in a pane on the right, and the run's settings along the
header](docs/images/earworm.png)

```sh
brew install yt-dlp ffmpeg chromaprint
cargo install --path .
earworm 'https://youtube.com/playlist?list=...'
```

## Features

yt-dlp already downloads. earworm does everything that comes after, which is
the part that decides whether the result sounds like a library or like a pile
of files:

- **Tags you can trust.** earworm splits artist and title out of the video
  title, then *checks* that guess — fingerprint against AcoustID, search
  against Deezer, then Apple Music. `guessed` is a status word, not a silent
  default, and when nothing fits, earworm asks instead of writing a tag it
  doesn't believe.
- **Cover art, embedded and sane.** Every track carries its art inside the
  file, capped at 1000×1000 — because YouTube's auto-generated albums serve
  2.9 MB of 2048² thumbnail that ends up in all twelve tracks.
- **It remembers.** Every playlist is a folder with an `.m3u8` and a
  `.earworm` manifest recording the URL. Re-running skips what it has,
  re-syncs what's new, and reports what left the playlist. No download
  history to re-walk.
- **A library, not a log file.** Open any folder from disk, fix a tag
  downloaded weeks ago, rename a playlist and keep the rename through the
  next sync, search every track you own across every folder.
- **Transcodes that don't throw away work.** Switch format and old tracks
  come along on their next sync, tags and art intact — in whatever way
  costs the fewest generations of lossy encoding, which is a genuinely
  strange and interesting table. See [docs/formats.md](docs/formats.md).
- **It plays.** With [cliamp](https://docs.cliamp.stream) running, one key
  hands the playlist over, with the right order and the tags showing.
- **Nothing is so settled you can't change it.** Edit, undo, re-search,
  retry. One key each.

## The shape of a run

Type a URL, and the run pauses between reading the playlist and downloading
it to let you choose what it fetches:

```
 PICK  158 of 361 selected       space  ·  a all  ·  enter downloads  ·  esc none
```

The gate exists because you can't know whether you want the whole thing until
you see it has 361 tracks. Everything else — extraction, lookup, tagging,
cover art, the playlist file, the library, retry — is covered by the guides
below. The defaults are ones worth keeping.

## Where to go next

| | |
| --- | --- |
| [docs/install.md](docs/install.md) | tools, the AcoustID key, `--check` |
| [docs/configuration.md](docs/configuration.md) | the config file, every flag |
| [docs/workflow.md](docs/workflow.md) | picks, filters, fixing tags, retry |
| [docs/formats.md](docs/formats.md) | opus, mp3, flac, and what converts |
| [docs/library.md](docs/library.md) | the library screen, search, `--resync` |
| [docs/playback.md](docs/playback.md) | playing through cliamp |
| [docs/keys.md](docs/keys.md) | every key, both screens, the status words |
| [docs/internals.md](docs/internals.md) | how tagging decides, what gets written |

## Who it's for

You, if you download playlists and then rename things by hand. earworm was
built on a real music library and behaves the way a library behaves: it
answers questions about itself (`--list`, `--check`), it never deletes what
you didn't ask it to, and when it's unsure it says so on screen instead of
being quietly wrong on every one of 40 tracks.

## Licence

MIT or Apache-2.0, at your option.
