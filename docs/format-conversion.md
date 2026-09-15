# Bringing a playlist to the current format

A plan, not a spec. Written after tracing the code; anything marked **check**
is an assumption worth confirming before it costs you a day.

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

One verb. "Bring this playlist to flac." No knowledge of codecs required, no
command that works on four formats out of six.

## The decision, per track

| Current | Action | Why |
| --- | --- | --- |
| already the target | skip | nothing to do |
| `opus`, `m4a` | convert | yt-dlp remuxes rather than re-encodes when the container matches the source stream, so the file on disk *is* YouTube's audio. ffmpeg gives the same result a download would. |
| `flac`, `alac` | convert | lossless in, so nothing extra is lost |
| `mp3`, `vorbis` | re-download | ffmpeg wrote these from the source, so converting stacks a second lossy generation. A download is one generation from the same origin. |
| `Gone`, `Skipped` | leave | no playlist position, or never fetched |

**check:** the opus remux claim. `yt-dlp --extract-audio --audio-format opus`
against a webm/Opus source should remux. Confirm with `ffprobe` on a real
download before leaning on it, because the whole "convert is free" argument
rests on it. If it turns out earworm's opus files are re-encodes, the table
collapses to "re-download everything" and the plan gets more expensive, not
less.

## Where it hangs off

Two entry points, one implementation.

- **After a format change.** `set_format` already knows the old format and the
  new one. One extra row: `12 tracks are still opus. Bring them to flac?`
  That is the moment the question is in the user's head.
- **A standalone key** on the track list, taking `targets()` so a marked
  selection works, exactly as `Cmd::Swap` and `Cmd::Artist` do. Also a
  library-screen version for a whole folder without opening it.

New `Cmd::Convert(Vec<usize>)`.

## Order of work

### 1. Report before acting (half a day)

A count of tracks not in the current format, on the library row and in
`--check`. No deletion, no download.

Useful on its own, answers "did my format change take", and it is the input
the rest of the plan needs anyway. If you build nothing else, build this.

### 2. Read the cover (an hour)

`tag.rs` has `set_cover` and no reader. Conversion has to carry the embedded
picture across, so add the getter beside it:

```rust
pub fn read_cover(path: &Path) -> Option<Vec<u8>>
```

Pulling `PictureType::CoverFront` off the primary tag, falling back to the
first picture. Small, and testable against the fixtures
`writes_tags_into_every_container_earworm_downloads` already builds.

### 3. The convert half (1.5 to 2 days)

Per track, and the ordering is the design:

1. `tag::read` the old file, plus `tag::read_cover`. Before anything is
   written, because these are the only record of a hand-typed correction.
2. `ffmpeg -v error -i <old> -vn -c:a <codec> <new>` through
   `lookup::run_bounded`, stdio set by the caller. `std::process::Command` has
   no timeout and a piped stream nobody drains blocks the child.
3. Verify: exit status zero, output non-empty, and `tag::read` can parse it
   back. An ffmpeg that half-wrote a file exits non-zero, but a full disk is
   the case that makes the third check worth having.
4. `tag::set_fields` and `tag::set_cover` onto the new file. **lofty, not
   ffmpeg's `-map_metadata`**: `with_tag` already knows which tag type each
   container takes, and ffmpeg's field mapping across containers is
   inconsistent in exactly the way that cost a day the first time.
5. `rename` to restore the edited filename with the new extension.
6. Point the manifest at the new file and `save_manifest`.
7. Delete the old file.

Anything that fails at any step leaves the old file where it was.

`FORMATS` carries the yt-dlp name and the extension; ffmpeg wants a codec
name, which is a third column or a small match. `vorbis` is `libvorbis` on
most builds and the native `vorbis` encoder on others, which the test fixtures
in `tag.rs` already work around by trying a list. Reuse that.

### 4. The re-download half (1 to 1.5 days)

`write_archive` writes an id for every track whose `path.is_file()`, and the
mp3 is on disk, so yt-dlp would skip exactly the tracks that need fetching.

