# earworm

A terminal UI for turning a YouTube playlist into a folder of tagged opus files.

`yt-dlp` does the downloading. earworm does everything after. It splits artist
and title out of the video title, checks that guess against AcoustID and
Deezer, writes Vorbis tags, fetches cover art and writes an `.m3u8`. When it
can't work out a track it asks rather than writing a tag it doesn't believe,
and you can still fix any track's artist, title or cover after the run.

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

Without one earworm falls back to Deezer search alone, which matches a lot
less well. Nothing else changes.

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
earworm --resync                         # every playlist already downloaded
```

Tracks land in `<dir>/<playlist name>/`, named `NN - Artist - Title.opus` once
earworm is confident of the tags, next to a `cover.jpg` (or `.png`) and
`<playlist name>.m3u8`.

Run without a URL and earworm asks what to do. If you already have playlists
downloaded it offers to resync them all; otherwise it goes straight to asking
for a link. It takes `youtube.com`, `youtu.be`
and the `m.` and `music.` subdomains, with or without the `https://`, and the
link has to name a video, playlist or channel. Anything else comes back for
editing rather than being rejected outright. Esc at that prompt quits.

### Flags

| Flag | Effect |
| --- | --- |
| `-d`, `--dir` | output directory (default `~/Music`) |
| `-P`, `--no-parse` | keep YouTube's own artist/track, skip title parsing |
| `-L`, `--no-lookup` | skip AcoustID/Deezer, keep parsed tags |
| `-C`, `--no-cover` | don't write `cover.jpg` |
| `-M`, `--no-m3u8` | don't write the playlist file |
| `-F`, `--no-fix` | never prompt; unresolved tracks keep parsed tags |
| `-R`, `--no-rename` | keep yt-dlp's filenames instead of renaming from tags |
| `-A`, `--no-album` | don't fill an empty album tag with the playlist name |
| `--resync` | sync every playlist already under `--dir` |
| `--config PATH` | read this config file instead of the default one |
| `--no-config` | ignore the config file entirely |

`-L -F` is the quick one: download and tag from titles, no network lookups, no
questions.

## Configuration

`~/.config/earworm/config.toml` (or `$XDG_CONFIG_HOME/earworm/config.toml`)
holds the settings you'd otherwise retype every run. Every key is optional.

```toml
dir = "~/Music/playlists"

parse = true       # split artist and title out of the video title
lookup = true      # confirm against AcoustID and Deezer
cover = true       # write cover.jpg beside the tracks
m3u8 = true        # write the playlist file
fix = true         # prompt when a track can't be resolved
rename = true      # rename files from the corrected tags
album = true       # fill an empty album tag with the playlist name

extra = ["--sleep-requests", "1"]   # always passed to yt-dlp
acoustid_key = "..."                # or use ACOUSTID_API_KEY
```

Command-line flags win over the file, and the file wins over the defaults
above. Every flag is a negation, so the command line can only switch a feature
off. To run with something the file disables, edit the file.

`extra` from the file comes before anything after `--` on the command line, so
a repeated yt-dlp option takes its command-line value.

`ACOUSTID_API_KEY` overrides `acoustid_key`, which is handy for one run with a
different key. A key in the file sits there in plain text, so
`chmod 600 ~/.config/earworm/config.toml` if the machine has other users.

An unknown key is a startup error rather than a silent no-op, so a typo tells
you instead of quietly ignoring the setting.

## Keys

| Key | Action |
| --- | --- |
| `j` `k` `↑` `↓` | move |
| `g` `G` | first, last |
| `f` | follow the active track |
| `l` | toggle the yt-dlp output pane |
| `e` | edit artist and title (after the run) |
| `c` | choose cover art (after the run) |
| `r` | retry every failed track (after the run) |
| `space` | mark or unmark the track under the cursor |
| `m` | mark every track sharing that track's status |
| `s` | swap artist and title on the marked tracks |
| `A` | set one artist across the marked tracks |
| `h` `?` | keys and current settings |
| `q` `Esc` `ctrl+c` | quit |

Keys that change files go quiet while one is running; the header spinner
comes back to say why.

### When a run finishes

A prompt says what happened and offers the two things worth doing next:

