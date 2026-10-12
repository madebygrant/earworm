# Wave plan: fixes from the two-day review

Written 2026-10-07 against `feat/create-new-playlists`. The source is the code
review of the uncommitted export and m4a work plus commits 5697c2c to 4fea247.
Line numbers are hints, so re-check them before acting. Nothing from the review
had been fixed when this was written. Item numbers refer to that review.

## 1. Export and format work: stop deleting or overwriting the wrong files

**Why first:** this is the only code that can delete your files, and it's
still uncommitted. Fixing it before the commit means the bad version never
enters history. The m4a default also lands on the next download.

Cheap, isolated items first:

- **Codec check on skip.** A re-encoded file is skipped on time alone, and
  alac and m4a share `.m4a`. Also require `worker::format_on_disk(to)` to
  equal the target (`export.rs` `already_there`, item 3).
- **Keep alac to m4a as a convert, not a refetch.** With native AAC, yt-dlp's
  output can land on the old file's own path (`worker.rs` `bring`, item 4).
  - Risk: this changes conversion behaviour approved on 2026-10-07.
- **Prompt text.** The destination note says "nothing is deleted there", and
  the format note still names opus. Fix both (nit).
- **Destination containing `--dir`.** `check` refuses a destination inside
  `--dir` but not one that contains it (nit).
- **Free space after conversion.** Estimate size from the target format, not
  the source size (nit).
- **Doc comment placement.** Move `NATIVE_M4A` above `run_with`'s `///` doc
  (`ytdlp.rs:318`, nit).

Then the design work:

- **Mirror name comparison (blocker).** Compare names lowercased and composed
  in `leftovers`, using the same key `portable::unique` uses. Add a test with a
  case-only rename and one with a composed-then-decomposed export.
- **Device-side ownership record.** Write a hidden file per home listing the
  files earworm wrote there. Only those may be offered for removal, or
  overwritten without asking. This covers an unrelated `Music/` on the stick,
  a list sharing a name with a folder, and the folder-layout list writing into
  a folder's home (item 1).
  - Risk: it's a new file on the device, so CLAUDE.md needs a rule for it.
- **Unique names in the folder layout.** Seed each home's `unique` set from
  every file the folder export would use, not only the list's files (item 2).
- **Decision: the m4a default for existing users (item 5).** Options:
  - Write `format = "opus"` into an existing config once.
  - Show a one-time notice.
  - Make m4a the default only when no config exists.
  - Risk: each of these changes what an existing install does.

Exit:

- The tests above fail before each fix and pass after, and `cargo test` is
  green.
- An export to a scratch folder twice, with a case-only rename in between,
  keeps every file.
- The work is then ready to commit.

Status 2026-10-07: done, except the m4a default for existing users, which
was decided as nothing: there is one user and no migration is needed. The
ownership record is `.earworm-export` (`export::RECORD`), a `key` that is
composed and lowercased, `folder_names` shared with the folder layout, and the
destination refusing a folder that holds the library. The alac-to-m4a rule is
in `bring`. Free space now estimates a lossless target at six times a lossy
source. Mutation-checked: the case-only test fails on a byte-for-byte compare,
and the `bring` test fails without the alac arm. Not run against a real device.

## 2. One path that carries every tag across a file swap

**Why here:** three later problems share one cause: lyrics lost on convert,
ISRC lost on refetch, and the same gaps in export's `reencode`. The cause is
that `swap_in` copies only four fields (`worker.rs:1915`). Building the helper
once makes those fixes trivial, and later conversion work is safe to check.

- **`tag::carry(from, to)`.** It carries artist, title, album, year, the
  cover, lyrics (`Extra::Lyrics`) and the ISRC. `swap_in` and
  `export::reencode` both call it (item 8, export nit).
- **Stale `#isrc` line.** When the replacement holds no ISRC after the swap,
  call `drop_isrc` so the sidecar line goes too (item 8).

