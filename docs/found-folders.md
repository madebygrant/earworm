# Folders earworm did not download

**Built.** All five steps. See `docs/library.md` for what it does and the
"Files and re-runs" bullets in CLAUDE.md for what holds it up. Kept as the
reasoning behind the shape, with what changed in the building marked below.

A plan, not a spec. Written after tracing the code; anything marked **check**
is an assumption worth confirming before it costs you a day.

## What changed in the building

- **`D` refusing a found folder was the wrong answer, and got replaced.**
  The plan has it refuse, on the grounds that the only row left would be
  deleting somebody's music. That left no way to get such a row off the
  library at all, since any folder of audio is on it. It now opens the menu
  without the forget row and with a hide row, which takes the row away and
  keeps every file. The row text moved on too: it is a filled `local` pill
  rather than a word at the end of the line.
- **`scan` needs no `~` guard.** It indexes the manifest *by* video id, and no
  video id starts with `~`, so a local entry is unreachable from there. Two
  readers needed one, not three: `note_departures` and `write_archive`, both
  of which walk the manifest rather than indexing into it. Written and then
  removed after breaking it left the test green.
- **The flat listing carries duration and uploader.** The open question below
  is answered: `%(duration)s` is now in the scan's `--print`, which took
  `splitn` to 5 fields and broke three stub yt-dlps at once. Uploader is there
  too and is not used: title and duration placed every fixture, and an artist
  rule would be a fourth thing to get wrong.
- **`#m3u8` exists.** The rule "adoption never replaces an existing playlist
  file" is not enough on its own: after adoption the sidecar is there, so the
  second edit would replace the file the first one spared. earworm records the
  playlist it wrote and rewrites only that.
- **`write_manifest` had to drop entries by file as well as by id.** A match
  rewrites a `~` id to a video id and both lines name one file, so the sidecar
  ended up holding two entries for one track.
- **`forget` no longer takes a row off the library.** It drops the sidecar and
  keeps the audio, and a folder of audio is on the list either way now. What
  it loses is its URL, and the flash says so.
- **The row says `no playlist url`, not `not synced`.** Too close to the
  existing `never synced`, which means something different.
- **`Pass` replaced the bare `gate` bool**, so the forced gate and the
  reconcile flag are named at every call site rather than positional.
- **Three of the bugs were on the read-back path, not the sync.** The plan
  reasoned about what a sync produces and stopped there, but a folder is
  reopened from disk far more often than it is attached. `Local` was lost on
  reopen, a local file stole a playlist number on reopen, and an adopted
  folder never noticed a song added to it. Worth remembering for the next
  feature that introduces a status: set it in one place, read it back in
  another, and test the round trip rather than the half.
- **`contents` is one derivation for both readers.** `library` and
  `open_shelf` were each deciding whether to believe the sidecar or the
  directory, and they have to agree or the row count and the track list say
  different things about one folder.
- **`Local` had to survive a reopen.** The plan set the status during the
  sync and stopped there. Reopening the folder reads it back out of the
  sidecar, and `open_shelf` handed every entry `Have` and `listed`, after
  which `mark_departed` called it `gone` off the playlist the sync had
  correctly left it out of. Two lines, both now tested.

## The problem

The library lists a folder only if its `.earworm` sidecar has a `#url` line.
The gate is one line, `let url = sidecar.url?;` in `worker::library`, and
everything below it assumes the folder passed: `Shelf.url` is a plain `String`,
`open_shelf` bails when the manifest has no entries, `S` syncs from the
recorded URL, `--resync` walks every shelf and syncs it, `--list` prints the
URL in its last column, and `mark_departed` reads the folder's `.m3u8` as
earworm's own last answer about the playlist.

So a folder of tagged audio under `--dir` that earworm never wrote, with or
without a playlist file, is invisible. The tags and filenames are everything
earworm needs to show it and edit it, and nothing else about the tool depends
on where the files came from.

## What the user asks for

Three things, in the order they become wanted:

1. **See it.** The folder appears in the library with its track count, and
   opens to a track list built from the tags on disk, exactly as a synced
   folder opens offline today.
2. **Edit it.** `e`, `c`, `s`, `A`, `u` and the rest work on it. Tags are
   tags.
3. **Attach it.** Give it a URL and let a sync fill in what is missing,
   without downloading what is already there.

The first is an afternoon. The third is where the trap is, and the design
below exists to close it before the second is allowed to open it.

## Three states for a folder

