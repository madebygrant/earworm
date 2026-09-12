# earworm

A terminal UI for turning a YouTube playlist into a folder of tagged audio files.

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
| `ffmpeg` | `ffmpeg` | audio conversion, thumbnail embedding |
| `fpcalc` | `chromaprint` | acoustic fingerprinting (optional) |
| `cliamp` | `cliamp` | playing a finished playlist (optional) |

`earworm --check` answers this table for the machine you are on, along with
the config file it reads, the AcoustID key and the format this run would
write:

```
earworm 0.1.0

tools
  ✓ yt-dlp     2026.08.19
  ✓ ffmpeg     9.0.1
  ✗ fpcalc     not found · brew install chromaprint · no acoustic fingerprinting
  ✓ cliamp     v2.1.0

settings
  ✓ config     ~/.config/earworm/config.toml
  ✗ acoustid   no key · set ACOUSTID_API_KEY · lookups fall back to Deezer
  ✓ format     opus · writes .opus
  ✓ directory  ~/Music · 7 playlists
```

It exits 1 when something earworm cannot work without is missing, and 0 when
only the optional tools are, so a script can act on it. It is also the first
thing worth pasting into a bug report.

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
earworm URL --format flac                # opus by default
earworm URL -- --cookies-from-browser firefox   # extra args go to yt-dlp
earworm --resync                         # every playlist already downloaded
earworm --list                           # what is already on disk, then exit
earworm --check                          # is everything installed, then exit
```

Tracks land in `<dir>/<playlist name>/`, named `NN - Artist - Title.opus` once
earworm is confident of the tags, next to a `cover.jpg` (or `.png`) and
`<playlist name>.m3u8`.

Opus is the default. `--format` takes `opus`, `m4a`, `mp3`, `flac`, `vorbis`
or `alac`. Pressing `f` on the library screen changes it mid-session and writes
the choice to the config file, so the next playlist arrives the same way. That
write edits the one line and leaves every comment where it was, lands through a
symlink onto the file it points at rather than replacing the link, keeps the
file's permissions, and is a rename rather than a truncate, so an interrupted
save cannot leave a config the next start refuses to read. If the save fails
the run keeps the format it had, which is what the file still says.

Changing it converts nothing already on disk. The manifest records each track
by the filename it has, so those files stay as they are and a folder you switch
format on ends up holding both. earworm leaves out wav and aac, since neither
carries tags or cover art.

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
| `-f`, `--format` | opus, m4a, mp3, flac, vorbis or alac (default opus) |
| `-P`, `--no-parse` | keep YouTube's own artist/track, skip title parsing |
| `-L`, `--no-lookup` | skip AcoustID/Deezer, keep parsed tags |
| `-C`, `--no-cover` | don't write `cover.jpg` |
| `-M`, `--no-m3u8` | don't write the playlist file |
| `-F`, `--no-fix` | never prompt; unresolved tracks keep parsed tags |
| `-R`, `--no-rename` | keep yt-dlp's filenames instead of renaming from tags |
| `-A`, `--no-album` | don't fill an empty album tag with the playlist name |
| `--no-pick` | download the whole playlist without stopping to choose tracks |
| `--no-intro` | skip the opening animation |
| `--no-notify` | don't ring the bell when a long run finishes |
| `--resync` | sync every playlist already under `--dir` |
| `--list` | print the playlists already under `--dir` and exit |
| `--check` | check the tools, the key and the config, then exit |
| `--config PATH` | read this config file instead of the default one |
| `--no-config` | ignore the config file entirely |

`-L -F` is the quick one: download and tag from titles, no network lookups, no
questions.

## Configuration

`~/.config/earworm/config.toml` (or `$XDG_CONFIG_HOME/earworm/config.toml`)
holds the settings you'd otherwise retype every run. Every key is optional.

```toml
dir = "~/Music/playlists"
format = "opus"    # or m4a, mp3, flac, vorbis, alac

parse = true       # split artist and title out of the video title
lookup = true      # confirm against AcoustID and Deezer
cover = true       # write cover.jpg beside the tracks
m3u8 = true        # write the playlist file
fix = true         # prompt when a track can't be resolved
rename = true      # rename files from the corrected tags
album = true       # fill an empty album tag with the playlist name
pick = true        # stop to choose tracks before downloading
intro = true       # the opening animation
notify = true      # ring the bell when a long run finishes

