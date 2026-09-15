# earworm

**Playlist in, library out.**

A terminal app that turns a YouTube playlist into a folder of properly tagged
audio files. `yt-dlp` does the downloading. earworm does everything after: it
splits artist and title out of the video title, checks that guess against
AcoustID, Deezer and Apple Music, writes the tags, embeds cover art and leaves
an `.m3u8` beside the tracks. When it can't work a track out it asks instead of
writing a tag it doesn't believe, and nothing is so settled you can't change it
later.

![A playlist open in earworm: the track list on the left, the selected track's
tags and file path in a pane on the right, and the run's settings along the
header](docs/images/earworm.png)

## Install

```sh
brew install yt-dlp ffmpeg chromaprint
cargo install --path .
```

## Requirements

Four binaries on PATH, two of them optional.

| Binary | Homebrew formula | Used for |
| --- | --- | --- |
| `yt-dlp` | `yt-dlp` | downloading and extracting audio |
| `ffmpeg` | `ffmpeg` | conversion, cover art, resizing |
| `fpcalc` | `chromaprint` | acoustic fingerprinting (optional) |
| `cliamp` | `cliamp` | playing a finished playlist (optional) |

`earworm --check` answers that table for the machine you're on, plus the
config it reads and the format this run would write. Paste it into a bug
report.

```
earworm 0.2.0

tools
  ✓ yt-dlp     2026.08.19
  ✓ ffmpeg     9.0.1
  ✗ fpcalc     not found · brew install chromaprint · no acoustic fingerprinting
  ✓ cliamp     v2.1.0

settings
  ✓ config     ~/.config/earworm/config.toml
  ✗ acoustid   no key · set ACOUSTID_API_KEY · lookups fall back to Deezer and Apple
  ✓ format     opus · writes .opus
  ✓ convert    off · tracks already on disk keep their format
  ✓ directory  ~/Music · 7 playlists
```

It exits 1 when something earworm can't work without is missing and 0 when
only the optional tools are, so a script can act on it.