Exit: a test converts a file holding lyrics and an ISRC, through both the
transcode path and the refetch path, and reads back both tags and the sidecar
line.

Status 2026-10-07: done. `tag::Kept` (read, then write) carries the identity
fields, lyrics, ISRC and picture; `swap_in` and `export::reencode` both use it,
and `swap_in` drops the `#isrc` line if the replacement still has none. Loudness
is deliberately not carried. Both tests fail with the lyrics and ISRC writes
taken out. The sidecar-drop branch has no test of its own.

## 3. Small library-integrity fixes

**Why here:** each is a few lines with no design question. Two of them (15 and
16) make the playlist store safe to write automatically, which wave 4 depends
on.

- **Copy recorded straight away.** Call `save_manifest` immediately after
  `copy_known`, so a run you quit doesn't orphan the copy and download a
  duplicate (item 6).
  - The test `a_name_already_taken_is_left_alone_and_the_track_downloads`
    pins the old outcome and needs revisiting.
- **Single-video guard.** Add `tracks.len() == 1` to the held-chapters branch
  (`worker.rs:1066`, item 14).
- **Album gain in `retry`.** Call `album_loudness` in `retry` after tagging,
  as `pipeline` does (`worker.rs:3271`, item 10).
- **Playlist name as album.** Don't send an album that came from the playlist
  fill to LRCLIB's exact lookup. When the exact lookup fails with a non-404
  error, fall through to search (`lookup.rs:716-740`, item 9).
- **ISRC marker clearing.** `flag_isrc_twins` should clear a title marker only
  when the other track's ISRC actually differs (`worker.rs:1740`, item 7).
- **cliamp key from the filename.** Build the key from the store's filename
  stem, not the raw list name, which closes the `../Focus` takeover
  (`playlists.rs:91`, item 15).
  - Risk: this changes the stored cliamp names for existing lists, which then
    need re-importing.
- **`save` refuses an unparseable file.** Refuse a file at the target path
  that doesn't parse, as `delete` already handles (`playlists.rs:207`, item
  16).

Exit: each item has a test that fails against the old code, and `cargo test`
is green.

Status 2026-10-07: done. Each fix has a test that fails against the old code
(mutation-checked), except the album gain in `retry`, whose call needs a live
download. Choices made: the ISRC guess is dropped only when the other side's
ISRC is known and different, and a `copy from` row is never dropped; an error on
LRCLIB's exact lookup falls through to the search unless it was the third
strike; the album is blanked when it equals the playlist name rather than
tracked separately, so a real album with the playlist's name costs one extra
request. Old cliamp names for lists with path characters or colons change, so
play such a list once and re-import.

## 4. Custom playlists keep resolving the right file

**Why here:** refreshing hints means writing lists without being asked, which
is only safe once wave 3 stops `save` from overwriting a broken file.

- **Cheap nits:**
  - `rehome` carries on past a list it can't save.
  - The tab-in-filename add isn't counted.
  - `on_air` follows a rename and clears on delete.
  - The `Synced` and `Missing` sorts treat lists as never needing a sync.
- **Library cursor by path.** After a refresh, keep the cursor on the same row
  by its path, as `Msg::Custom` already does by id (`app.rs:1386`, item 19).
- **Hints refreshed from the resolved path** when a list is opened or played,
  and on promotion from a `~` id to a video id. This covers hidden folders and
  an adopted folder that later gets a URL (item 17).
- **Repeated `~` ids in one folder.** Decide how to handle them (item 18). One
  option is for `id_for` to make a fresh id when that id already maps to a
  different file.
  - Risk: this is the same issue as the `contents()` id rule, so the decision
    touches adoption too.

Exit:

- A test renames a track in a hidden folder that a list holds, and the list
  still resolves it.
- Another renames list Zeta to Alpha, and the cursor stays on Alpha.

