# The library and syncing

![The library screen: three playlist folders with their track counts, missing
files and last sync, and a pane listing the selected folder's
tracks](images/library.png)

`earworm` with no URL lists what's on disk: each folder's name, its track
count, how many the manifest names that are no longer there, and how long ago
it synced. `--list` prints the same to stdout, with the URL, and exits.

Enter opens a folder without touching the network. The rows come from
`.earworm`, the tags on the files and the `.m3u8`. Every post-run key then
works on those tracks, which makes the library the way to fix a tag on
something downloaded weeks ago. `S` syncs the open playlist against its saved
URL, `R` syncs all of them, Esc goes back.

## Renaming a playlist

`e` renames a playlist. The folder is named from the playlist's title on
YouTube and that name is chosen again on every download, so the rename is
recorded in `.earworm` and the next sync writes back into the folder you named
rather than re-creating the old one beside it. A folder renamed outside
earworm has no such record, so press `e` on it and hit Enter on the name it
already shows. Nothing moves, and the record is written.

## Removing a playlist

`D` removes the playlist under the cursor and asks what goes with it.
Forgetting drops `.earworm` and the `.m3u8` but keeps the audio, so the folder
stays as plain files and the next `--resync` walks past it. Deleting removes
the folder. Either way cliamp's copy is dropped too.

`O` hands the folder to the platform's file manager, `open` on macOS and
`xdg-open` elsewhere. It's `O` because `o` cycles the sort.

## Finding a track

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

Most failures are a network blip or a video that was briefly unavailable, so
`r` re-downloads and re-tags every `failed` track in one pass.

Folders downloaded before earworm started recording the URL need one sync by
URL before `--resync` picks them up.
