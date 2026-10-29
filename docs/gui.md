# The desktop app

The desktop app is earworm in a window. It runs the same downloads, lookups
and tag writes as the terminal app, reads the same config and library, and
takes the same keys. Anything you do in one shows up in the other, because
both work on the files in your music folder.

[Installing](install.md#the-desktop-app) covers the download and the
one-time step macOS asks for on first launch.

## First launch

![The first-launch screen: a large field for a playlist link, a line saying
where the folder will go, and the tools earworm found with their versions](images/desktop/first-launch.jpg)

With nothing in the library yet, the first screen is the link field. Paste a
playlist, a single video or a file of links and press Enter. Under the field
are the tools earworm found, with their versions, and anything missing with
the line that installs it. A Mac app started from Finder doesn't get your
shell's PATH, so earworm also looks in Homebrew's folders and `~/.local/bin`
itself, and `[tools]` in the config names any tool kept somewhere else.

## The window

The window has three parts:

- **The rail** on the left: your folders, the filter box and the theme menu.
- **The middle:** the open folder's tracks, or whatever screen you're on.
- **The detail pane** on the right: the track or folder under the cursor in
  full, with buttons for the keys that act on it.

The header shows the folder, what the run is doing and how far it has got.
During a run its right half lists the five settings that change what the run
fetches and writes. The bar along the bottom shows the counts and the keys
that work on that screen.

## A run

![The pick gate: twelve tracks of a Krautrock playlist, all ticked, with a
Download 12 tracks button](images/desktop/pick.jpg)

A new playlist stops at the pick gate first. Space unticks a track, `a` and
`i` tick all or invert, and Enter downloads the ticked ones. Esc takes none,
and still tags whatever is already in the folder.

![A run halfway through: tiles for each track with their status, the
yt-dlp log open underneath with one error in red](images/desktop/run.jpg)

Each track's status word changes as it goes, from `fetched` to `tagging` to
`ok`, and the cursor follows the track being worked on until you move it.
`l` opens the yt-dlp log. When the log holds an error, the `l` hint in the
bar gets a `!`.

![The tag form over the run: artist and title fields for a track nothing
could confirm](images/desktop/prompt.jpg)

A track that no lookup confirms gets a form, as in the terminal. Tab moves
between the fields, `^s` swaps artist and title, Enter saves, and Esc skips
the track.

![The finished run as tiles: cover art for most tracks, status words under
each, and the selected unsure track's tags in the detail pane](images/desktop/finished.jpg)

Covers arrive once the run has written them. A finished run opens in review,
with the tracks worth a look listed together. The detail pane says why each
track has its status ("deezer found a match that didn't fit"), and its
buttons search again or open the edit form. `w` switches between tiles and a
list:

![The same run as a list: track number, status, title and artist, and
where the tags came from](images/desktop/list.jpg)

## The library

![The library: one folder in the rail, its tracks as tiles with their
covers, and the folder's details on the right](images/desktop/library.jpg)

Esc from a folder goes back to the library. The rail lists every folder:
`/` filters it, `s` sorts it and `a` narrows it to albums or playlists. Enter
opens a folder from disk, with no network, and `t` searches every track in
the library. The keys are the terminal's, listed on the
[keys](keys.md#library) page, and `h` shows the ones that work on the screen
you're on.

## Settings

![The settings screen: every setting with its value, where the value came
from, and what it does](images/desktop/settings.jpg)

`,` opens every setting, its value, and whether that came from the config
file, a flag or the default. Enter or space changes one and writes it to the
config, so the terminal app picks it up too. The theme menu at the foot of
the rail and `^t` switch between the same five themes as the terminal, plus
any of your own.

## Keys and the mouse

Every key in [keys](keys.md) works here, and `w` is the one key only the
desktop app has. The mouse is another way to do the same things: clicking a
tile or row moves the cursor there, and the buttons in the detail pane press
the key printed on them. On a Mac the ⌘ shortcuts are left alone, so ⌘c
copies and ⌘v pastes. On Linux `^c` quits, as it does in the terminal.

## Updates

The app can't update itself: it isn't signed, so it has no way to prove an
update came from the same place. Once a day it checks GitHub for a newer
release and, if there is one, says so once each time the app starts and
names the releases page.
`update_check = false` turns the check off.