Fingerprinting also wants a free
[AcoustID](https://acoustid.org/new-application) key:

```sh
export ACOUSTID_API_KEY=your-key
```

Without one, lookups fall back to Deezer and then to Apple Music, which match
a lot less well. Nothing else changes.

## Use

```sh
earworm                                  # asks for a URL
earworm 'https://youtube.com/playlist?list=...'
earworm URL --dir ~/Music/new
earworm URL --no-pick                    # don't stop to choose, fetch it all
earworm URL --format flac                # opus by default
earworm URL -- --cookies-from-browser firefox   # extra args go to yt-dlp
earworm --resync                         # every playlist already downloaded
earworm --list                           # what's on disk, then exit
earworm --check                          # is everything installed, then exit
```

Tracks land in `<dir>/<playlist name>/` as `NN - Artist - Title.opus`, next to
`<playlist name>.m3u8`. Cover art is embedded in each track rather than
dropped in as a folder image, and capped at 1000x1000. YouTube serves an
auto-generated album's art at 2048 square, which is several times the size of
the audio it's attached to, repeated in every file.

Run without a URL and you get the library, one row per playlist already
downloaded. With nothing on disk yet it asks for a link. The prompt takes
`youtube.com`, `youtu.be` and the `m.` and `music.` subdomains, with or
without the `https://`, naming a video, playlist or channel. Anything else
comes back for editing rather than being thrown away.

## Formats

Opus by default. `--format` takes `opus`, `m4a`, `mp3`, `flac`, `vorbis` or
`alac`. wav and aac are left out because neither carries tags or cover art.

Typing a URL offers the format before the download starts. The choice is
written to the config, so the next playlist arrives the same way, and `f` on
the library changes it later.

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

### Flags

| Flag | Effect |
| --- | --- |
| `-d`, `--dir` | output directory (default `~/Music`) |
| `-f`, `--format` | opus, m4a, mp3, flac, vorbis or alac (default opus) |
| `--no-convert` | leave tracks already on disk in the format they have |
| `-P`, `--no-parse` | keep YouTube's own artist/track, skip title parsing |
| `-L`, `--no-lookup` | skip AcoustID/Deezer/Apple, keep parsed tags |
| `--no-apple` | skip the Apple Music fallback, Deezer only |
| `-C`, `--no-cover` | don't fetch or embed cover art |
| `-M`, `--no-m3u8` | don't write the playlist file |
| `-F`, `--no-fix` | never prompt; unresolved tracks keep parsed tags |
| `-R`, `--no-rename` | keep yt-dlp's filenames instead of renaming from tags |
| `-A`, `--no-album` | don't fill an empty album tag with the playlist name |
| `--no-pick` | download the whole playlist without stopping to choose |
| `--no-intro` | skip the opening animation |
| `--no-notify` | don't ring the bell when a long run finishes |
| `--no-update-check` | never ask GitHub about a newer release |
| `--resync` | sync every playlist already under `--dir` |
| `--list` | print the playlists already under `--dir` and exit |
| `--check` | check the tools, the key and the config, then exit |
| `--config PATH` | read this config file instead of the default one |
| `--no-config` | ignore the config file entirely |

`-L -F` is the quick one. Download and tag from titles, no network lookups, no
questions.

## Configuration

`~/.config/earworm/config.toml`, or `$XDG_CONFIG_HOME/earworm/config.toml`.
Every key is optional.

```toml
dir = "~/Music/playlists"
format = "opus"    # or m4a, mp3, flac, vorbis, alac
convert = false    # bring tracks already on disk to `format` when they sync

parse = true       # split artist and title out of the video title
lookup = true      # confirm against AcoustID, Deezer and Apple
apple = true       # fall back to Apple Music when Deezer finds nothing
cover = true       # fetch cover art and embed it in the tracks
m3u8 = true        # write the playlist file
fix = true         # prompt when a track can't be resolved
rename = true      # rename files from the corrected tags
album = true       # fill an empty album tag with the playlist name
pick = true        # stop to choose tracks before downloading
intro = true       # the opening animation
notify = true      # ring the bell when a long run finishes
update_check = true # ask GitHub about a newer release, at most once a day

extra = ["--sleep-requests", "1"]   # always passed to yt-dlp
acoustid_key = "..."                # or use ACOUSTID_API_KEY
```

Flags beat the file, the file beats the defaults. Every flag is a negation, so
the command line can only switch something off. To run with a feature the file
disables, edit the file. `extra` from the file comes before anything after
`--` on the command line, so a repeated yt-dlp option takes its command-line
value.

An unknown key is a startup error, so a typo tells you rather than silently
changing nothing. A key in the file sits there in plain text, so
`chmod 600 ~/.config/earworm/config.toml` on a shared machine.
`ACOUSTID_API_KEY` overrides `acoustid_key` for one run.

Once a day earworm asks GitHub whether a newer version exists and says so on
the intro and in `--check`. Equal versions, older ones, network failures and
`--no-update-check` all say nothing. Updating is
`git pull && cargo install --path .`.

Two environment variables change how it draws. `NO_COLOR` set to anything
non-empty drops colour, and the mode bands switch to reversed video so they
stay visible. `COLORTERM=truecolor` says the terminal can do 24-bit colour;
without it earworm assumes 256 and maps the palette to the nearest available,
since a truecolor terminal shown 256 colours loses a little smoothness in the
gradient while the other way round loses the text.

## Keys

`h` or `?` lists every key that works on the screen you're on, with the status
words grouped by what to do about them.

![The keys overlay: keys grouped by what they do, then the status words
grouped into confirmed tags, guesses worth a look, tracks this run did not
touch, and failures](docs/images/keys.png)

| Key | Action |
| --- | --- |
| `j` `k` `↑` `↓` | move |
| `^d` `^u` `PgDn` `PgUp` | move by a screenful |
| `g` `G` | first, last |
| `n` `N` | next, previous track nothing has confirmed |
| `f` | follow the active track (any movement key stops it) |
| `l` | toggle the yt-dlp output pane |
| `L` `J` `K` | grow the log pane to half the screen, scroll it |
| `/` | filter the list by name or status |
| `v` | show only the tracks nothing has confirmed |
| `space` | mark or unmark the track under the cursor |
| `m` | mark every track sharing that track's status |
| `M` | mark every track by the artist under the cursor |
| `e` | edit artist, title, album and year (after the run) |
| `u` | undo the last tag change (after the run) |
| `c` | choose cover art (after the run) |
| `T` | search again for the track under the cursor (after the run) |
| `s` | swap artist and title on the marked tracks |
| `A` | set one artist across the marked tracks |
| `tab` `^s` | next field, swap artist and title (in the edit form) |
| `r` | retry every failed track (after the run) |
| `S` | sync the open playlist against its saved URL |
| `D` | delete the files of tracks a sync found had left the playlist |
| `p` | play this playlist in cliamp (only while cliamp is running) |
| `a` `i` `enter` | all, invert, download (at the pick gate) |
| `Esc` `t` | find a track in any playlist (the search is a library key) |
| `1`–`9` | answer a prompt by number |
| `h` `?` | keys and current settings |
| `Esc` | clear the filter, unmark all, else back to the library |
| `q` `ctrl+c` | quit, asking first if a download is going |

`Esc` never ends the session, and does nothing at all while a run or a command
is going. `q` and `ctrl+c` are the way out. `q` asks before it kills a
download, since that's the last thing in the tool one keypress can lose. A
second `q` answers the question, so `qq` never makes you read the box.

The library screen lists folders, so most of those keys have nothing to act
on.

| Key | Action |
| --- | --- |
| `j` `k` `↑` `↓` `g` `G` | move |
| `^d` `^u` `PgDn` `PgUp` | move by a screenful |
| `enter` | open this playlist from disk |
| `e` | rename this playlist |
| `D` | remove this playlist: forget it or delete its folder |
| `O` | open this folder in the file manager |
| `/` | filter the folders by name, or by a track inside them |
| `t` | every matching track across the library, flat |
| `o` | order: name, last synced, most missing |
| `p` | play the folder under the cursor in cliamp |
| `space` | play/pause cliamp |
| `R` | sync every playlist |
| `n` | sync a new URL |
| `f` | choose the format new downloads arrive in |
| `l` | toggle the yt-dlp output pane |
| `h` `?` | keys and current settings |
| `q` `Esc` `ctrl+c` | quit |

## On screen

Nothing is told apart by colour alone. Statuses are words, a missing file
carries `!`, an error waiting in the log pane puts `!` in front of the hint,
and the bar says `♪ cliamp ▶` or `♪ cliamp ⏸` rather than leaving the
transport to the colour.

While a run is going the bar says `⟳ following`, because the cursor being
dragged to whatever is downloading reads as a bug until you know a mode is
doing it. Any movement key ends it and says so once.

At 100 columns or wider both screens grow a pane on the right. The library's
lists the folder's files and marks the missing ones. The track list's gives
the track under the cursor in full, including the video title and the path,
which the row has to truncate and can't fit at all.

The window title follows the run, so a tab you aren't looking at reads
`earworm · Chill Evenings · 12/40` and then `earworm · Chill Evenings ·
finished`. A run longer than half a minute rings the bell when it settles,
which is the point at which you've gone to do something else.

## When a run finishes

```
┌ finished ────────────────────────────────────────────────────────┐
│ 12 tracks in ~/Music/Chill Evenings, playlist Chill Evenings.m3u8│
│                                                                  │
│ ▌ review 9 tracks worth a look                                   │
│   keep this open                                                 │
│   sync another playlist                                          │
│   quit                                                           │
└──────────────────────────────────────────────────────────────────┘
```

The review row only appears when the run left guesses or failures behind, and
it's first because it's what the run is asking for. It closes the menu with
the list narrowed to those tracks and the cursor on the first. Otherwise
keeping the menu open is first, so neither Enter nor Esc starts work or ends
the session.

## Choosing what to download

Every run but `--resync` pauses between reading the playlist and downloading
it, and hands you the list.

```
 PICK  158 of 361 selected       space  ·  a all  ·  enter downloads  ·  esc none
```

The gate exists because you can't know whether you want the whole thing until
you see it has 361 tracks. Everything that needs fetching starts selected, so
Enter is the whole playlist and the work is unselecting. Tracks already on
disk start unselected.

`space` toggles one, `m` toggles every track sharing its status, `a` toggles
every row the filter shows, `i` inverts, and `/` narrows as ever, so
`/sheeran` then `a` picks one artist. Esc downloads nothing and carries on,
still tagging what's already in the folder. Enter with nothing selected says
so rather than acting, since it would mean the same as Esc and is one stray
`a` from being an accident.

Tracks you leave out get a `skipped` row. They aren't failures, nothing
deletes or re-tags them, and running again downloads them.

## Narrowing the list

`/` filters as you type, against the track name and the status word, so
`/sheeran` finds one artist and `/failed` finds the rows worth retrying. `^w`
takes back a word, `^u` the lot. Enter puts the keys back and leaves the
filter up; Esc clears it.

A filtered list looks exactly like a short one, so the rule under the header
becomes a band naming the filter and counting what it shows.

```
 FILTER  /sheeran                             3 of 40 shown  ·  esc clears
```

The filter is a view and nothing else. Track numbers stay as they are, `j` and
`k` step over what's hidden, and the cursor is always on a row you can see.
`m` marks only within the filter, which is what makes `/` then `m` worth
having. It grabs one artist's bad rows and leaves the rest.

Fixing a track can push it out of the filter you found it with, since the name
it matched on has changed. The row goes and the cursor moves on.

`v` narrows to the tracks nothing has confirmed. It's a filter with no text,
because those four statuses share no word to type, and the band reads
`REVIEW`. `n` and `N` walk the same set without hiding anything, since a guess
is judged against the tracks either side of it. Neither wraps.

## Fixing tags

`e` opens every field at once, with the video title the tags came from above
them.

```
┌ track 12 ───────────────────────────────────────────────┐
│ was  Roygbiv - Boards Of Canada (Official Audio)        │
│                                                         │
│ artist  Boards of Canada█                               │
│ title   Roygbiv                                         │
│ album   Music Has the Right to Children                 │
│ year    1998                                            │
│                                                         │
│ tab field   ^s swap   enter save   esc skip             │
└─────────────────────────────────────────────────────────┘
```

`tab` moves between boxes, `^s` swaps artist and title wherever they sit,
`enter` saves the lot. Album and year may be left empty, which removes the
tag. A year that isn't four digits refuses the edit rather than quietly
dropping what the file had. The run asks a shorter version of the same form
when it can't work a track out, artist and title only, since the album is what
the lookup is there to find.

For several at once, `space` marks a track and `m` marks every track sharing
its status, so grabbing all the `guessed` rows takes one keypress. `s` then
swaps artist and title across the lot, for a playlist that parsed the wrong
way round, and `A` sets one artist, for a compilation the lookup split into
different names. With nothing marked, both act on the track under the cursor.

`u` puts back the last tag change, whether that was one `e`, an `s` over
twenty tracks, or an `A`. One level, because the edit worth taking back is the
one you just watched land on the row. The bar names what it's holding, and it
goes after one use rather than turning into a redo. The tracks come back
reading `undone`, since somebody typed those values back and `ok` would claim
a lookup that's no longer what's in the file.

## The library

![The library screen: three playlist folders with their track counts, missing
files and last sync, and a pane listing the selected folder's
tracks](docs/images/library.png)

`earworm` with no URL lists what's on disk: each folder's name, its track
count, how many the manifest names that are no longer there, and how long ago
it synced. `--list` prints the same to stdout, with the URL, and exits.

Enter opens a folder without touching the network. The rows come from
`.earworm`, the tags on the files and the `.m3u8`. Every post-run key then
works on those tracks, which makes the library the way to fix a tag on
something downloaded weeks ago. `S` syncs the open playlist against its saved
URL, `R` syncs all of them, Esc goes back.

`e` renames a playlist. The folder is named from the playlist's title on
YouTube and that name is chosen again on every download, so the rename is
recorded in `.earworm` and the next sync writes back into the folder you named
rather than re-creating the old one beside it. A folder renamed outside
earworm has no such record, so press `e` on it and hit Enter on the name it
already shows. Nothing moves, and the record is written.

`D` removes the playlist under the cursor and asks what goes with it.
Forgetting drops `.earworm` and the `.m3u8` but keeps the audio, so the folder
stays as plain files and the next `--resync` walks past it. Deleting removes
the folder. Either way cliamp's copy is dropped too.

`O` hands the folder to the platform's file manager, `open` on macOS and
`xdg-open` elsewhere. It's `O` because `o` cycles the sort.

### Finding a track

`/` searches inside the folders as well as across their names, so typing a
track you half remember narrows the library to whatever holds it. Nothing is
read off disk to do it.

`t` takes the same query and lists every match flat, with the folder each is
in.

```
 earworm  ·  search  ·  4 tracks in 3 playlists
 SEARCH  /neu!                          4 of 612 shown  ·  esc clears
▌  Focus        01 Neu! - Hallogallo
   Road trip  ! 02 Neu! - Negativland
   Sleep        01 Neu! - Weissensee
   Sleep        07 Neu! - Isi
```

`enter` opens that playlist with the cursor on the track, `esc` goes back to
the folders, `/` edits the query from either screen. A `!` means the manifest
names a file that's no longer on disk. This is the screen for a library too
big to scroll, and the only way to see matches from several folders at once.

Only one playlist is on screen at a time, because a track's number is its
position in its own playlist and two playlists both have a track 1.

## Syncing everything at once

Each folder records the URL it came from, so `earworm --resync` walks `--dir`,
finds the folders that know their own URL and syncs each in turn. No
arguments, no list to maintain. New tracks download, departed ones get
reported, and a playlist that has since gone private is logged and skipped
rather than stopping the rest.

Folders downloaded before earworm started recording the URL need one sync by
URL before `--resync` picks them up.

Most failures are a network blip or a video that was briefly unavailable, so
`r` re-downloads and re-tags every `failed` track in one pass.

## Playing it

With [cliamp](https://docs.cliamp.stream) running, `p` hands it the folder's
`.m3u8`. It works from the library too, so you can play a folder without
opening it. earworm imports the playlist file rather than the folder, because
that file is its own answer about what's in the playlist and in what order,
departures already left out. cliamp reads the tags earworm wrote, so tracks
show up as `Caravan - Thelonious Monk` rather than as filenames.

Playlists land in cliamp as `earworm - <folder> (<tag>)`. A folder called
`Focus` would otherwise destroy a `Focus` playlist you'd built in cliamp by
hand, and the tag is a hash of the folder's path, so two `--dir` roots that
both hold a `Focus` get a playlist each. A re-sync refreshes the copy rather
than failing on a stale one.

`space` on the library is play/pause, which cliamp applies to whatever it has
loaded, earworm's or not. The folder cliamp is playing is marked `▶`, or `⏸`
when loaded but stopped. When cliamp has a track loaded the bar names it:
`♪ cliamp ⏸  Vogel im Käfig`. The row says which playlist is on air, this says
which song.

earworm talks to a running cliamp over a socket and never launches it, so it
never takes the terminal. `♪ cliamp off` means installed but stopped, and `p`
is withdrawn rather than left there to fail. earworm re-checks every few
seconds while idle, so starting cliamp in another window brings the key back.

## How a track gets its tags

1. **Parse.** Split the video title on its separator. A dash means
   artist-first, a pipe or bullet means artist-last.
2. **Fingerprint.** `fpcalc` plus AcoustID, accepted at score 0.8 or better.
3. **Search.** Deezer, then Apple Music when Deezer has nothing plausible,
   matched on normalised artist and title.
4. **Ask.** Anything still unresolved gets a prompt listing the candidates,
   unless `--no-fix`.

The status column says which route a track took. `ok` is a confirmed match and
`manual` is a prompt you answered, both tags something checked, which is why
they share a colour. `guessed` means the tags came off the title with nothing
to check them against, `unsure` means a match turned up and didn't fit, and
`no match` means the lookup found nothing. `failed` is a download or tag write
that didn't work, `on disk` was already there, `skipped` is one you left out at
the gate, and `gone` is a file whose video has left the playlist. During a run,
`queued`, `fetching`, `fetched` and `tagging` say where each track has got to,
in the same words the header uses for the run as a whole.

## What it writes

earworm renames a track it identified confidently, or that you typed the tags
for, to `NN - Artist - Title.opus`. Tracks it was unsure about keep the name
yt-dlp gave them, because an `unsure` name baked into a file looks more
decided than it is. It never writes over a file that already exists.

Renaming would break the next run's already-downloaded check, which works by
predicting the filename. So `.earworm` maps each video id to the file it ended
up in, which also survives a video being retitled on YouTube, and records the
playlist URL and last sync time for the library to read. Delete a track and it
downloads again. Delete `.earworm` and every renamed track downloads again.

The `.m3u8` names its tracks by filename, never by full path, and sits in the
same folder as them, so copying that folder to a phone or another machine
plays. Any sync or tag edit rewrites it.

Re-running is cheap. earworm skips ids it already has and reads its own tags
off the disk instead of looking them up again.

A file whose video has left the playlist gets a `gone` row. earworm never
deletes it on its own: it keeps remembering it in `.earworm` so a video that
comes back doesn't download twice, and leaves it out of the `.m3u8`, which
therefore matches the playlist rather than the folder. `D` deletes those
files, after a question that names every one.

That only works when the sync you just ran asked for the whole playlist and
got it. earworm stays quiet about departures when yt-dlp didn't finish
listing, or when you passed extra yt-dlp arguments: `--playlist-items 4` makes
every other track absent from the listing while it sits in the playlist
untouched, and yt-dlp exits 0 either way. Open a folder from the library, see
`gone` rows, and `D` isn't there until you press `S`.

The album tag gets the playlist name only when the lookup found no real album,
since the release a track actually came from is the better answer.

On exit earworm prints the folder it wrote to.

## Notes

- Only downloads what you have the right to download.
- Editing tags renames the file and rewrites the `.m3u8` to match.
  `--no-rename` turns that off.
- Quitting kills the yt-dlp child and lets a tag write already under way
  finish.

## Licence

MIT or Apache-2.0, at your option.
