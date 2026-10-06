# Wave plan: duplicate tracks, AcoustID bypassed

Written 2026-10-06 against `feat/loudness-tags`, with uncommitted changes in
`src/lookup.rs` and `src/worker.rs`. Treat line numbers as hints and re-check
them before acting.

The five ideas this plan sequences:

1. A library-wide video id index, copying the existing file instead of
   downloading it again.
2. A same-title, same-duration warning at the pick gate.
3. An ISRC from Deezer, stored in the tag and the sidecar.
4. Preferring the Art Track over the music video for a detected pair.
5. A local Chromaprint comparison (`fpcalc -raw`) with a cache.

**Already done or already true:**

- Nothing has to be removed to bypass AcoustID. With `ACOUSTID_API_KEY`
  unset, `identify` never calls it (`src/tag.rs:709`), and no wave below
  reads it.
- Deezer's `/search` returns `isrc` (checked with a live request on
  2026-10-06), so idea 3 costs no extra requests.
- `fpcalc` 1.6.1 and `yt-dlp` are installed on the development machine, so
  the investigation steps can run there.

**Found while planning:** sidecar entries are `id\tfilename`, read with
`split_once('\t')` (`src/manifest.rs:290`). A third column on an entry line
would give older binaries a filename like `01 - A.opus\tGB01A…`. They would
see every file as missing and download the whole library again. So per-track
data has to go in `#`-prefixed lines, which old readers skip
(`src/manifest.rs:300`), or outside the sidecar. Wave 1 settles which.

## 1. Decisions and measurements

Why first: every later wave needs a storage format, a way to show a
duplicate, and numbers for its thresholds, and each of these is cheap now and
costly to change after it ships.

- **Investigation, Art Track vs video.** Check whether yt-dlp's flat listing
  exposes anything that tells an Art Track from a music video (try
  `%(media_type)s`, `%(channel_id)s`, `%(availability)s`). CLAUDE.md already
  rules out the uploader: Art Tracks report the bare artist name. Wave 5
  depends on this result.
- **Investigation, album page links.** Run one
  `music.youtube.com/browse/MPREb_…` link through a scan and confirm
  `playlist_id` comes back with the `OLAK5uy_` prefix. Not one of the five
  ideas, but a plan about YouTube Music albums should settle it.
- **Investigation, fingerprint thresholds.** Run `fpcalc -raw -json` on three
  kinds of pair: the same Art Track downloaded twice, an Art Track against its
  music video, and two different songs of the same length. Record the
  bit-error rates and the offsets that worked in docs/history.md, which is
  where this repo keeps measurements.
- **Decision: how a duplicate is shown.** Recommended: a field on `Track`
  (for example, `twin: Option<…>`) drawn as a marked word or pill, not a new
  `Status`. A new status has to pass the `settled()` and `tally()` traps. It
  would also replace the status word, but the status is about tagging and a
  duplicate isn't a tagging outcome.
- **Decision: where per-track data lives.** ISRC goes in the audio tag
  (lofty supports ISRC) and in a `#isrc` sidecar line. The fingerprint cache
  goes either in `#print` sidecar lines or in a file under `XDG_CACHE_HOME`,
  keyed on path, size and mtime. Leaning towards the cache file: a
  fingerprint can be recomputed, and a sidecar that travels to a phone
  doesn't need to carry it.
- **Decision: a playlist that lists both copies.** Keep both and flag them,
  or skip one? Recommended: keep both, because the `.m3u8` follows the
  playlist and skipping one leaves a gap in it.

Exit: three measured pairs in docs/history.md, answers to both
investigations, and the three decisions written down.

## 2. Catching duplicates before the download

Why here: this needs only the wave 1 decisions and data earworm already has,
and it's the only wave that prevents a download rather than reporting one.

- **Library-wide video id index (idea 1).** Build an id-to-file map from
  every sidecar under `--dir`, reusing what `worker::library`
  (`src/worker.rs:350`) already loads, once per run. For `--resync`, build it
  once for all folders, not once per folder. Leave `~` ids out;
  `manifest::is_local` already identifies them.
- **Copy on a hit.** For a `Pending` track whose id is in the index, copy the
  file to the path the scan predicted, mark the track `Have`, and let the
  existing `save_manifest` and `write_archive` handle the rest. Copy rather
  than hardlink, so editing a tag in one folder doesn't change the other.
  - Risk: this changes what a sync does. The copied file brings the other
    folder's tags and sleeve, and that's now what this folder holds. Log
    every copy. The `Have` branch of `tag_tracks` must not re-identify a
    copied file, or it overwrites the tags the user kept.
- **Same-song warning at the pick gate (idea 2).** Flag two ids in one
  listing with the same `normalised(strip_number(…))` title and durations
  within 2 seconds, using the rules `reconcile` already uses
  (`src/worker.rs:1159`). For files already in the library, match on title
  only: their durations need a tag read per file, and the draw loop must not
  do disk IO. Mark them as possible matches.
  - Risk: a live version and the studio version can share a title. This is
    a warning only, never a skip.