```
┌ finished ────────────────────────────────────────────────────────┐
│ 12 tracks in ~/Music/Chill Evenings, playlist Chill Evenings.m3u8│
│                                                                  │
│ ▌ keep this open                                                 │
│   sync another playlist                                          │
│   quit                                                           │
└──────────────────────────────────────────────────────────────────┘
```

Keeping it open is first, so Enter never starts work or ends the session, and
Esc does the same. That leaves the keys below available for fixing tags.
Another playlist asks for a URL and starts over in the same session; cancelling
that question comes back here rather than dropping out.

### Fixing several tracks at once

`space` marks a track. `m` marks every track sharing the status of the one
under the cursor, so grabbing all the `kept` rows takes one keypress, and a
second `m` gives them back. `s` and `A` then act on the whole marked set. `s`
swaps artist and title, for a playlist whose titles all parsed the wrong way
round; `A` sets one artist across them, for a compilation the lookup split
into different names. With nothing marked, both act on the track under the
cursor.

Marks survive a cancelled prompt and clear once an action has written
something. Bulk edits rename and rewrite the playlist just as a single edit
does.

### Syncing everything at once

Each playlist folder records the URL it came from in its `.earworm` file, so
`earworm --resync` walks `--dir`, finds the folders that know their own URL and
syncs each one in turn. No arguments, no list to maintain. New tracks download,
departed ones get reported, and a playlist that has since gone private is
logged and skipped rather than stopping the rest.

Folders downloaded before earworm started recording the URL have no entry, so
they need one sync by URL before `--resync` picks them up. Playlists are
visited in folder-name order, and the rows on screen are one playlist at a
time, since a track's number is its position in its own playlist.

### Retrying failures

Most failures are a network blip or a video that was briefly unavailable, so
`r` re-downloads and re-tags every `failed` track in one pass, leaving the
rest alone.

## How a track gets its tags

1. **Parse.** Split the video title on its separator. A dash means
   artist-first, a pipe or bullet means artist-last.
2. **Fingerprint.** `fpcalc` plus AcoustID, accepted at score 0.8 or better.
3. **Search.** Deezer, matched on normalised artist and title.
4. **Ask.** Anything still unresolved gets a prompt listing the candidates,
   unless `--no-fix`.

The status column says which route a track took: `ok` for a confident match,
`weak` for a low-confidence one, `manual` for an answered prompt, `kept` for
parsed tags left alone, `none` when nothing matched, `gone` for a file whose
video has left the playlist.

## What it writes

earworm renames a track it identified confidently, or that you typed the tags
for, to `NN - Artist - Title.opus`. Tracks it was unsure about keep the name
yt-dlp gave them, because a `weak` or `kept` guess baked into a filename looks
more decided than it is. It never writes over a file that already exists.

Renaming would break the next run's already-downloaded check, which works by
predicting the filename. So earworm keeps a `.earworm` file in the
folder mapping each video id to the file it ended up in, which also survives a
video being retitled on YouTube. Delete a track and it downloads again; delete
`.earworm` and every renamed track downloads again.

Re-running is cheap either way: earworm skips ids it already has and reads
its own tags off the disk instead of looking them up again.

A file whose video has since left the playlist gets a `gone` row and a line in
the summary. earworm never deletes it, keeps remembering it in `.earworm` so a
video that comes back doesn't download twice, and leaves it out of the
`.m3u8`, which now matches the playlist.

That check needs a listing that asked for the whole playlist and got all of
it, so earworm stays quiet about departures when yt-dlp didn't finish listing,
or when you passed extra yt-dlp arguments. `--playlist-items 4` makes every
other track absent from the listing while it sits in the playlist untouched,
and yt-dlp exits 0 either way, so the arguments are the only clue. It logs how
many files it skipped over rather than guessing.

The album tag gets the playlist name only when the lookup found no real album,
since the release a track actually came from is the better answer.

On exit earworm prints the folder it wrote to.

## Notes

- Only downloads what you have the right to download.
- Editing tags renames the file and rewrites the `.m3u8` to match.
  `--no-rename` turns that off.
- Quitting kills the yt-dlp child, and lets a tag write already under way
  finish.

## Licence

MIT or Apache-2.0, at your option.
