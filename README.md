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
| `cliamp` | `cliamp` | playing a finished playlist (optional) |

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
earworm URL --no-pick                    # don't stop to choose, fetch it all
earworm URL -- --cookies-from-browser firefox   # extra args go to yt-dlp
earworm --resync                         # every playlist already downloaded
earworm --list                           # what is already on disk, then exit
```

Tracks land in `<dir>/<playlist name>/`, named `NN - Artist - Title.opus` once
earworm is confident of the tags, next to a `cover.jpg` (or `.png`) and
`<playlist name>.m3u8`.

Reading a playlist takes a second or so, and the screen has nothing to say
until it comes back, so that is where the wordmark animates in. It runs once
per session and holds for about two seconds, over the library screen too. Any
key skips it, and a track list or a question it is hiding ends it outright.

Run without a URL and earworm opens the library: one row per playlist folder
already under `--dir`. With nothing downloaded yet it asks for a link instead.
The link prompt takes `youtube.com`, `youtu.be` and the `m.` and `music.`
subdomains, with or without the `https://`, and the link has to name a video,
playlist or channel. Anything else comes back for editing rather than being
rejected outright. Esc there quits if it is the first thing you saw, and
otherwise backs out to the screen behind it.

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
| `--no-pick` | download the whole playlist without stopping to choose tracks |
| `--resync` | sync every playlist already under `--dir` |
| `--list` | print the playlists already under `--dir` and exit |
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
pick = true        # stop to choose tracks before downloading

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

An unknown key is a startup error, so a typo tells you instead of silently
changing nothing.

## Keys

| Key | Action |
| --- | --- |
| `j` `k` `↑` `↓` | move |
| `g` `G` | first, last |
| `f` | follow the active track (any movement key stops it) |
| `l` | toggle the yt-dlp output pane |
| `/` | filter the list by name or status |
| `e` | edit artist and title (after the run) |
| `c` | choose cover art (after the run) |
| `r` | retry every failed track (after the run) |
| `S` | sync the open playlist against its saved URL |
| `p` | play this playlist in cliamp (only while cliamp is running) |
| `space` | mark or unmark the track under the cursor |
| `m` | mark every track sharing that track's status |
| `a` | mark every row the filter shows (at the pick gate) |
| `enter` | start the download (at the pick gate) |
| `s` | swap artist and title on the marked tracks |
| `A` | set one artist across the marked tracks |
| `h` `?` | keys and current settings |
| `Esc` | clear the filter, else back to the library, else quit |
| `q` `ctrl+c` | quit |

On the library screen the list is folders, so most of those keys have nothing
to act on:

| Key | Action |
| --- | --- |
| `j` `k` `↑` `↓` `g` `G` | move |
| `enter` | open this playlist from disk |
| `p` | play the folder under the cursor in cliamp (only while it is running) |
| `R` | sync every playlist |
| `n` | sync a new URL |
| `l` | toggle the yt-dlp output pane |
| `h` `?` | keys and current settings |
| `q` `Esc` `ctrl+c` | quit |

Keys that change files go quiet while one is running; the header spinner
comes back to say why.

### When a run finishes

A prompt says what happened and what to do about it:

```
┌ finished ────────────────────────────────────────────────────────┐
│ 12 tracks in ~/Music/Chill Evenings, playlist Chill Evenings.m3u8│
│                                                                  │
│ ▌ keep this open                                                 │
│   sync another playlist                                          │
│   quit                                                           │
└──────────────────────────────────────────────────────────────────┘
```

Keeping it open is first, so neither Enter nor Esc starts work or ends the
session. That leaves the keys below available for fixing tags.
Another playlist asks for a URL and starts over in the same session; cancelling
that question comes back here rather than dropping out.

### Narrowing the list

`/` filters as you type, against the track name and the status word, so
`/sheeran` finds one artist and `/failed` finds the rows worth retrying. `^w`
takes back a word and `^u` the lot. Enter puts the keys back and leaves the
filter up; Esc clears it.

A filtered list looks exactly like a short one, so the rule under the header
turns into a filled band naming the filter and counting what it shows:

```
 FILTER  /sheeran                             3 of 40 shown  ·  esc clears
```

The filter is a view and nothing else. Track numbers stay the numbers they
have, `j` and `k` step over what is hidden, and the track under the cursor is
always one you can see, so `e` can never reach a row off screen. `m` marks
only within the filter, which is what makes `/` then `m` worth having. It
grabs one artist's bad rows and leaves the rest. Marks you made before filtering
still count, and the bar keeps saying how many there are.

Fixing a track can push it out of the filter you found it with, since the name
it matched on has changed. The row goes and the cursor moves to the next one
still showing.

### Choosing what to download

Every run but `--resync` pauses between reading the playlist and downloading
it, and hands you the list:

```
 PICK  158 of 361 selected       space  ·  a all  ·  enter downloads  ·  esc none
```

The gate is there because you can't know whether you want the whole thing until
you see it has 361 tracks. Everything that needs fetching starts selected, so
Enter is the whole playlist and the work is unselecting. Tracks already on disk
start unselected, since nothing was going to fetch them.

