# Keys and what's on screen

`h` or `?` lists every key that works on the screen you're on, with the status
words grouped by what to do about them.

![The keys overlay: keys grouped by what they do, then the status words
grouped into confirmed tags, guesses worth a look, tracks this run did not
touch, and failures](images/keys.png)

## Track list

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

## Library

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

## The details

`Esc` never ends the session, and does nothing at all while a run or a command
is going. `q` and `ctrl+c` are the way out. `q` asks before it kills a
download, since that's the last thing in the tool one keypress can lose. A
second `q` answers the question, so `qq` never makes you read the box.

## Statuses

The status column says which route a track took.

- `ok` — a confirmed match. `manual` — a prompt you answered. Both are tags
  something checked, which is why they share a colour.
- `guessed` — the tags came off the title with nothing to check them against.
- `unsure` — a match turned up and didn't fit. `no match` — the lookup found
  nothing.
- `failed` — a download or tag write that didn't work.
- `on disk` — already there. `skipped` — one you left out at the gate. `gone`
  — a file whose video has left the playlist.
- During a run: `queued`, `fetching`, `fetched` and `tagging`, in the same
  words the header uses for the run as a whole.

`v`, `/` and the `n`/`N` walk are described with the rest of the workflow in
[fixing tags](workflow.md#fixing-tags).

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
