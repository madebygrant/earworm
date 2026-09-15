# Bringing a playlist to the current format

**Built.** Kept as the record of why it is shaped this way. What actually
landed differs from the plan below in four places, all of them found while
building it:

- **A `convert` key, not a format change.** As planned, and the part worth
  keeping: `format` means "format for new downloads" and a menu pick must not
  rewrite files it said nothing about.
- **`alac` and `m4a` share an extension**, so the decision reads the codec off
  the container for `.m4a` files. `Mp4Properties::codec()` exists in lofty
  0.25, so this cost nothing beyond the call.
- **A lossy source must not become a 24-bit lossless file.** ffmpeg decodes
  opus to float and then writes 24-bit flac, twice the size to store the
  decoder's own rounding. On the test folder that was 436MB against 235MB for
  the same audio. Not in the plan at all, and the single largest thing it got
  wrong.
- **The swap renames over the old file.** `fs::rename` replaces atomically, so
  the in-place case never has a moment with no file in it.

Measured on a copy of a real twelve-track folder: 83MB of opus became 235MB of
flac, 90MB of m4a, or 239MB of alac, with names, tags and art intact each time
and a second pass over an already-converted folder doing nothing at all.

A plan, not a spec. Written after tracing the code; anything marked **check**
is an assumption worth confirming before it costs you a day.

Second draft. The first hung the feature off a new key and a `Cmd::Convert`.
This one hangs it off the sync, which turns out to be where most of the work
already happens.

## The problem

`Config::build` refuses an unknown format and `set_format` writes the choice
back, but nothing acts on it retroactively. `scan_with` looks each id up in the
sidecar, finds the existing file whatever its extension, marks it `Have`, and
`write_archive` puts that id in yt-dlp's archive so the download skips it.

So a format change applies to tracks fetched *after* it. A folder goes mixed by
accretion, and there is no way to say "make this one uniform".

Note what is **not** wrong today: nothing duplicates. The sidecar is doing its
job. This is a missing feature, not a bug.

## What the user asks for

One verb, and it already exists: sync. "Sync" means "make the folder match the
playlist"; this widens it to "and match the settings". No new key, no codec
knowledge, no command that works on four formats out of six.

## The trigger is a setting, not the format alone

The format setting means "format for new downloads". The menu says so, the
README says so, and CLAUDE.md records why: re-fetching a whole folder is a far
more expensive guess than leaving it alone. If a plain format change made the
next `--resync` rewrite the library, someone who set flac for one car stereo
would find their whole collection several times larger, from a keypress that
said nothing about the files already on disk. That is the one thing the tool
promised not to do.

So: a second key, `convert`, off by default.

- **`format`** says what new tracks are.
- **`convert`** says whether tracks already on disk follow. Only a sync acts on
  it. Opening a folder from the library converts nothing, because that path is
  offline by design.

`--no-convert` follows the flag convention (the command line can only switch a
feature off; the file is the only way to hold one on), so a script can pin a
sync to "download only" whatever the config says.

Where it gets switched on is the format menu, because that is the moment the
question is in the user's head. After a pick, when `convert` is off, one
follow-up:

> Bring tracks already on disk to flac when their playlist next syncs?

with the lossy-to-lossless caveat as the note (see the last section). Yes
writes `convert = true` through the same one-line editor `save_format` uses,
which wants generalising to a key and a value.

## The decision, per track

Taken at the start of `pipeline`, after the scan and before the download, over
the tracks the scan marked `Have`.

| Current | Action | Why |
| --- | --- | --- |
| already the target | skip | nothing to do |
| `opus`, `m4a` (AAC) | convert | yt-dlp remuxes rather than re-encodes when the container matches the source stream, so the file on disk *is* YouTube's audio. ffmpeg gives the same result a download would. |
| `flac`, `alac` | convert | lossless in, so nothing extra is lost |
| `mp3`, `vorbis` | re-download | ffmpeg wrote these from the source, so converting stacks a second lossy generation. A download is one generation from the same origin. |
| `Gone`, `Skipped`, `Pending` | leave | departed, never fetched, or about to be fetched in the target format anyway |

The remux claim is **confirmed**: `ffprobe` on a real earworm download tags
`ENCODER=Lavf` (the muxer) where a re-encode tags `Lavc… libopus`, and the
stream is 130 kbps, which is YouTube's itag 251, not an ffmpeg default.

