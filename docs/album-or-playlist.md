# Albums and playlists

**Built.** The marker parser, the `#kind` header, detection, the precedence,
the shared sleeve, the kept folder image and the docs are in. One thing step 3
deliberately left out is not: see the album tag fill below. What the building
changed is at the bottom.

A plan, not a spec. Written after tracing the code and probing yt-dlp; anything
marked **check** is an assumption worth confirming before it costs you a day.

## The problem

earworm treats every folder as a playlist, and two places already know that is
wrong half the time. Both say so in their own comments.

`tag_tracks` fills an empty album tag with the playlist's name
(`worker.rs:1631`). On a real album that is a decent fallback. On a playlist it
stamps "Chill Evenings" as the album on a dozen unrelated records, and the row
has no column for album so nothing on screen says it happened. The only control
is `--no-album`, which is global and permanent.

Worse, and this one has no control at all: `lookup::cover_bytes` is keyed on
each track's own `Match` (`lookup.rs:472`), with no cache and no agreement
check. Twelve tracks of one album resolve twelve times, and a track that
matches a single, a reissue or a compilation comes back with that sleeve. A
folder of one record ends up holding three different covers. Right for a
playlist, which is what the `c` menu's restore row exists to protect; wrong for
an album, where the whole point is that the sleeve is the same.

The folder image is the third case. `apply` writes `cover.jpg` once per run,
from whichever track resolves art first, and `drop_folder_cover` deletes it at
the end because every track carries the art embedded. For a playlist that is
right: the first track's sleeve is no more the folder's than any other's. For
an album it is the folder's sleeve, thrown away.

## What the user asks for

To not have to think about it. An album should tag and illustrate like an
album, a playlist like a playlist, and the one time earworm guesses wrong it
should be one keypress to say so.

## The signals, measured

Probed against a real playlist on 2026-09-22.

**The playlist id, per track.** `%(playlist_id)s` is in the flat listing, so
this costs nothing and needs no URL parsing. An official album release is
`OLAK5uy_…`; a hand-made playlist is `PL…`. This is the strongest signal and
the cheapest, and it is known before a single byte is downloaded.

~~**The uploader, across all tracks.**~~ Wrong, and measured so while
building. yt-dlp reports an Art Track's uploader as `Kraftwerk`, not
`Kraftwerk - Topic`, so one shared channel cannot tell an album from a
single-artist playlist or a channel dump. Dropped, and the field is not in the
scan.

**Lookup agreement, after the pass.** Every track resolving to one release
group. earworm already fetches this per track, so it is free, but it only
arrives once the tagging is done. Useful as a correction, not as the decision.

**Tags on disk.** One album value across every file. The only signal a folder
earworm did not download has, and free.

Track count and duration spread are worthless. A twelve-track playlist and a
twelve-track album are indistinguishable by shape.

~~**check** whether `%(album)s` is populated in the flat listing.~~ Answered
while building: no. A flat entry carries `id`, `title`, `duration`, `channel`,
`channel_id`, `uploader`, `timestamp`, `view_count` and thumbnails, and no
album or release year. The extractor fixes that shape, so an `OLAK5uy_`
listing has the same keys. A single-video fetch does return `album`, but the
scan is deliberately one flat request rather than one per video.

So the playlist id is the only automatic signal there is, which is why
detection promotes and never demotes.

## The name is the override

A bracket group in the folder name whose contents are exactly `album` or
`playlist`, case ignored, anywhere in the name:

```
Autobahn [album]/
Autobahn [album] [1974]/
Chill Evenings [playlist]/
```

earworm reads it and never writes it. That is the whole design, and it buys
more than a setting would.

It is an override anybody already knows how to use. Rename the folder in Finder
or with `e` and you have said which it is, with no key to learn and nothing
hidden in a dotfile. Removing the marker hands the folder back to detection,
which no header can do as cleanly.

It works upstream too. A folder is named from the playlist's title on the first
download, so naming the playlist "Mixtape [playlist]" on YouTube names the
folder that and earworm reads it on arrival. That is the same code path, not a
second feature.

It is visible where the music is. A folder marked in Finder stays marked on a
phone, which is the point of putting it in the name rather than in `.earworm`.

And a folder nobody marks stays clean, which is why earworm must not write it.
A suffix on every folder forever is a high price for a fact that matters twice.