- **Found.** Audio files, no sidecar. Read-only as far as earworm is
  concerned: nothing in the folder is written until the user changes
  something in it.
- **Adopted.** earworm has written a sidecar, because the user edited a tag
  or renamed the folder. The sidecar has no `#url` and its ids are not video
  ids. Every post-run command works.
- **Attached.** A URL was given and a sync has run. The sidecar has a `#url`
  and video ids for every file the sync could match. Anything it could not
  match keeps its local id and is shown as such.

A synced folder today is the third state with nothing local in it. Nothing
about that path changes.

## The trap, and the marker that closes it

Every edit calls `save_manifest`, so the first edit in a found folder writes
a sidecar, and the ids in it have to be invented: there is no video id. If
somebody later attaches a URL and syncs, `scan` looks each listed video's id
up in `manifest::read`, finds nothing, predicts the filename from the
playlist template, finds nothing there either, and downloads the whole
playlist again beside the files already on disk. CLAUDE.md records that
failure twice already, reached by other roads.

The marker is on the id, not on the file. A local id is the filename at the
moment of adoption with `~` in front: `~03 - Some Track.m4a`. Filenames are
unique within a folder, so it is unique; it is assigned once and never
changes, so a later rename through `e` updates the filename column and leaves
the id alone, which is what the manifest does for video ids already; and it
is readable in the file, which a hash is not.

Per id rather than a `#ids local` header, because a folder can be half
reconciled: the sync matches eight files, cannot place two, and the two keep
their local ids beside eight real ones. A header is one bit for the whole
folder and would have to be cleared when the last local id went, or left on
for good. A sigil on the id costs one `starts_with('~')` at each of the four
places that read ids, and it composes with everything the sidecar already
does: `#`-prefixed lines stay headers, the entry parser is untouched, and a
manifest written before this existed has no `~` in it and reads exactly as
before.

The four places, and what each does with a `~` id:

- **`ytdlp::scan`** (`ytdlp.rs:122`) matches listed videos to files by id.
  A `~` id can never match a video id, so it skips them. The matching a
  reconciling sync does instead is described below.
- **`note_departures`** (`worker.rs:2370`) calls every manifest id not in
  the listing departed. A `~` id is never in any listing and is never
  departed. Without this, attaching a URL would call every unmatched local
  file `gone` and offer to delete it.
- **`write_archive`** names every id whose file exists so yt-dlp skips it.
  A `~` id is meaningless to yt-dlp and is left out. Harmless either way, but
  the archive is read by a tool that did not write it, so it gets only what
  that tool understands.
- **`write_manifest`** preserves entries the current tracks do not cover. It
  already does, keyed on the file existing. Nothing to change, but the test
  for adoption has to prove a `~` entry survives a sync that did not match
  it.

## Reconciling, when a URL is attached

The first sync of a folder holding any `~` id is different from every other
sync in one way: it must not download anything it might already have. It
lists the playlist, then for every listed video tries to find its file on
disk, in this order:

1. **The predicted filename**, exactly as `scan` does now. This catches a
   folder somebody made with earworm and then lost the sidecar from.
2. **The title.** The file's title tag against the listed video's title,
   both lowercased, composed to NFC and trimmed, the same way
   `shelf_matches` and `mark_departed` already normalise. Then the same
   against the filename stem with any leading number and separator stripped,
   for a file with no tags. **check** what the flat listing carries besides
   `id`, `title` and `duration`: an Art Track's title is the bare song title
   and the artist sits in the uploader field, so artist matching may need
   `%(uploader)s` with ` - Topic` stripped, or may not be worth having.
3. **Duration** as a tiebreaker only, within two seconds. Never on its own:
   every three-minute song is three minutes.

A match rewrites that entry's id from `~name` to the video id, in place, and
the track is `Have`. A listed video with no match is `Pending` and downloads,
which is the ordinary case of a playlist holding tracks the folder does not.
A file with no match keeps its `~` id and gets a status of its own, below.

Two guards on that first sync, because a false negative is exactly the
duplicate the marker exists to prevent:

- **It goes through the pick gate whatever called it.** `pipeline` takes
  `gate` as a parameter and `resync` passes `false`; a reconciling sync
  passes `true` regardless. The user sees "12 to download" against a folder
  they know holds twelve tracks and stops it. CLAUDE.md calls that `false`
  literal load-bearing, and this is the one condition that overrides it.
- **`--resync` skips a folder with no URL** and logs why, once per folder.
  Attaching is an interactive act through `S`, never something an unattended
  walk of the library does.