`space` toggles one, `m` toggles every track sharing its status, `a` toggles
every row the filter shows, and `/` narrows the list the same as ever, so
`/sheeran` then `a` picks one artist. Enter starts the download; Esc downloads
nothing and carries on, still tagging what is already in the folder and
writing the `.m3u8`.

Tracks you leave out get a `skipped` row and a line in the summary. They are
not failures, and nothing deletes or re-tags them. Run again and they
download.

`--no-pick` skips the gate too, for when you already know you want everything.
The `--resync` exception is there because it walks the whole library
unattended, and a question per folder is the one thing that would stop it.

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

### The library

`earworm` with no URL lists what is already on disk: each folder's name, how
many tracks are in it, how many the manifest lists that are no longer there,
and how long ago it was last synced. `--list` prints the same thing to stdout,
with the URL, and exits.

Enter opens a folder without touching the network: the rows come from
`.earworm`, the tags on the files, and the `.m3u8`, which is what the last sync
said the playlist held, so a file that has since left it still shows as `gone`.
Every post-run key then works on those tracks, which makes the library the way
to fix a tag on something downloaded weeks ago. `S` syncs the open playlist
against the URL its manifest recorded, `R` syncs all of them, and Esc goes
back.

When there is room, a pane beside the rows lists that folder's tracks, with
anything the manifest names that is no longer on disk marked `!` in amber. That
turns the `2 missing` on the row into something you can act on without opening
the folder. It is read-only and starts from the top, with a count of what it
could not fit. Below about 100 columns it makes way for the rows themselves.

Only one playlist is on screen at a time, because a track's number is its
position in its own playlist and two playlists both have a track 1.

### Syncing everything at once

Each playlist folder records the URL it came from in its `.earworm` file, so
`earworm --resync` walks `--dir`, finds the folders that know their own URL and
syncs each one in turn. No arguments, no list to maintain. New tracks download,
departed ones get reported, and a playlist that has since gone private is
logged and skipped rather than stopping the rest.

Folders downloaded before earworm started recording the URL have no entry, so
they need one sync by URL before `--resync` picks them up. Playlists are
visited in folder-name order, one at a time on screen for the same reason the
library opens one folder at a time.

### Playing what you just downloaded

With [cliamp](https://docs.cliamp.stream) running, `p` hands the folder's
`.m3u8` to it, and the finish menu offers the same thing as a row. It works
from the library screen too, so you can play a folder without opening it.

earworm imports the `.m3u8` rather than the folder, because that file is its
own answer about what is in the playlist and in what order, with departed
tracks already left out. cliamp reads the tags earworm wrote, so tracks show up
as `Caravan - Thelonious Monk` rather than as filenames.

`cliamp playlist import` refuses a name it already holds, so earworm deletes
that playlist first and imports over it. A re-sync therefore refreshes the copy
in cliamp instead of failing on a stale one. The delete fails when there is
nothing to delete, which is the ordinary case and not an error.

Playlists land in cliamp as `earworm - <folder> (<tag>)`, never as the bare
folder name. A folder called `Focus` would otherwise destroy a `Focus`
playlist you had built in cliamp by hand. The prefix keeps them together in
cliamp's listing, and the tag is a hash of the folder's path, so two `--dir`
roots that both hold a `Focus` get a playlist each.

`cliamp load` talks to a running instance over a socket, so earworm never
launches it and never takes the terminal. `♪ cliamp` in green in the status bar
means it is up and `p` will play. `♪ cliamp off` in grey means installed but
stopped, and `p` is withdrawn rather than left there to fail. With cliamp not
installed, neither appears. earworm re-checks every few seconds while idle, so
starting cliamp in another window brings the key back.

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
video has left the playlist, `skipped` for one you left out at the pick gate.

## What it writes

earworm renames a track it identified confidently, or that you typed the tags
for, to `NN - Artist - Title.opus`. Tracks it was unsure about keep the name
yt-dlp gave them, because a `weak` or `kept` guess baked into a filename looks
more decided than it is. It never writes over a file that already exists.

Renaming would break the next run's already-downloaded check, which works by
predicting the filename. So earworm keeps a `.earworm` file in the
folder mapping each video id to the file it ended up in, which also survives a
video being retitled on YouTube. It also records the playlist URL and the time
of the last sync, which is what the library screen reads. Delete a track and it
downloads again; delete `.earworm` and every renamed track downloads again.

A pass over part of a playlist, such as a retry or a folder opened from the
library, leaves the entries it did not look at alone. Rewriting the manifest
from just that part would forget files that are on disk under names `scan`
cannot predict, and the next sync would download them all over again.

The `.m3u8` names its tracks by filename alone, not by full path. It sits in
the same folder as the tracks, which is where a player resolves those names
from, so copying the folder to a phone or another machine plays. A playlist
holding absolute paths was written by an older earworm and works only on the
machine that wrote it; any sync or tag edit rewrites it, and `earworm --resync`
does the whole library.

Re-running is cheap. earworm skips ids it already has and reads its own tags
off the disk instead of looking them up again.

A file whose video has since left the playlist gets a `gone` row and a line in
the summary. earworm never deletes it, keeps remembering it in `.earworm` so a
video that comes back doesn't download twice, and leaves it out of the
`.m3u8`, which therefore matches the playlist rather than the folder.

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
