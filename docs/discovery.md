# Artists and discovery

`b` on the library lists every artist your tags name, with how many tracks and
albums of theirs you have. Starred artists come first, then the rest by track
count. `/` filters the list, and ignores accents and case, so `bjork` finds
Björk.

An artist is the lead credit on a track. "Massive Attack feat. Elizabeth
Fraser" counts towards Massive Attack. A list like "Alestorm, Christopher
Bowes" counts towards Alestorm once your library shows Alestorm on its own:
credited alone, or leading lists that go on to different names. A band name
with `&`, `,`, `/` or "with" in it stays whole, and "Earth, Wind & Fire, The
Emotions" counts towards Earth, Wind & Fire. A star on a whole list from an
earlier version moves to its artist the next time you open `b`. A song you have on several playlists counts once, and
"Various Artists" isn't listed.

## Starring

`*` stars or unstars the artist under the cursor. On the track list it stars
the artist of the track under the cursor. Stars are kept in `.artists` in your
library folder, so they travel with it.

## An artist's page

Enter on an artist opens their page, built from Deezer. The header names the
artist Deezer took them to be and their follower count, since several acts can
share a name, and how many of their tracks you have.

Tab moves between three lists.

**Releases** marks each one against your library:

- `● owned`: every track is here.
- `◐ 7 of 10`: some of it is.
- `● 3 here`: some is here and Deezer didn't say how long it is.
- `○ missing`: none of it.

A deluxe, remastered or anniversary edition counts as the same record. Live,
acoustic and instrumental versions don't. Albums and EPs show by default; `a`
adds singles and compilations.

**Top tracks** is the artist's ten most played on Deezer, each `● here` or
`○ missing`.

**Related artists** lists who Deezer thinks is like them, and how many of their
tracks you have. Enter opens their page, and Esc steps back through the pages
you came from.

`o` on the artists screen or a page narrows the library to the folders holding
that artist's tracks. The band says `ARTIST` and Esc clears it.

## Getting what's missing

Enter on a missing or part-owned release, or on the top tracks, downloads them
from YouTube Music. This needs YouTube Music links unlocked for the session,
the same unlock the URL prompt asks for. Locked, the page only lists: Enter
on a release or the top tracks does nothing, and no hint or help row mentions
downloading.

earworm searches YouTube Music for the record by that artist, then asks before
downloading, naming what it found, who made it and how many tracks. An album
lands in a folder named after it, the same one if you fetch it again, and goes
through the usual pick gate.
For one you own part of, the tracks you already have start unmarked. Top
tracks land in `<Artist> - Top Tracks`, and a later fetch adds to it. Any it
couldn't find are listed in the log pane (`l`).

Each top track is its own search, a few seconds apiece. `Esc` stops the search
after the one under way, and nothing is downloaded. Folder names have `/` and
`\` turned into `-`, so top tracks for AC/DC land in `AC-DC - Top Tracks`.

## New releases

Once you've starred someone, earworm checks for their new releases once each
time it starts, in the background. The library header counts them, and the
artists screen marks the artist `+ 2 new`. Opening the artist's page marks the
new releases `✦ new` and stops counting them.

The first check for an artist only notes what's already out, so their back
catalogue isn't reported as new. Releases already seen are kept in
`.artists-seen` beside `.artists`. Nothing is written until you star someone.
