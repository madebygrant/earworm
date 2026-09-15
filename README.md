# earworm

**Playlist in, library out.**

A terminal app that turns a YouTube playlist into audio files with verified tags, cover art and an .m3u8.

![A playlist open in earworm: the track list on the left, the selected track's
tags and file path in a pane on the right, and the run's settings along the
header](docs/images/earworm.png)

```sh
brew install yt-dlp ffmpeg chromaprint
cargo install --path .
earworm 'https://youtube.com/playlist?list=...'
```

## Features

yt-dlp downloads the audio. earworm does the rest, which is what separates a
download from a music library:

- **Tagging that checks itself.** Artist and title are parsed out of the video
  title, then confirmed against AcoustID fingerprints, Deezer and Apple Music.
  A track that survives none of that is marked `guessed`, and earworm asks
  about it rather than writing a tag it can't back up.
- **Cover art embedded at a sane size.** YouTube serves 2048² thumbnails for
  its auto-generated albums. earworm keeps them out of your files: what gets
  embedded is capped at 1000×1000.
- **Re-runs skip work you already did.** Each playlist folder keeps an
  `.earworm` manifest with its URL and current filenames. Sync it again and
  earworm fetches only the new tracks, reports the ones that left the
  playlist, and reads the tags it already wrote instead of looking them up.
- **Fix anything weeks later.** Every folder opens from disk, offline. Edit
  tags, swap artist and title, search `/` across every folder you own, rename
  a playlist and have the rename survive the next sync.
- **Change format without throwing away what you have.** Tracks already on
  disk convert on their next sync in whatever way costs the least re-encoding.
  Why some never re-encode at all is [documented](docs/formats.md), because
  the answer surprised me too.
- **Plays through [cliamp](https://docs.cliamp.stream).** `p` hands over the
  playlist in the right order, tags as track titles.
- **Retries and undos.** `r` re-downloads whatever failed. `u` puts back the
  last tag edit.

## How a run goes

Give it a URL and earworm does the whole job: read the playlist, download,
identify, tag, add art, name files, write the `.m3u8`. One step waits for
you. Before downloading, the run shows the list and asks which tracks you
want.

Enter takes all of them. Esc takes none. `space` and the `/` filter are there
for everything in between, which you will need the first time a playlist
turns out to be 361 tracks of seminar recordings.

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

People who download playlists and then rename the files by hand. earworm
doesn't delete anything you didn't ask it to, shows you when it isn't sure
about a tag, and answers `--list` and `--check` in plain text so scripts can
use them too.

## Licence

MIT or Apache-2.0, at your option.