Exit: an integration test with a stub yt-dlp, two folders sharing an id, and
a run that copies instead of downloading (check the stub's argv: the shared
id is in the archive). A second test where two listing rows with the same
title show the flag at the pick gate. Break each fix and watch its test fail.

## 3. Identity that persists

Why here: this builds on the display from wave 2 and gives wave 4 something
cheaper to try before decoding audio.

- **ISRC from Deezer (idea 3).** Add `isrc: Option<String>` to
  `lookup::Match` (`src/lookup.rs:88`) and read it in `lookup::deezer`
  (`src/lookup.rs:300`). Carry it through `identify` and write it to the tag.
  The iTunes Search API probably has no ISRC, so Apple matches won't get
  one. Unverified; check during the build.
- **Record it in the sidecar** as a `#isrc` line, per the wave 1 decision,
  and add it to the wave 2 index so two different videos with the same ISRC
  are flagged after tagging.
  - Risk: a new sidecar header. Test that a sidecar written by the old
    binary still loads, and that the old reader ignores the new lines (the
    "`#` headers must stay invisible" rule).

Exit: a test that builds a `Match` with an ISRC by hand (the same route as
`an_albums_files_all_read_back_the_same_album_tag`) and reads it back from
the tag, plus a sidecar round-trip test with the new line next to an
old-format entry.

## 4. Local audio comparison

Why here: this covers only what waves 2 and 3 miss, and it's the most code,
so build it once you can see how many duplicates are actually left.

- **`fpcalc -raw` through `run_bounded`** with stderr discarded and stdin
  null, for the same reasons as `transcode`. Store results under the cache
  key from wave 1, so a resync costs a stat per file and no decoding.
- **Comparison:** minimum bit-error rate over a window of offsets, with
  thresholds from the wave 1 measurements, not chosen by feel. Keep it a pure
  function over two integer slices, so it can be tested without ffmpeg.
- **Scope:** only tracks that got no ISRC and no id match. Once per new file,
  never across the whole library on every sync.
  - Risk: decoding cost on a large first run. Make it opt-in through
    `opt_in`, like `loudness` and `lyrics`, for the same reason: it reads
    every file already on disk.

Exit: a test of the comparison function using pairs saved from the wave 1
measurements, and a test that a second pass reads the cache and starts no
`fpcalc` (count spawns with a stub, as the lyrics tests count requests).

## 5. Acting on a detected pair

Why here: preferring one copy of a song decides what lands on disk, so it
waits until detection has been in use and trusted. It also depends on the
wave 1 investigation.

- **Prefer the Art Track (idea 4).** If wave 1 found a field that tells the
  two apart: at the pick gate, pre-unmark the video copy when its Art Track
  pair is in the same listing. If no such field exists, drop this item and
  say so in the docs, rather than guessing from the channel.
  - Risk: this changes the default selection at the pick gate. `--resync`
    passes no gate (`gate: false`), so decide explicitly whether this
    applies there. Recommended: no, since the user asked for the sync, not
    for decisions about each playlist.
- **Removing a duplicate**, if wave 1 decided on anything beyond flagging.
  Use `purge`'s rules: name every file in the note, check every path is under
  `--dir`, and never act on an unproven match.
  - Risk: deletion can't be undone. It's the last item in the plan for that
    reason.

Exit: a pick gate test where a pair arrives with the video pre-unmarked, and
a `--resync` test showing nothing pre-unmarked.

## 6. Documenting it

Why last: these docs describe how the earlier waves end up behaving.

- One rule per trap in CLAUDE.md: the entry-line compatibility rule, why a
  copy and not a hardlink, why a duplicate is a field and not a `Status`, the
  fingerprint opt-in, and why `--resync` has no preference.
- README and docs/library.md: what the duplicate marker means, the
  fingerprint setting, and the ISRC tag.
- docs/history.md: the threshold measurements (if wave 1 didn't already add
  them) and the Art Track investigation result.

Exit: every new rule in CLAUDE.md names the test that pins it, or says
plainly that nothing tests it, as the `gate` literal entry does.

## Why this order

The sidecar finding is what fixes the sequence: three of the five ideas store
something per track, and getting that format wrong costs anyone who
downgrades a full re-download. So the storage decision comes before any code.
Wave 2 goes next because it's the only one that saves bandwidth and it needs
nothing new. ISRC (wave 3) comes before fingerprinting (wave 4) because it's
a lookup that's already being made, while fingerprinting is new code with
decoding cost. See what's left after wave 3 before paying for wave 4. The Art
Track preference comes last because it's the only item that changes what gets
downloaded by default.

## What can slip

Waves 1 and 2 are the core: they deliver most of the benefit and set the
design. Wave 3 is small and cheap, so it's worth doing. Wave 4 can slip
indefinitely: if waves 2 and 3 leave few duplicates, don't build it. Wave 5
depends entirely on what the investigation finds and may become just a note
in the docs. Wave 6 slips only as far as the waves it describes.
