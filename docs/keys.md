# Keys and what's on screen

`h` or `?` lists every key that works on the screen you're on, with the status
words grouped by what to do about them.

`^t` is the one key that works everywhere, the help overlay and prompts
included: it walks the five [colour themes](themes.md) and remembers the one
you stop on.

![The keys overlay: keys grouped by what they do, then the status words
grouped into confirmed tags, guesses worth a look, tracks this run did not
touch, and failures](images/keys.png)

The [desktop app](gui.md) takes every key on this page. `w` is the one
key it adds. On a Mac its ⌘ shortcuts (⌘c, ⌘v) keep their usual meaning; on
Linux `^c` quits, as it does here.

## Track list

| Key | Action |
| --- | --- |
| `j` `k` `↑` `↓` | move |
| `^d` `^u` `PgDn` `PgUp` | move by a screenful |
| `g` `G` | first, last |
| `n` `N` | next, previous track nothing has confirmed |
| `f` | follow the active track (any movement key stops it) |
| `w` | tiles or a list (desktop app only) |
| `l` | toggle the yt-dlp output pane |
| `L` `J` `K` | grow the log pane to half the screen, scroll it |
| `/` | filter the list by name or status |
| `v` | show only the tracks nothing has confirmed |
| `^t` | next colour theme, and remember it |
| `space` | mark or unmark the track under the cursor |
| `m` | mark every track sharing that track's status |
| `M` | mark every track by the artist under the cursor |
| `e` | edit artist, title, album and year (after the run) |
| `u` | undo the last tag change (after the run) |
| `c` | choose cover art, or give every track its own back (after the run) |
| `T` | search again for the track under the cursor (after the run) |
| `s` | swap artist and title on the marked tracks |
| `A` | set one artist across the marked tracks |
| `*` | star or unstar the artist of the track under the cursor |
| `tab` `^s` | next field, swap artist and title (in the edit form) |
| `r` | retry every failed track (after the run) |
| `S` | sync the open playlist, asking for a URL if it has none |
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
| `D` | remove this playlist: forget, hide, or delete its folder |
| `O` | open this folder in the file manager |
| `/` | filter the folders by name, or by a track inside them |
| `t` | every matching track across the library, flat |
| `b` | every artist in the library, starred first ([discovery](discovery.md)) |
| `s` | sort: name, last synced, most missing |
| `w` | tiles or a list (desktop app only) |
| `a` | show every folder, albums only, or playlists only |
| `p` | play the folder under the cursor in cliamp (needs a playlist file) |
| `space` | play/pause cliamp |
| `S` | sync the playlist under the cursor (asks for a URL if it has none) |
| `R` | sync every playlist |
| `n` | sync a new URL |
| `^t` | next colour theme, and remember it |
| `l` | toggle the yt-dlp output pane |
| `,` | settings, from any screen with a list |
| `h` `?` | keys and current settings |
| `q` `Esc` `ctrl+c` | quit |

## Artists

| Key | Action |
| --- | --- |
| `j` `k` `↑` `↓` `g` `G` | move |
| `^d` `^u` `PgDn` `PgUp` | move by a screenful |
| `enter` | this artist's page |
| `o` | the library, narrowed to the folders holding their tracks |
| `*` | star or unstar this artist |
| `/` | filter by name, ignoring accents and case |
| `Esc` | clear the filter, else back to the library |

## Artist page

| Key | Action |
| --- | --- |
| `tab` | releases, top tracks, related artists |
| `j` `k` `↑` `↓` `g` `G` | move |
| `enter` | open a related artist's page |
| `a` | albums and EPs, or every release |
| `o` | the library, narrowed to their folders |
| `w` | tiles or a list (desktop app only) |
| `Esc` | the page before, else the artists |

## Settings

| Key | Action |
| --- | --- |
| `j` `k` `↑` `↓` `g` `G` | move |
| `enter` `space` | switch it, or move to the next choice |
| `Esc` `,` | back to the screen you came from |

What each row means is in [configuration](configuration.md#the-settings-screen).

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
- `local` — a file of yours that the playlist doesn't have. Nothing touches
  it, it stays out of the `.m3u8`, and `D` never deletes it.
- During a run: `queued`, `fetching`, `fetched` and `tagging`, in the same
  words the header uses for the run as a whole.

`v`, `/` and the `n`/`N` walk are described with the rest of the workflow in
[fixing tags](workflow.md#fixing-tags).

## Cover art

`c` offers what the lookup found for the track under the cursor, whatever
image the folder already holds, and a file of your own. Whichever you pick is
written into the folder as `cover.jpg` or `cover.png` and embedded in every
track, which is what an album wants.

A playlist is the opposite case. Its tracks come from a dozen different
records and one shared sleeve is the only thing hiding that, so the menu
carries a row that takes the folder image away and looks each track up again
to embed its own album art. That is the only way back, because choosing a
cover writes over each track's own art rather than sitting beside it.

It asks first, naming the folder and how many tracks it would rewrite, because
the art it embeds is chosen automatically and nothing puts the old art back.
Refusing leaves the folder exactly as it was, cover included. It needs the
network; a track it can't find art for keeps what it has and is named in the
log pane.

After a playlist's run there is no folder image to remove. earworm writes one
while it tags and deletes it again at the end, since every track already
carries the art. One you dropped in yourself is never touched.

A folder earworm has decided is an album keeps its `cover.jpg`, and its tracks
all get the one sleeve rather than each looking up its own.
[The library](library.md#albums-and-playlists) covers which folders those are
and how to mark one yourself.

## On screen

Nothing is told apart by colour alone. Statuses are words, a missing file
carries `!`, an error waiting in the log pane puts `!` in front of the hint,
a library row's pills say `album` and `local` in words, and the bar says
`♪ cliamp ▶` or `♪ cliamp ⏸` rather than leaving the transport to the colour.

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