**`alac` and `m4a` share an extension.** Both write `.m4a`, so the extension
cannot say which one a file is, and "already the target" is wrong in both
directions: an AAC file sits in an alac folder untouched, and an alac file in
an m4a folder does too. lofty's `Mp4Properties::codec()` answers it
(`Mp4Codec::Aac` against `Mp4Codec::Alac`); the decision has to go through a
tag read for `.m4a` and can stay on the extension for everything else.
**check:** that lofty 0.25 exposes the codec on the properties it already
reads for `duration`, so this costs no second parse.

## Where it hangs off, and what the sync gives for free

Everything runs inside `pipeline`, so it applies whether the run started from
a URL or from `--resync`. Sync is the verb either way.

**The re-download half collapses.** yt-dlp is already running against this
playlist. `write_archive` names every id whose file exists; leave the
re-download set out of it and yt-dlp fetches those tracks in the same batch as
the new ones. The first draft budgeted a day and a half for a separate
download run. It is now one parameter:

```rust
fn write_archive(tracks: &[Track], path: &Path, refetch: &HashSet<usize>) -> Result<usize>
```

**Do not add a `Status::Refetch`.** `counts()` is hand-kept and does not fail
to compile when a variant is missing, which is how `Skipped` came to be
counted in the header total and left out of the tally beside it.

**Departures are already decided.** A track whose video left the playlist
comes back `Gone` from the scan and is left alone, as `tag_tracks` already
leaves it. An mp3 whose video is still listed but no longer available fails
inside yt-dlp, and nothing happens to the mp3: the manifest still points at
it, the row stays `Have`, and the summary says `left 1 (video no longer
available)`. No rule needed, the structure does it.

**The manifest is keyed by id.** The swap is "point this id at the new file",
and `write_manifest` already writes whatever `track.path` says. No new manifest
code.

**`gate` stays false.** `--resync` walks the library unattended and a question
per folder is the one thing that would stop it. That is exactly why the
decision to convert has to be a setting read before the walk, not a prompt
during it.

## Order of work

### 1. Report before acting (half a day)

A count of tracks not in the current format, on the library row and in
`--check`. `Shelf.files` already carries every filename, so this is a string
comparison per file and no disk. The `.m4a` case is the exception and can be
reported as "m4a" without saying which; the sync resolves it.

Useful on its own, answers "did my format change take", and it is what makes
an unattended sync legible afterwards: the row says twelve before and none
after.

### 2. Read the cover (done)

`tag::read_cover` landed with the cover cap. Pulls `PictureType::CoverFront`
off the primary tag, falling back to the first picture.

### 3. The setting (half a day)

`convert` in `FileConfig` and `Config`, `--no-convert` in `Cli` through `off`,
`save_format` generalised to `save_key(path, key, value)`, and the follow-up in
`set_format`. `Msg::Settings` already exists for the overlay to pick it up.

### 4. The swap (a day)

One function, used by both halves, given an old path and a new one:

1. `tag::read` the old file, plus `tag::read_cover`. Before anything is
   written, because these are the only record of a hand-typed correction.
2. Verify the new file: non-empty, and `tag::read` can parse it back. ffmpeg
   exits non-zero when it half-writes, but a full disk is the case that makes
   the parse worth having.
3. `tag::set_fields` and `tag::set_cover` onto the new file. **lofty, not
   ffmpeg's `-map_metadata`**: `with_tag` already knows which tag type each
   container takes, and ffmpeg's field mapping across containers is
   inconsistent in exactly the way that cost a day the first time.
4. Rename to the old file's stem with the new extension, so an edited name
   survives.
5. `track.path` to the new file, `save_manifest`.
6. Delete the old file.

Anything that fails at any step leaves the old file, the manifest entry and
the row exactly as they were. Saving the manifest per track rather than at the
end is what makes a cancelled run leave every track either fully swapped or
fully untouched; the sidecar is a small text file and 300 rewrites cost
nothing next to 300 transcodes.

The track lands back in `Have` afterwards, with `source: "converted"` on the
`Msg::Update` rather than a new settled status. `tag_tracks`'s `Have` branch
then reads the tags off the new file and neither identifies nor renames, which
is the point: a converted track is not a fresh download and must not go
through the lookup, or the hand-typed correction it just carried across is
replaced by a guess.

### 5. The two halves (a day)