extra = ["--sleep-requests", "1"]   # always passed to yt-dlp
acoustid_key = "..."                # or use ACOUSTID_API_KEY
```

Command-line flags win over the file, and the file wins over the defaults
above. Every flag is a negation, so the command line can only switch a feature
off. To run with something the file disables, edit the file.

`extra` from the file comes before anything after `--` on the command line, so
a repeated yt-dlp option takes its command-line value.

Two environment variables change how it draws. `NO_COLOR` set to anything
non-empty drops colour altogether, and the mode bands switch to reversed
video so they stay visible. `COLORTERM=truecolor` or `24bit` says the terminal
can do 24-bit colour; without it earworm assumes 256 and maps the palette to
the nearest colours the terminal has, since a truecolor terminal shown 256
colours only loses some smoothness in the gradient while the other way round
loses the text.

`ACOUSTID_API_KEY` overrides `acoustid_key`, which is handy for one run with a
different key. A key in the file sits there in plain text, so
`chmod 600 ~/.config/earworm/config.toml` if the machine has other users.

An unknown key is a startup error, so a typo tells you instead of silently
changing nothing.

## Keys

| Key | Action |
| --- | --- |
| `j` `k` `↑` `↓` | move |
| `^d` `^u` `PgDn` `PgUp` | move by a screenful |
| `g` `G` | first, last |
| `n` `N` | next, previous track nothing has confirmed |
| `M` | mark every track by the artist under the cursor |
| `f` | follow the active track (any movement key stops it) |
| `l` | toggle the yt-dlp output pane |
| `L` `J` `K` | grow the log pane to half the screen, scroll it |
| `/` | filter the list by name or status |
| `v` | show only the tracks nothing has confirmed |
| `e` | edit artist and title (after the run) |
| `u` | undo the last tag change (after the run) |
| `tab` `^s` | next field, swap artist and title (in the edit form) |
| `c` | choose cover art (after the run) |
| `r` | retry every failed track (after the run) |
| `S` | sync the open playlist against its saved URL |
| `p` | play this playlist in cliamp (only while cliamp is running) |
| `space` | mark or unmark the track under the cursor |
| `m` | mark every track sharing that track's status |
| `a` | mark every row the filter shows (at the pick gate) |
| `i` | invert what is marked (at the pick gate) |
| `enter` | start the download (at the pick gate) |
| `s` | swap artist and title on the marked tracks |
| `A` | set one artist across the marked tracks |
| `1`–`9` | answer a prompt by number |
| `h` `?` | keys and current settings |
| `Esc` | clear the filter, else back to the library |
| `q` `ctrl+c` | quit, asking first if a download is going |

`Esc` never ends the session, and it does nothing at all while a run or a
command is still going. It used to quit whenever there was no library to fall
back on, which is the state a first run is in from beginning to end. `q` and
`ctrl+c` are the way out.

`q` asks before it ends a download, since that is the last thing in the tool
one keypress can lose. A second `q` answers the question, so typing `qq` never
makes you read the box, and `ctrl+c` is still immediate.

On the library screen the list is folders, so most of those keys have nothing
to act on:

| Key | Action |
| --- | --- |
| `j` `k` `↑` `↓` `g` `G` | move |
| `^d` `^u` `PgDn` `PgUp` | move by a screenful |
| `enter` | open this playlist from disk |
| `O` | open this folder in the file manager |
| `/` | filter the folders by name |
| `o` | order: name, last synced, most missing |
| `p` | play the folder under the cursor in cliamp (only while it is running) |
| `space` | play/pause cliamp (only while it is running) |
| `R` | sync every playlist |
| `n` | sync a new URL |
| `f` | choose the format new downloads arrive in |
| `l` | toggle the yt-dlp output pane |
| `h` `?` | keys and current settings |
| `q` `Esc` `ctrl+c` | quit |

A run longer than half a minute rings the terminal bell when it settles,
which is the point at which you have gone to do something else. `notify = false`
or `--no-notify` turns it off. The opening animation has the same pair,
`intro = false` and `--no-intro`.

`O` on the library hands the folder to the platform's file manager, `open` on
macOS and `xdg-open` elsewhere. Everything downstream of earworm happens in a
file manager or a player, and the alternative is retyping a path the screen is
already showing. It is `O` because `o` cycles the sort.

When cliamp has a track loaded the bar names it: `♪ cliamp ⏸  Vogel im Käfig`.
It is cliamp's own title, which for a track earworm wrote is the corrected
one. The library row marked `▶` says which playlist is on air; this says which
song. It is truncated, and it is the first thing on the bar to give way on a
narrow terminal, because it is the one hint whose length is somebody else's
song title and the counts beside it cannot be recovered by pressing anything.

Nothing on screen is told apart by colour alone. Statuses are words, a missing
file carries `!`, an error waiting in the log pane puts `!` in front of the
hint rather than only turning it amber, and the bar says `♪ cliamp ▶` or
`♪ cliamp ⏸` rather than leaving the transport to the colour.

While a run is going the status bar says `⟳ following`, because the cursor
being dragged to whatever is downloading reads as a bug until you know a mode
is doing it. Any movement key ends it and says so once; `f` turns it back on.

On a terminal 100 columns or wider both screens grow a pane on the right: the
library's lists the folder's files and says how many are missing, and the track
list's describes the track under the cursor in full. The row has to truncate
the video title at 32 characters and never has room for the path at all, and
those are the two things a wrong identification is diagnosed from.

The window title follows the run, so a tab you are not looking at says
`earworm · Chill Evenings · 12/40` and then `earworm · Chill Evenings ·
finished`.

Keys that change files go quiet while one is running; the header spinner
comes back to say why. The right of the header carries a bar for the run as a
whole with a rough estimate of what is left, and once the run is over it says
what that run was doing instead: the format and the directory.

### When a run finishes

A prompt says what happened and what to do about it:

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

The review row is only there when the run left guesses or failures behind, and
it is first because it is what the run is asking for: it closes the menu with
the list narrowed to those tracks and the cursor on the first. Failing that,
keeping the menu open is first, so neither Enter nor Esc starts work or ends
the session. That leaves the keys below available for fixing tags.
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

`v` narrows to the tracks nothing has confirmed: the three kinds of guess and
the failures. It is a filter with no text, because those four statuses share no
word to type, and the band says `REVIEW` instead of naming one. The finish
menu offers the same thing when a run leaves any behind.

`n` and `N` walk the same set without hiding anything, which is the other half
of the same question: a guess is judged against the tracks either side of it.
Neither wraps. Reaching the last one says so rather than starting again.

### Undoing a tag change

`u` puts back the last tag change, whether that was one `e`, a `s` over twenty
marked tracks, or an `A`. One level: the edit worth taking back is the one you
have just watched land on the row. The status bar names what it is holding, so
the key is only there when there is something behind it, and it goes after one
use rather than turning into a redo. The tracks come back reading `undone`,
because that is what they are: somebody typed those values back, and the old
`ok` would claim a lookup that is no longer what is in the file.

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
every row the filter shows, `i` inverts what is selected, and `/` narrows the
list the same as ever, so `/sheeran` then `a` picks one artist. Enter starts
the download; Esc downloads nothing and carries on, still tagging what is
already in the folder and writing the `.m3u8`.

Enter with nothing selected does nothing but say so. It would mean the same as
Esc, and it is one stray `a` away from being an accident.

Tracks you leave out get a `skipped` row and a line in the summary. They are
not failures, and nothing deletes or re-tags them. Run again and they
download.

`--no-pick` skips the gate too, for when you already know you want everything.
The `--resync` exception is there because it walks the whole library
unattended, and a question per folder is the one thing that would stop it.

### Fixing one track

`e` opens both fields at once, with the video title the tags came from above
them:

```
┌ track 12 ───────────────────────────────────────────────┐
│ was  Roygbiv - Boards Of Canada (Official Audio)        │
│                                                         │
│ artist  Boards of Canada█                               │
│ title   Roygbiv                                         │
│                                                         │
│ tab field   ^s swap   enter save   esc skip             │
└─────────────────────────────────────────────────────────┘
```

`tab` moves between them, `^s` swaps the two, and `enter` saves both. The same
form is what the run shows when it cannot work a track out on its own.

### Fixing several tracks at once

`space` marks a track. `m` marks every track sharing the status of the one
under the cursor, so grabbing all the `guessed` rows takes one keypress, and a
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

`space` on the library screen is play/pause, which cliamp applies to whatever
it has loaded, earworm's or not. It is free there, unlike on the track list
where it marks a row.

In the library, the folder cliamp has loaded is marked `▶` while it plays and
`⏸` when it is loaded but stopped, so a list of folders says which one is on
air. cliamp reports the track and never the playlist, so the row is found by
the folder that track sits in; a radio stream belongs to no folder and marks
nothing.

`cliamp load` talks to a running instance over a socket, so earworm never
launches it and never takes the terminal. `♪ cliamp` in teal in the status bar
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

The status column says which route a track took. `ok` is a confirmed match and
`manual` is a prompt you answered; both are tags something checked, which is
why they share a colour. `guessed` means the tags came off the title with
nothing to check them against, `unsure` means a match turned up and did not
fit, and `no match` means the lookup found nothing. `failed` is a download or
a tag write that did not work, `on disk` was already there before this run,
`skipped` is one you left out at the pick gate, and `gone` is a file whose
video has left the playlist. While a run is going, `queued`, `fetching`,
`fetched` and `tagging` say where each track has got to, in the same words the
header uses for the run as a whole. `h` shows the same list grouped by what to
do about each.

## What it writes

earworm renames a track it identified confidently, or that you typed the tags
for, to `NN - Artist - Title.opus`. Tracks it was unsure about keep the name
yt-dlp gave them, because an `unsure` or `guessed` name baked into a file looks
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
