# earworm

A terminal UI for turning a YouTube playlist into a folder of tagged opus files.

`yt-dlp` does the downloading. earworm handles everything after it: parsing
artist and title out of video titles, confirming them against AcoustID and
Deezer, writing Vorbis tags, fetching cover art, and generating an `.m3u8`.
Tracks that can't be resolved automatically get a prompt instead of a wrong
tag, and you can fix any track's artist, title or cover after the run without
leaving the app.

## Requirements

Three external binaries, all on PATH:

| Binary | Homebrew formula | Used for |
| --- | --- | --- |
| `yt-dlp` | `yt-dlp` | downloading and extracting audio |
| `ffmpeg` | `ffmpeg` | opus conversion, thumbnail embedding |
| `fpcalc` | `chromaprint` | acoustic fingerprinting (optional) |

```sh
brew install yt-dlp ffmpeg chromaprint
```

Fingerprint lookups also need a free [AcoustID](https://acoustid.org/new-application)
key:

```sh
export ACOUSTID_API_KEY=your-key
```

Without it earworm falls back to Deezer search alone, which is noticeably worse
at matching. Nothing else changes.

## Install

```sh
cargo install --path .
```

## Use

```sh
earworm                                  # asks for a URL
earworm 'https://youtube.com/playlist?list=...'
earworm URL --dir ~/Music/new
earworm URL -- --cookies-from-browser firefox   # extra args go to yt-dlp
```

Files land in `<dir>/<playlist name>/NN - Title.opus`, alongside `cover.jpg`
and `<playlist name>.m3u8`.

### Flags

| Flag | Effect |
| --- | --- |
| `-d`, `--dir` | output directory (default `~/Music`) |
| `-P`, `--no-parse` | keep YouTube's own artist/track, skip title parsing |
| `-L`, `--no-lookup` | skip AcoustID/Deezer, keep parsed tags |
| `-C`, `--no-cover` | don't write `cover.jpg` |
| `-M`, `--no-m3u8` | don't write the playlist file |
| `-F`, `--no-fix` | never prompt; unresolved tracks keep parsed tags |

`-L -F` is the fast path: download and tag from titles, no network lookups, no
questions.

### Keys

| Key | Action |
| --- | --- |
| `j` `k` `↑` `↓` | move |
| `g` `G` | first, last |
| `f` | follow the active track |
| `l` | toggle the yt-dlp output pane |
| `e` | edit artist and title (after the run) |
| `c` | choose cover art (after the run) |
| `h` `?` | keys and current settings |
| `q` `Esc` `ctrl+c` | quit |

## How a track gets its tags

1. **Parse.** The video title is split on its separator. A dash means
   artist-first, a pipe or bullet means artist-last.
2. **Fingerprint.** `fpcalc` plus AcoustID. Accepted at score 0.8 or better.
3. **Search.** Deezer, matched on normalised artist and title.
4. **Ask.** Anything still unresolved gets a prompt with the candidates found,
   unless `--no-fix`.

The status column says which route a track took: `ok` for a confident match,
`weak` for a low-confidence one, `manual` for an answered prompt, `kept` for
parsed tags left alone, `none` when nothing matched.

Re-running over a folder is cheap. Already-downloaded ids go into a yt-dlp
archive so they're skipped, and tracks already tagged are read from disk rather
than looked up again.

## Notes

- Only downloads what you have the right to download.
- Editing tags never renames files, so an existing `.m3u8` stays valid.
- Quitting kills the yt-dlp child; a tag write in progress is allowed to finish.

## Licence

MIT or Apache-2.0, at your option.