Status 2026-10-07: done. The `~` id decision was made as: a taken id gets a
number (`~Song.opus (2)`), so two files never share one. `contents()` is left
alone. Hint refresh also migrates the id, since a hint alone cannot follow a
file that is renamed again. All six changes were mutation-checked. Lists are
refreshed on open and on play, not on every keypress.

## 5. Chapter splits survive interruptions and upstream edits

**Why late:** every item needs a design decision and touches the sidecar
format. The paths are also rare: a kill in a narrow window, or a creator
editing chapters.

- **Test the `Failed` half** of the full-file guard on its own (nit).
- **Sweep scratch files** on a split folder's next sync, not only inside
  `fetch_split` (nit).
- **Record the "split" answer before the first cut,** so quitting early
  doesn't silently keep the whole video. Then correct CLAUDE.md:901-904 (item
  11).
  - Risk: this changes the sidecar.
- **Detect changed chapters.** Store each chapter's start time or title beside
  its id and compare on sync, rather than trusting position (item 12).
  - Risk: this changes the sidecar.
- **Decision: unpicked chapters (item 13).** Either delete the full file once
  every picked chapter exists, or keep it but show it on screen and stop
  asking about the same chapter.

Exit: tests cover a kill before the first cut, a chapter inserted at position
2, and an unpicked chapter, each ending in the decided state.

Status 2026-10-07: done. Decisions made: the split answer is a `#split`
header; chapters are matched by recorded span (`#chapter` headers) with the
old files left alone; and an unpicked chapter no longer keeps the full video.
Not done: rebuilding failed chapters from the sidecar once upstream loses its
chapters, since the spans are now recorded but nothing reads them for that.
Each change was mutation-checked.

## 6. Leftover nits and making the docs true

**Why last:** these change nothing anyone has hit, and the docs should
describe the state waves 1 to 5 leave behind.

- **Duplicate tracks:**
  - `elsewhere` keeps every holder of an ISRC, not just the first.
  - The ISRC same-song check composes titles first (`tag.rs:932`).
  - An unpicked row loses its `≈ copy from` marker.
- **Loudness:**
  - A gain earworm can't parse is re-measured.
  - The album peak counts only tracks that have one.
  - Decide whether another tool's album gain is kept.
- **ffmpeg tests:** fail or report a skip when no encoder is present, instead
  of passing vacuously.
- **CLAUDE.md corrections:**
  - The `--resync` "square of the library" claim.
  - The folder-layout safety claim.
  - The chapter "asks again" claim, if wave 5 slips.
  - New rules for the device ownership record and `tag::carry`, each naming
    its test.
- **Not actioned:** the single-video template fix riding in the lyrics commit.
  It's history and needs no change.

Exit: every CLAUDE.md rule touched names its test or says plainly that nothing
tests it.

Status 2026-10-07: done. Every code change was mutation-checked. Decisions made:
another tool's album gain is still overwritten (the alternative is a stale
one), and the vacuous ffmpeg skips are caught by one test that fails when an
encoder is missing and not by editing the 57 skip sites. `--resync` is still
quadratic over the library's sidecars, and CLAUDE.md now says so instead of
claiming it was fixed. The "asks again" claim and the folder-layout claim were
corrected in waves 1 and 5.

## Why this order

Wave 1 comes first because it holds the only code that can delete your files,
and doing it before the commit keeps the bad version out of history. Wave 2 is
the guardrail: one tag-carrying path turns three separate losses into one fix,
and later conversion work becomes testable. Wave 3 is small and quick, and it
includes the two store fixes that make wave 4's automatic list writes safe.
Chapters wait because each item needs a decision and touches the sidecar.

## What can slip

Waves 1 and 2 are non-negotiable: one deletes files, and the other loses tags
you typed. Wave 3 should follow soon, because the copy orphan and the
`../Focus` takeover can happen in ordinary use. Waves 4 and 5 can wait without
much cost, since their failures need unusual sequences. Wave 6 can slip
indefinitely.
