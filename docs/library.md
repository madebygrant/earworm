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

## Folders earworm didn't download

A folder of audio you copied into `--dir` yourself is on the list too, with
its track count, and opens the same way. The tags and the filenames are
everything these screens draw, and nothing about them depends on where the
files came from, so `e`, `c`, `s`, `A`, `u` and the rest work on it.

Its row wears a `local` pill where a synced one says `synced 3d ago`, and
`--list` prints `-` in the URL column. That is the whole difference: there is
no playlist behind it, so `R` walks past it and says so in the log pane, `S`
asks for a URL instead of running, and `p` is only offered if the folder
brought a `.m3u8` of its own. It is a pill because those are the three keys
that behave differently, and dim text at the end of the row buried them.

Nothing is written into such a folder until you change something in it.
Listing it and opening it are reads. The first edit writes `.earworm`, the
same sidecar every downloaded folder has, and says so once. From then on the
folder behaves like any other except that it still has no URL. `D` offers no
forget row while there is no sidecar, because there would be nothing to
forget.

If the folder already holds a playlist file, earworm leaves it exactly as it
is, even when its name is the one earworm would have chosen. While the folder
has no URL, earworm rewrites only a playlist it wrote itself. Giving it a URL
changes that: from the first sync the playlist upstream is what the folder
holds, in that order, and earworm writes `<folder>.m3u8` to say so.

## Giving one a URL

`S` on a folder with no URL asks for one, then syncs. That sync is different
from every other: before downloading anything it works out which files in the
folder are which videos, so a folder you already have does not arrive again
beside itself. It matches on the filename earworm would have written, then on
the title, from the tag or from the filename with its leading number taken
off, and uses the length only to break a tie between two files of the same
title. Anything it cannot place is left alone and downloaded fresh.

It always stops at the pick gate, whatever your `--pick` setting says, because
the matching is a guess and the screen showing `12 to download` against a
folder you know holds twelve is the point at which to stop it. `--resync`
never attaches a URL to anything: it is unattended, and there would be nobody
at that gate.

That first sync also takes over the `.m3u8`. Up to then earworm left yours
alone; afterwards the playlist is what the folder is for, so if the ordering
in that file is yours and worth keeping, copy it somewhere else before you
attach a URL.

Files the playlist turns out not to have read `local`. They stay in the
folder, keep their tags, are left out of the `.m3u8`, and `D` never touches
them: they are yours, and nothing about them says a video left a playlist.
They still read `local` when you reopen the folder later, rather than turning
into departures because the playlist file never named them.

## Renaming a playlist

`e` renames a playlist. The folder is named from the playlist's title on
YouTube and that name is chosen again on every download, so the rename is
recorded in `.earworm` and the next sync writes back into the folder you named
rather than re-creating the old one beside it. A folder renamed outside
earworm has no such record, so press `e` on it and hit Enter on the name it
already shows. Nothing moves, and the record is written.

## Albums and playlists

A folder is a playlist unless earworm has reason to think otherwise, and the
one reason it accepts by itself is the playlist id: YouTube Music gives a
release an id starting `OLAK5uy_`, where a playlist you or anyone else made
starts `PL`. That is the only automatic signal, and it only ever promotes.
Calling an album a playlist costs nothing you can see; calling a playlist an
album stamps one sleeve across a dozen unrelated records.

You say it yourself by putting it in the folder's name, in brackets:

```
Autobahn [album]
Autobahn [album] [1974]
Chill Evenings [playlist]
```

Case and spaces inside the brackets don't matter. The brackets have to hold
nothing but the word, so `[Album Version]`, `[Deluxe Edition]` and `[2024]` are
not markers and a library full of `[FLAC]` and `[Remastered]` folders reads as
it always did. Two different markers in one name is no answer rather than the
first one.

earworm reads the marker and never writes one. A folder nobody marks keeps the
name it has, and deleting a marker hands the folder straight back to detection
on the next sync. The name beats everything, including what an earlier sync
worked out, which is what makes it the override: rename the folder in Finder
or with `e` and you have said which it is.

It works upstream too. Name the playlist `Mixtape [playlist]` on YouTube and
the folder arrives named that on the first download. And the marker travels
with the music, so a folder marked here stays marked on a phone.

An album wears a filled `album` pill beside its name on the library screen,
whichever way earworm arrived at the answer, so a release it recognised by its
id is as visible as one you marked. A playlist wears nothing, and a library
with no album in it draws exactly as it did before.

A folder you marked `[album]` yourself shows the pill instead of the word, not
both. Only that row is shortened: the folder keeps its name, `e` opens on the
real one, and `/[album]` still finds it.

`a` narrows the library to albums, then to playlists, then back to every
folder. A folder nothing has decided about is on the playlist list, since
playlist is the default. It works alongside `/`, so `a` then `/kraft` is the
Kraftwerk albums alone, and Esc clears both. `t` searches only the folders
the kind is showing, and coming back from it leaves the kind on.

When a run decides, it says so and says where the answer came from:

```
12 tracks in Autobahn  ·  album , from the folder name
12 tracks in Autobahn  ·  album , from the playlist id
40 tracks in Chill Evenings
```

A folder nothing has decided about says nothing, because playlist is the
default rather than a finding.

What changes for an album is the art. Every track gets the first sleeve the
lookup resolves, instead of whatever its own match returns. Twelve tracks of
one record used to come back with three different covers, and now it costs one
image download instead of twelve. The `cover.jpg` in the folder stays, because
on an album it is that album's sleeve rather than whichever track resolved
first. Nothing about a playlist changes.

Renaming to add a marker is an ordinary rename, so the note above applies: do
it with `e`, or do it in Finder and then press `e` and Enter on the name it
shows, or the next sync writes the old name back beside it.

## Removing a playlist

`D` removes the playlist under the cursor and asks what goes with it. Three
answers, and only the last one touches your music.

**Forget** drops `.earworm` and the `.m3u8` and keeps the audio. The folder
stays on the list, because any folder of audio does, but it now has no URL:
`--resync` walks past it and `S` would ask for a new one. Not offered on a
folder earworm didn't download, since there is no sidecar to drop.

**Hide** takes the row off the library and changes nothing else. Every file
stays exactly where it is. This is the answer for a folder of music you keep
in `--dir` and don't want earworm listing: deleting it was the only way to get
the row off the screen before, which is a high price for a tidy list. cliamp
keeps its copy, so a playlist you imported still plays there.

Hiding writes a `#hidden` line into `.earworm` in that folder, and taking that
line out brings the folder back. Take the line, not the file: for a folder
earworm downloaded, the rest of that file is its URL, its rename and the name
every track was saved under, and throwing it away downloads the whole playlist
again beside the copies already there. `earworm --check` lists what is
currently hidden, since a hidden folder is one `--resync` passes over with
nothing on screen to say why.

A forgotten folder wears the `local` pill too. The pill means there is no
playlist behind the folder, which is true of one earworm never downloaded and
of one you've told it to forget.

**Delete** removes the folder and its music, and says how many tracks that is
before it does. cliamp's copy goes too.

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
arguments, no list to maintain. Folders with no URL are named in the log pane
and passed over. New tracks download, departed ones get
reported, and a playlist that has since gone private is logged and skipped
rather than stopping the rest.

Most failures are a network blip or a video that was briefly unavailable, so
`r` re-downloads and re-tags every `failed` track in one pass.

Folders downloaded before earworm started recording the URL need one sync by
URL before `--resync` picks them up, and so does anything you copied in
yourself. `S` on the open folder is where that URL is given.