**Re-download.** Before `run`, remember the old path per refetch index,
because the `@D` line overwrites `track.path` with wherever yt-dlp put the new
file. After `run`, for each: if the path changed and the swap succeeds, done;
otherwise put the old path and `Have` back and count it as left. The old
`.mp3` and the new `.opus` differ in name, so both exist while the swap
happens.

**Convert.** A stage between the download and the tagging, `converting` in the
header, running `ffmpeg -v error -i <old> -vn -c:a <codec> <new>` through
`lookup::run_bounded` with the stdio set by the caller, then the swap. Same
stage word on the row: `Status::Converting`, label `convert`, since
`STATUS_WIDTH` is 8. Adding the variant touches `label()`, `color()` and
`settled()`, all exhaustive, and **`counts()`, which is not**.

`FORMATS` carries the yt-dlp name and the extension; ffmpeg wants a codec
name, which is a third column. `vorbis` is `libvorbis` on most builds and the
native encoder on others, which the fixtures in `tag.rs` already work around
by trying a list. Reuse that.

### 6. Say what happened (half a day)

`summarise` gains `converted N` and `left N (video no longer available)`.
Both are said out loud because the rows scroll off during a library walk and
`--resync`'s per-folder log line is the only record.

## What it must not do

**Never delete before the replacement is verified.** A video pulled from
YouTube since the first download cannot be re-fetched, and deleting first
loses it for good.

**Never silently downgrade.** An `mp3` whose video has gone has no good route:
converting it would produce a worse file to keep a count tidy. Leave it and
say so.

**Never convert on a format change alone.** See the trigger section. The
CLAUDE.md bullet "Changing the format converts nothing" becomes "Changing the
format converts nothing unless `convert` is on, and then only a sync does",
and the guarantee moves from the format to the second key.

**Leave `Gone` tracks alone.** Their index is synthetic and they hold no
playlist position.

**Only what this sync listed.** A run narrowed by `--playlist-items` converts
only the tracks in its listing; the rest wait for a full sync. Converting
files the scan never mentioned would mean walking the folder, which is a
different feature.

## Tests

Break each one before trusting it. Three tests in this repo have passed
against the bug they described.

- Tags, album, year, cover and an edited filename survive a container change.
  Drive it off `FORMATS` like the existing container test, so a format added
  to the table without a working conversion fails here.
- A failed ffmpeg leaves the old file, the manifest entry and the row as they
  were.
- A track already in the target format is not rewritten. Include an AAC `.m4a`
  under `alac` and an alac `.m4a` under `m4a`, both of which *are* rewritten.
- With `convert` off, a sync over a mixed folder changes no file. This is the
  test that pins the promise.
- The refetch set is what the archive honours: a stub yt-dlp, the argument
  read back, the way `the_scan_predicts_filenames_in_the_chosen_format` does.
  Asserting on the helper would leave the call site free to pass anything.
- A refetch whose download fails keeps its mp3, its manifest line and its row,
  and is counted as left.
- A cancelled run leaves every track either swapped or untouched.
- `--no-convert` beats `convert = true` in the file.

## Effort

| Stage | Estimate |
| --- | --- |
| 1. Report the mismatch | half a day |
| 2. `tag::read_cover` | done |
| 3. The setting | half a day |
| 4. The swap | a day |
| 5. The two halves | a day |
| 6. The summary | half a day |
| **Total** | **about 2.5 days** |

Down from three in the first draft, with the key, the `Cmd`, the marked
selection, the library-screen variant and a second download orchestration all
gone. Stage 1 still ships on its own.

## What this buys over re-downloading everything

- No network for the common case. A 300-track playlist is roughly 1.5GB saved.
- It works offline for four of the six formats.
- It fixes tracks whose YouTube videos have been deleted, which re-downloading
  can never do.

And what it costs: both copies of a track exist briefly, so the folder needs
headroom for its largest file, not for a second copy of itself. A library
brought to flac needs headroom for the whole thing several times over, which
is the next section.

## One thing to say out loud in the UI

YouTube serves Opus or AAC. Bringing a playlist to flac gives a lossless
container around audio that was already lossy: no fidelity is recovered and
the folder gets about three times larger. The feature earns its place for
players that cannot read Opus, which is a real problem people have. It is not
a quality upgrade, and the `convert` follow-up is where the menu has to say
so, because once it is on the next sync does the whole library without asking
again.