After the first sync the folder is attached. Later syncs behave exactly as
they do for any folder, because `scan` and `note_departures` skip `~` ids and
the matched ones are real. Nothing is re-reconciled: a file that was not
matched the first time is a local extra, and stays one until somebody renames
or retags it and syncs again, which runs the same matching over the `~`
entries only.

## A status for a file the playlist does not have

A reconciled folder can hold files that are on disk and not in the playlist.
That is what `Gone` means today, except `Gone` also says the file *was* in
the playlist once, and `D` is allowed to delete a proven one. Neither is true
of a file the user put there themselves.

So a new `Status::Local`, row word `local`, dim like `kept`: it is settled and
nothing about it is a guess earworm made. `settled()` and `tally()` are
exhaustive matches, so adding it fails to compile until both have an answer,
which is the safety CLAUDE.md built into them. It is `listed = false`, like
`Gone`, so the `.m3u8` a sync writes names the playlist and not the folder.
`D` never reaches it: `can_purge` reads `Gone && departure_proven` and a
`Local` track is neither. `tag_tracks` skips it beside `Gone` and `Skipped`,
since a file the playlist does not know about is not one the lookup may
retag. Every edit key works on it, since the user owns it more than they own
anything else in the folder.

Before a URL is attached there are no `Local` rows. A found or adopted folder
*is* its own playlist, so every file is `Have`, `listed = true`, and the
`.m3u8` earworm would write names all of them.

## Where it hangs off

- **`worker::library`.** Make the URL optional and, when there is no sidecar,
  build `files` from a directory read filtered on the extensions in
  `FORMATS`. That is one stat per file, which is what the manifest path costs
  today. Two new fields on `Shelf`: `url: Option<String>` and
  `playlist: Option<PathBuf>`, the second being the folder's `.m3u8` if it
  has exactly one, whatever it is called. Sorting and `off_format` read
  filenames and are unaffected.
- **`open_shelf`.** With a sidecar it does what it does. Without one it walks
  the same directory listing, hands every file a `~` id, and builds the
  track from the tags as it already does for a manifest entry. `numbered`
  already returns `None` for a name with no leading number and the renumber
  below it hands out the rest, so arbitrary filenames already work.
  `mark_departed` is skipped when there is no `#url`: the `.m3u8` in a found
  folder is not earworm's last answer about anything.
- **`save_manifest`.** Already writes whatever ids the tracks carry and
  already preserves entries it does not cover. Adoption is this function
  running for the first time in a folder, on the first edit. It says so once,
  as a flash: `adopted Neon Venom  ·  earworm keeps a .earworm here now`.
  Writing a hidden file into somebody's folder on an edit is the one moment
  they asked earworm to change that folder.
- **`write_playlist`.** Runs on every edit and writes `<folder>.m3u8`. In a
  found folder that name may already be the user's own playlist file, and
  the first edit would replace it. Rule: adoption never writes a playlist
  file over one that exists; only a sync does, since a sync is the point at
  which earworm has an answer worth writing. `Shelf.playlist` is what says
  whether one is there.
- **`S` on the open track list.** Today it syncs from `cfg.url`. With no URL
  it prompts for one, the same prompt `n` uses, sets `cfg.folder` to this
  folder's name so `output_template` writes into it rather than into
  `%(playlist)s`, runs the reconciling sync, and writes `#url` and `#name`
  into the sidecar. `#name` is what makes the next sync land here too.
- **`p`.** `player::load` builds the `.m3u8` path from the folder name. Take
  `Shelf.playlist` instead, so a found folder plays from whatever playlist
  file it has, and hides `p` with a reason when it has none. **check** that
  cliamp's `playlist import` accepts a file not named after its folder; it
  takes a path, so it should.
- **`D` on the library.** Forgetting means deleting the sidecar, which a
  found folder does not have. It refuses on a found folder and says earworm
  did not put these files here, since the delete option is the user's music
  and no undo. On an adopted folder, forget removes the sidecar and the
  folder becomes found again, which is the reversible half of adoption.
- **`e` on the library** writes `#name` through `set_name` and so adopts the
  folder. Fine, same rule: a write is a write.
- **The row, the preview, `--list`.** A found or adopted row says
  `not synced` where a synced one says `synced 3d ago` and the URL column in
  `--list` prints `-`. The preview's head already counts tracks and missing.
  No colour carries this: the word does.