The contents must match the keyword exactly, so `[Album Version]` and
`[Deluxe Edition]` are not markers. That matters: brackets in music folder
names are usually a year, a format or an edition, and a substring match would
read half a library as albums.

`folder_field` already escapes `%` for the yt-dlp template and brackets are
literal there, so a marked name needs nothing extra to survive a download.

## Where the answer lives

Three sources, in this order:

1. **The name**, if it carries a marker. Re-read every time, so it always wins
   and removing it always takes effect.
2. **Detection**, when the listing's playlist id says `OLAK5uy_`. It records
   its answer as it goes.
3. **A `#kind` header** in the sidecar, written only by detection. It fits the
   existing header machinery exactly: invisible to the entry parser, absent
   from every sidecar written so far, and read back by `manifest::load` like
   the other four. Offline it is all there is, which is why it exists.
4. Failing all three, `playlist`.

Detection sits above the header rather than below it, which is not where the
first draft of this put it. They cannot disagree in practice, since the header
is only ever detection's own note and detection never writes `playlist`, so
what the order actually buys is that a sync re-decides and refreshes rather
than letting one old answer stick.

A marked name writes no header. Two records of one fact drift, and the drift
here is silent: a header left behind by a marker somebody has since deleted
would go on deciding with nothing on screen saying why. Detection writes the
header because detection has nowhere else to put its answer; the name does not
need one.

Absent everything means playlist, which is every folder today, so nothing that
exists changes behaviour until it is synced or renamed.

## What changes when the answer is album

**One sleeve for the folder.** The first track to resolve art wins, and every
other track is given that same image rather than its own lookup's. This is the
fix for the three-covers-in-one-record case, and it makes the run faster:
twelve image downloads become one. `CoverState` already carries `written`, so
it is the natural place to hold the bytes as well.

**The folder image stays.** `drop_folder_cover` skips an album, because
`cover.jpg` is that album's sleeve rather than an arbitrary track's.

**The album tag is filled from the playlist name with more confidence,** since
for an `OLAK5uy_` release the playlist name is the album name.

## What changes when the answer is playlist

Nothing, which is the point. Today's behaviour is the playlist behaviour.

The first draft of this said `cfg.album` should stop filling the album tag
from the playlist name, and that was wrong to put here. It contradicts this
plan's own "must not change anything for a folder nobody has synced since",
and it is a silent change to the default path for every existing folder. A
playlist's name genuinely is not any of its tracks' album, so it is worth
doing; it is worth doing on its own, where somebody can decide it, rather than
arriving inside a change about cover art. `--no-album` is the control that
exists meanwhile.

## Saying which

Detection is never silent. The run summary names the kind and where the answer
came from, because getting it wrong on an album gives you exactly the bug this
exists to fix and nothing else on screen would explain it.

```
12 tracks in Autobahn  ·  album , from the folder name
12 tracks in Autobahn  ·  album , from the playlist id
40 tracks in Chill Evenings
```

Naming the source is what makes the override discoverable without a key. A
folder nothing has decided about says nothing, because "playlist" is the
default rather than a finding, and a line claiming a decision nobody made
would be the same false confidence this is trying to avoid.

No new key. The name is the override and `e` already edits it.

## Where it hangs off

- **`manifest.rs`.** A fifth header beside `#url`, `#synced`, `#name` and
  `#m3u8`. `Sidecar.kind`, carried by `Headers` like the rest, plus a
  `set_kind`. The enum is two values and `None`, so it should be a real type
  rather than a `String`, or every reader re-parses it.
- **The marker parser.** One function over a folder name, and the one place
  the bracket rule lives. It is pure text in, `Option<Kind>` out, which is how
  it should be tested: no folder, no sidecar, no run.
- **`ytdlp::scan`.** Two more fields in the `--print` line, which takes it to
  seven and breaks the three stub yt-dlps together, as the duration field did.
  `playlist_id` and `uploader` both need to reach `Listing`, and neither
  belongs on `Track`: they describe the playlist, not the song. A field on
  `Listing` beside `complete`.
- **`pipeline`.** Resolves the kind after the scan and before the tagging pass,
  writes the header only when detection decided it, and hands the answer to
  `tag_tracks`.
- **`tag_tracks` and `tag::apply`.** The album fill is gated on the kind as
  well as on `cfg.album`, and `apply` takes the folder's sleeve rather than
  fetching per track when the kind is album.
