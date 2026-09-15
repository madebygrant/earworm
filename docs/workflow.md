# Day to day: picking, filtering, fixing

## Running it

```sh
earworm                                  # asks for a URL
earworm 'https://youtube.com/playlist?list=...'
earworm URL --dir ~/Music/new
earworm URL --no-pick                    # don't stop to choose, fetch it all
earworm URL --format flac                # opus by default
earworm URL -- --cookies-from-browser firefox    # extra args go to yt-dlp
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

## Notes

- Only downloads what you have the right to download.
- Editing tags renames the file and rewrites the `.m3u8` to match.
  `--no-rename` turns that off.
- Quitting kills the yt-dlp child and lets a tag write already under way
  finish.