## Order of work

### 1. See it (half a day)

`Shelf.url` optional, `Shelf.playlist`, the directory walk in `library`, the
`--resync` skip, `S` refusing with a reason, `p` following `Shelf.playlist`,
`D` refusing on a found folder, the row and `--list` words. Every key that
meant "playlist" either works or says why it cannot.

Tests: a folder of three `.m4a` files and no sidecar appears with three
tracks and no URL; a folder with a sidecar and no `#url` appears the same
way; `--resync` over a library holding one found folder syncs the others and
logs the skip; `--list` prints `-`; the row reads `not synced`.

### 2. Open it (half a day)

`open_shelf` from the directory, `~` ids, `mark_departed` skipped without a
URL. Nothing is written.

Tests: opening a found folder yields `Have` rows with tags off disk and
indices from the filenames; a found folder with a `.m3u8` naming half its
files does not call the other half `gone`; opening writes no file.

### 3. Adopt it (half a day)

The flash on first write, `write_playlist` refusing to replace an existing
file, and the four `~` guards in `scan`, `note_departures`, `write_archive`
and `write_manifest`.

Tests: an edit in a found folder writes a sidecar with `~` ids and says so
once; a second edit says nothing; the user's `Neon Venom.m3u8` survives the
first edit byte for byte; a sidecar holding `~` ids run through a stub yt-dlp
sync requests no download for a matched file and never marks a `~` entry
departed. That last one is the trap, and it is the test to write before any
of this ships: build the sidecar by hand, run the sync against the stub, and
read the archive file and the stub's argv, exactly as
`a_resync_keeps_the_name_an_edit_gave_a_track` does for renames.

### 4. Attach it (a day and a half)

The URL prompt behind `S`, `cfg.folder`, the matching pass in the order
above, `Status::Local` with its two exhaustive arms, the forced pick gate.

Tests: each matching rule alone, through a stub listing; a tie on title
broken by duration and not by position; a match rewrites the id in place and
the file is not downloaded; a listed video with no match is `Pending`; an
unmatched file is `Local`, `listed = false`, absent from the written
`.m3u8`, untouched by `tag_tracks`, and refused by `D`; a reconciling sync
under `--resync` is skipped, not gated.

### 5. Say it (half a day)

`docs/library.md` gains a section, `docs/keys.md` the refusals, CLAUDE.md the
bullets: the `~` sigil and why per id, the forced gate as the one override of
the `false` literal, the `.m3u8` rule on adoption, and `Local` beside `Gone`
in the list of statuses `tag_tracks` skips.

## What it must not do

- **Download a file it already has.** Everything above is in service of this
  one line. The pick gate is the last defence when the matching is wrong.
- **Call a `~` entry departed.** `note_departures` is the one place that
  decides, and the test for it is in step 3.
- **Write anything into a found folder before the user edits something in
  it.** Listing and opening are reads.
- **Replace a playlist file it did not write.** A sync may; adoption may not.
- **Delete a user's cover.** `folder_covers` already protects an image that
  was there before a pass. Unchanged, but worth the test on a found folder.
- **Let `--resync` attach anything.** The walk is unattended by design.
- **Tell them apart by colour.** `local`, `not synced`, `-` are words.

## Open questions

- ~~**What the flat listing carries.**~~ Answered: `id`, `playlist_index`,
  `title`, `duration` and `uploader`, measured against a real playlist.
  Duration is used as the tiebreaker; uploader is available and unused.
- **Whether adoption should be explicit.** Shipped as the middle road: it
  adopts on the first edit and flashes once. If that reads as a surprise in
  use, a confirm on the first edit is a one-line change to where the flash is,
  in `save_manifest`.
- **Whether matching should try the artist.** The uploader field is in the
  listing already. It was left out because title and duration placed every
  case tried, and a fourth rule is a fourth thing to get wrong. Worth
  revisiting only if a real folder comes back with tracks it could not place.
- **Hidden and nested folders.** `library` reads one level under `--dir` and
  skips nothing by name. A found folder full of `.DS_Store` and one `.m4a` is
  a one-track playlist. Filtering on `FORMATS` extensions handles the first;
  whether to descend is a different feature and out of scope here.

## Effort

Three to three and a half days for the whole of it. The first two steps are
one day and are worth shipping alone: a folder somebody copied in by hand
becomes browsable and playable with nothing written to it. Step 3 is what
makes editing safe to allow, and step 4 is the reason step 3 is shaped the
way it is.