- **`drop_folder_cover`.** Skipped for an album.
- **`open_shelf`.** Resolves the kind the same way and holds it for the
  session, so a `c` in a folder opened offline behaves the same as one during a
  run. A folder earworm did not download has no header, so here the name is
  usually the only source there is.
- **The library row and `--list`.** Nothing to add. A marked folder already
  reads `Autobahn [album]` in the name column, which is the whole reason for
  putting the marker there, and a row is already carrying name, count, missing
  and sync time. `shelf_matches` lowercases the name, so `/album` filters the
  library to the marked folders for free.
- **Nothing in `rename_shelf`.** It already moves the `.m3u8`, the `#m3u8`
  header and cliamp's stored copy with the folder, so adding or removing a
  marker with `e` is an ordinary rename and needs no new handling. Renaming in
  Finder instead leaves `#name` stale, which is a pre-existing trap that
  `docs/library.md` already documents (press `e`, Enter on the name it shows).
  This feature gives people a new reason to rename in Finder, so that note
  wants repeating where the marker is described.

## Order of work

### 1. Read it off the name (half a day)

The marker parser and the `Kind` type. Pure text in, `Option<Kind>` out, and
nothing calls it yet.

Tests: `Autobahn [album]` is an album; `Chill Evenings [playlist]` is a
playlist; `[ALBUM]` and `[ Album ]` are too, case and space ignored;
`[Album Version]`, `[Deluxe Edition]`, `[2024]` and `Albumania` are not
markers; a name with both markers is neither, because two answers is no answer;
a bare name is `None`.

That list is the feature. Everything else is plumbing, and the "not markers"
half is what stops a library of `[FLAC]` and `[Remastered]` folders reading as
albums.

### 2. Detect it and record it (a day)

`playlist_id` and `uploader` through the scan onto `Listing`, the detection
that reads them, the `#kind` header, and the precedence. Still no behaviour
change beyond the summary line: it decides, records and says so, and nothing
acts on the answer yet.

Tests: an `OLAK5uy_` listing is an album; a `PL` listing of one Topic channel
is an album; a `PL` listing of many channels is a playlist; an empty or unknown
uploader falls to playlist rather than guessing. Then the precedence, which is
the part worth breaking: a marked name beats a header that disagrees; a marked
name writes no header; removing the marker from a folder that once had one
falls back to detection rather than to a stale header; a sidecar written before
`#kind` existed still loads and reads as playlist.

### 3. Act on it (a day)

The album branch: one sleeve for the folder, the folder image kept, the album
fill gated. This is the step that changes what lands on disk, and the one worth
breaking to watch fail.

Tests: an album folder ends with one image across every track and a `cover.jpg`
still there; a playlist folder ends as it does today, with per-track art and no
folder image; a playlist does not get its name written into the album tag; the
album pass makes one artwork request rather than one per track.

### 4. Say it (half a day)

Built. `docs/library.md` gained an "Albums and playlists" section: the
`OLAK5uy_` signal, the bracket rule and what it refuses, the precedence, the
summary line, and what changes for an album. `docs/keys.md` gained a note in
the cover art section, since that is where somebody wondering why a folder
kept its `cover.jpg` will look. The status list gained nothing, because the
kind never reaches a row. The README gained a feature bullet. CLAUDE.md
already carried the precedence rule, the no-header-for-a-marked-name rule and
the exact-match bracket rule, written while steps 2 and 3 were built.

The summary line lands in step 2, because a step that decides without saying so
is one nobody can check by using it.

## What it must not do

- **Give one album twelve sleeves.** The whole reason this exists.
- **Change anything for a folder nobody has synced since.** Absent means
  playlist means today's behaviour, exactly.
- **Delete a folder image the user put there.** `folder_covers` already
  protects that and the album branch must not route around it.
- **Decide silently.** A wrong guess is cheap to fix and expensive to find.
- **Guess from track count or duration.** Neither carries the answer.
- **Rename anybody's folder.** The marker is read, never written. A folder
  nobody marks keeps the name it has.
- **Read a bracket group that only contains the word.** `[Album Version]` is an
  edition, not an answer, and half a music library is brackets.

## Open questions

- **What an attached folder does.** A folder earworm did not download can now
  be given a URL. Detection runs on that sync like any other, but the folder
  may already hold tags and art from wherever it came from, and overwriting a
  correct album's sleeve with one earworm looked up is a loss. Probably: an
  attached folder's kind is detected and recorded, and the sleeve half waits
  for the user to ask with `c`.