Add a parameter rather than a status:

```rust
fn write_archive(tracks: &[Track], path: &Path, refetch: &HashSet<usize>) -> Result<usize>
```

skipping ids in the set. `tag_tracks` already takes
`wanted: Option<&HashSet<usize>>` for the same shape of problem. `ytdlp::run`
threads it through from its one call site.

**Do not add a `Status::Refetch`.** `counts()` is hand-kept and does not fail
to compile when a variant is missing, which is how `Skipped` came to be
counted in the header total and left out of the tally beside it.

Then one batched yt-dlp pass for all of them, since that is what yt-dlp is
good at, followed by the same steps 1 and 4-7 above. The old `.mp3` and the
new `.opus` differ in name, so both can exist while the swap happens.

### 5. Progress and the verb (half a day)

Two kinds of work at very different speeds under one counter.

A new `Status::Converting`, label `convert`. `STATUS_WIDTH` is 8 and
`converting` is 10, so the longer word is not available. Re-downloaded tracks
reuse `fetching`, which exists.

Adding the variant means touching, at minimum:

- `label()` and `color()`, both exhaustive matches that fail to compile
- `settled()`, also exhaustive
- **`counts()`, which is not** — this is the one that bites

The header word and the row word must be the same verb. One activity, one
word, is a rule this codebase has already paid for once.

## What it must not do

**Never delete before the replacement is verified.** Download or convert
first, check the output, then remove. A video pulled from YouTube since the
first download cannot be re-fetched, and deleting first loses it for good.

**Never silently downgrade.** An `mp3` whose video has gone has no good route:
converting it would produce a worse file to keep a count tidy. Leave it and
say so. `left 1 (video no longer on YouTube)` in the summary.

**Save the manifest per track, not at the end.** The sidecar is a small text
file and 300 rewrites cost nothing next to 300 transcodes. It means a
cancelled run leaves every track either fully swapped or fully untouched.

**Leave `Gone` tracks alone.** Their index is synthetic and they hold no
playlist position. `tag_tracks` already skips them for the same reason.

## Tests

Break each one before trusting it. Three tests in this repo have passed
against the bug they described.

- Tags, album, year and cover survive a container change. Drive it off
  `FORMATS` like the existing container test, so a format added to the table
  without a working conversion fails here rather than in someone's folder.
- A failed ffmpeg leaves the old file, the manifest entry and the row exactly
  as they were.
- A track already in the target format is skipped, and nothing is rewritten.
- The refetch set is what the archive honours: a stub yt-dlp, the argument
  read back, the way `the_scan_predicts_filenames_in_the_chosen_format` does.
  Asserting on the helper would leave the call site free to pass anything.
- A cancelled run leaves no half-converted folder: every track resolves one
  way or the other.
- An mp3 whose download fails keeps its mp3 and is reported, not converted.

## Effort

| Stage | Estimate |
| --- | --- |
| 1. Report the mismatch | half a day |
| 2. `tag::read_cover` | an hour |
| 3. Convert half | 1.5 to 2 days |
| 4. Re-download half | 1 to 1.5 days |
| 5. Progress and the status verb | half a day |
| **Total** | **about 3 days** |

Stage 1 is independently useful and ships on its own. Stages 2 and 3 are the
feature for four of the six formats. Stage 4 is what makes it one coherent
command rather than one with an asterisk, which is the whole point.

## What this buys over re-downloading everything

- No network for the common case. A 300-track playlist is roughly 1.5GB saved.
- It works offline.
- It fixes tracks whose YouTube videos have been deleted, which re-downloading
  can never do.

And what it costs: both copies of a track exist briefly, so the folder needs
headroom for its largest file, not for a second copy of itself.

## One thing to say out loud in the UI

YouTube serves Opus or AAC. Bringing a playlist to flac gives a lossless
container around audio that was already lossy: no fidelity is recovered and
the folder gets five to ten times larger. The feature earns its place for
players that cannot read Opus, which is a real problem people have. It is not
a quality upgrade, and the menu should not let anyone believe it is.
