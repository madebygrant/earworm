# The desktop app

The desktop app is earworm in a window. It runs the same downloads, lookups
and tag writes as the terminal app, reads the same config and library, and
takes the same keys. Anything you do in one shows up in the other, because
both work on the files in your music folder.

[Installing](install.md#the-desktop-app) covers the download and the
one-time step macOS asks for on first launch.

## First launch

![The first-launch screen: a large field for a playlist link, a line saying
where the folder will go, and the tools and settings earworm found](images/desktop/first-launch.jpg)

With nothing in the library yet, the first screen is the link field. Paste a
playlist, a single video or a file of links and press Enter. Under the field
are the tools earworm found, with their versions, and anything missing with
the line that installs it, then the settings that matter before a first run:
whether there is an AcoustID key, the format and the folder. A Mac app
started from Finder doesn't get your shell's PATH, so earworm also looks in
Homebrew's folders and `~/.local/bin` itself, and `[tools]` in the config
names any tool kept somewhere else.

Without an AcoustID key, lookups fall back to Deezer and Apple and confirm
fewer tracks. The AcoustID row says where to get a free key, at
acoustid.org/new-application, and has a field under it: paste the key and
press Enter, and earworm saves it to the config and restarts with it, still
on this screen.

## The window

The window has four parts:

- **The header:** where you are, a pill for each screen (Library, Artists,
  Playlists, Settings), the run's chip, the track search and the theme
  button. The chip appears once you sync a link: while it downloads it
  counts the tracks, once finished it says how many failed or want a look,
  and from anywhere else a click opens that folder again.
- **The middle:** the screen itself, usually a wall of tiles or a table,
  with a toolbar over it.
- **The sheet** on the right: the tile or row under the cursor in full, with
  buttons for what you can do to it.
- **The bar** along the bottom: the keys that work on this screen, and the
  counts.

## A run

![The pick gate: twelve tracks of a Krautrock playlist, all ticked, with a
Download 12 tracks button in the sheet](images/desktop/pick.jpg)

A new playlist stops at the pick gate first. Space unticks a track, `a` and
`i` tick all or invert, and Enter downloads the ticked ones. Esc takes none,
and still tags whatever is already in the folder.

![A run halfway through: counts of fetched, confirmed, worth a look and
failed tracks over the track table, the run's settings and a gauge beside
it, and the yt-dlp log with one error in red](images/desktop/run.jpg)

While it runs, four tiles count the tracks fetched, confirmed, worth a look
and failed, over a table of every track. Each track's status word changes as
it goes, from `fetched` to `tagging` to `ok`, and the cursor follows the
track being worked on until you move it. Beside the table are the run's
settings and how much of it is fetched. `l` opens the yt-dlp log. When the
log holds an error, the `l` hint in the bar gets a `!`.

![The tag form over the run: artist and title fields for a track nothing
could confirm](images/desktop/prompt.jpg)

A track that no lookup confirms gets a form, as in the terminal. Tab moves
between the fields, `^s` swaps artist and title, Enter saves, and Esc skips
the track.

![The finished run as a wall of sleeves, each with its status, and the
selected unsure track in the sheet](images/desktop/finished.jpg)

A finished run shows the folder as a wall of sleeves, each showing the art
in its file once the run has written it. A track lookup could not place
keeps YouTube's thumbnail, and one with no art at all is a numbered tile.
The sheet says why each track has its status ("deezer found a
match that didn't fit"), and its buttons search again, edit the tags or set
the cover. The toolbar's warning button (`v`) narrows the folder to the
tracks worth a look, and the arrow (`n`) jumps to the next one. The first
time a run finishes with tracks to fix, a tip over the wall says how many and
names those keys; Got it, or `v`, puts it away for good. `w` switches
between tiles and a table:

![The same run as a table: track number, sleeve, title and artist,
confidence, status, where the tags came from and the length](images/desktop/list.jpg)

## The library

![The library: one playlist as a mosaic of its tracks' sleeves, and the
folder's details in the sheet](images/desktop/library.jpg)

Esc from a folder goes back to the library. An album shows its sleeve and a
playlist a mosaic of four of its tracks'. The toolbar narrows the wall to
albums, playlists or custom playlists (`a`), sorts it by name, by last sync
or by missing files (`s`), and switches between tiles and a table (`w`);
`/` filters it. Enter opens a folder from disk, with no network, and `t`
searches every track in the library. The keys are the terminal's, listed on
the [keys](keys.md#library) page, and `h` shows the ones that work on the
screen you're on.

`b` opens the artists in your library as a table, starred ones first, with
their track and album counts and any new releases.
Enter on one opens their releases, top tracks and related artists.

## Settings

![The settings screen: every setting with its value, where the value came
from, and what it does](images/desktop/settings.jpg)

`,` opens every setting, its value, and whether that came from the config
file, a flag or the default. Enter or space changes one and writes it to the
config, so the terminal app picks it up too. The theme button in the header
and `^t` switch between the same five themes as the terminal, plus any of
your own.

## Keys and the mouse

Every key in [keys](keys.md) works here, and `w` is the one key only the
desktop app has. The mouse is another way to do the same things: clicking a
tile or row moves the cursor there, double-clicking a folder or an artist
opens it, and the pills, toolbar buttons and sheet buttons do what a key
does: most press it, and the rest set the same state the key walks to, as
the kind and sort buttons do for `a` and `s`. The bar and `h` name the keys. On a Mac the ⌘ shortcuts are left alone, so ⌘c copies
and ⌘v pastes. On Linux `^c` quits, as it does in the terminal.

## Updates

The app can't update itself: it isn't signed, so it has no way to prove an
update came from the same place. Once a day it checks GitHub for a newer
release and, if there is one, says so once each time the app starts and
names the releases page.
`update_check = false` turns the check off.