- **Whether `--no-album` survives.** With `#kind` deciding per folder, the
  global flag means "never, even on an album". That may still be a setting
  somebody wants, but it is now the unusual case rather than the only control.

## Effort

Three days. Steps 1 and 2 are a day and a half, ship alone, and change nothing
but a line in the summary, which makes them worth taking first: a fortnight of
runs reporting which kind they think each folder is, while still behaving
exactly as they do today, is how you find out the detection is wrong before it
can cost anybody their cover art.

## What changed in the building

- **The uploader signal does not exist.** yt-dlp normalises an Art Track's
  channel to the bare artist name, so the plan's second signal was never going
  to work. Detection is the `OLAK5uy_` prefix and nothing else, which is why
  it promotes and never demotes: silence has to mean playlist.
- **The flat listing carries no album field**, which closes the one open
  question the plan opened with. Dumped the entry keys rather than guessing.
- **Detection runs before the header, not after.** The plan put the header
  second in precedence. It is third: the name wins, then a listing that says
  `OLAK5uy_`, then the header. In practice they never disagree, because the
  header is only ever written by detection and detection never writes
  `playlist`. Putting detection first means a sync re-decides and refreshes,
  and the header is what `open_shelf` reads when there is no listing to ask.
- **Step 1 could not ship alone.** earworm is a binary crate at zero warnings,
  so a parser nothing calls is two `dead_code` warnings. Worth remembering for
  the next plan: a step that adds only a pure function has no warning-clean
  stopping point in a binary.
- **`open_shelf` reports the kind too.** Not in the plan, and it is what gives
  `kind_of` its second caller: the same answer offline as during a sync, from
  one derivation, which is the rule this codebase keeps.
- **The album tag fill did not change.** Step 3 was meant to gate it on the
  kind and does not, because that contradicts this plan's own must-not: it is
  a silent change to the default path for every folder that exists. Left as a
  separate decision.
- **`tag::sleeve` takes the fetch as a closure.** Without that seam the
  sharing could only be tested against a network, and "the files all match"
  cannot tell one request from twelve. The count is the assertion.
- **Two lines in `pipeline` have no test.** `one_sleeve: album` and the
  `album` argument to `drop_folder_cover` both flip to `false` with the whole
  suite still green, because reaching them needs a live scan. Recorded in
  CLAUDE.md beside the `gate` literal, which has the same problem.
- **One guard turned out to be dead.** `if one_sleeve && found.is_some()` is
  the same as `if one_sleeve`, since storing `None` is what not storing looks
  like. Found by breaking it and watching nothing fail, which is the only way
  that kind of redundancy shows up.
- **The cover art docs were the place the marker had to be mentioned.** Not in
  the plan's list, which named library.md and the status list. Somebody finds
  this feature by wondering why one folder kept its `cover.jpg`, and that
  question is asked at `c`, not at the library screen.
- **The header was written where the answer was decided, and the folder was
  not there yet.** A brand new playlist's folder is made by the download, and
  detection runs before it, so `set_kind` failed with ENOENT on the one run
  that discovered the album: no header, no pill in the library until a second
  sync, an error line in the log pane every first time, and `r` in the same
  session re-resolving `playlist` and deleting the `cover.jpg` the run had
  kept. Found by review, confirmed by probe rather than by reading. Resolving
  and recording are two functions now.
- **Sharing the sleeve without sharing the album tag made the file
  contradict itself.** `apply` still wrote each track's own `m.album`, so an
  album whose fifth track matched a compilation got that compilation's name
  beside the record's sleeve. Worse than the bug it replaced, because the two
  fields are read together. `tag::album_name` is the other half.
- **A folder image the user put there was being overwritten, not deleted.**
  Pre-existing, and the must-not list only said "delete": `folder_covers`
  keeps the name and nothing kept the bytes. `pipeline` had `written: false`
  where `retry` had `has_cover`. Worth remembering when writing a must-not:
  name the loss, not the mechanism.
- **"The library row: nothing to add" was wrong.** The plan reasoned that a
  marked folder already reads `Autobahn [album]` in the name column, and
  forgot the other half: a folder detection promoted has nothing in its name,
  so the row said nothing at all about the answer that had just been reached.
  It wears a filled `album` pill now, and a marked name gives the word up to
  it rather than saying it twice. The slot is only reserved when a row on
  screen needs it, so a library with no album in it is unchanged.
