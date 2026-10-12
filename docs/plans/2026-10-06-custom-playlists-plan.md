# Wave plan: custom playlists and export

Written 2026-10-06 against `feat/loudness-tags`, with uncommitted changes.
Treat line numbers as hints and re-check them before acting.

The request: create custom playlists from tracks already in earworm, and
export the files and custom playlists to other devices.

**Already true, or found while planning:**

- Every library row is a folder (`Shelf.path`). There is no playlist without
  a folder of its own.
- `.m3u8` entries are bare filenames, decomposed for iOS (`src/worker.rs:4114`,
  CLAUDE.md). A playlist that spans folders can't follow that rule.
- `View::Found` (`t`) already searches tracks across the whole library, with a
  cursor stored as a (folder, file) pair. That is the natural picker.
- `App.marked` holds track indices within one open folder, so it can't hold
  picks from several folders.
- `library()` drops a folder that has no audio and no URL
  (`src/worker.rs:379`), and `--resync` logs every folder with no URL. A
  playlist folder holding no audio would trip both.
- **Found while planning:** `clean()` only replaces `/ \ :` in filenames. FAT
  and exFAT also refuse `* ? " < > |`, so a filename that is valid on macOS can
  fail to copy to a USB stick.
- docs/history.md:17-19 already names a per-target composed/decomposed choice
  as the point "where this becomes a setting". Export is that point.
- The duplicates plan's `library_index` (id to file across `--dir`,
  `2026-10-06-duplicate-tracks-plan.md` wave 2) is the lookup a custom
  playlist needs. This plan depends on it.

## 1. Storage model and device facts

Why first: the storage format is the expensive thing to get wrong, as in the
duplicates plan, and every later wave reads it.

- **Decision: what an entry points at.** Recommended: the video id, resolved
  through the library-wide index.
  - A folder plus filename breaks on the first edit (`write_track` renames the
    file) and on `rename_shelf`.
  - A `~` id is built from the filename, so it breaks on a rename too. Store
    the last known relative path beside it as a fallback.
- **Decision: where custom playlists live.** Recommended: a separate store
  under `--dir`, for example `.playlists/<name>`, never mixed with download
  folders. Otherwise every folder rule would have to learn an exception:
  resync, `D`, kind detection, adoption and the URL checks.
- **Decision: the in-library `.m3u8`.** Recommended: entries like
  `../Folder/file`, for playback inside the library only. Export (wave 4)
  rewrites them.
  - Risk: this breaks the filenames-only rule for this one kind of file, so
    CLAUDE.md has to say why.
- **Decision: editing tags from a custom playlist.** A tag write has to go
  through `write_track` on the source folder, so the file, the folder's
  sidecar and the folder's `.m3u8` change together. Recommended for v1: no tag
  edits from a custom playlist. Enter jumps to the source folder, which
  `Cmd::Open` with a filename already does.
  - Risk: users may expect to edit in place. Say so on screen.
- **Investigation: `../` entries in players.** Does cliamp `playlist import`
  accept them? Do the players on the target devices?
- **Investigation: devices.** A USB stick or SD card mounts under `/Volumes`.
  An iPhone (Finder file sharing) and an Android phone (MTP) do not mount on
  macOS. Recommended: export means "copy to a directory". Confirm which
  devices matter.
- **Investigation: AppleDouble files.** Check whether `std::fs::copy` onto
  exFAT writes `._` files, since copying with xattrs creates them. If it does,
  export copies data only.

Exit: the four decisions recorded here, and the three investigations answered
in docs/history.md.

Status 2026-10-07: done. Decided:

- Entries are video ids resolved through the library, with the last known
  relative path as a hint. A `~` id is made once and survives a rename, but two
  folders can hold the same one, so it only resolves in its own folder.
- The store is `<dir>/.playlists/<name>.playlist`, with `#` headers. The
  in-library `.m3u8` uses `../Folder/file`; cliamp imports that (history.md).
- v1 has no tag edits from a custom playlist. Enter jumps to the source folder.
- Export means "copy to a directory". USB sticks, SD cards and synced folders
  are the same code. An iPhone or Android phone does not mount, so for those v1
  produces a folder to move across by other means.
- Custom playlist layout on the device is a per-export choice: a self-contained
  folder of copies, or `../` references.
- The investigations changed one premise: macOS does not fail on the FAT
  characters, it writes them. The portable-name mapping is still needed, for
  the other direction (history.md). Copying onto exFAT also leaves `._` files
  that a data-only copy does not avoid, so export removes the one it caused
  per file.

## 2. Shared groundwork

Why here: both building a playlist and exporting one need to resolve ids to
files across the library and to produce filenames that are safe on any
filesystem.

- Generalise `library_index` from the duplicates plan, which only answers for
  wanted ids in one format, into a full id-to-file lookup.
  - Risk: land the duplicates plan's wave 2 first, or build it here and have
    that plan reuse it.
- Write a portable-filename function for export only, covering the FAT and
  exFAT characters and trailing dots and spaces.
  - Risk: leave `clean()` alone. Changing it renames library files on their
    next edit.
- Write the custom playlist store's reader and writer. Use `#` headers so
  older readers skip unknown lines, following the "`#` headers must stay
  invisible" rule. Add round-trip tests.

Exit: unit tests for the index, the filename mapping and a store round-trip.

Status 2026-10-07: done, in `src/worker.rs` (`resolve_entries`, which reuses
`Others`), `src/portable.rs` and `src/playlists.rs`. Nothing calls them yet, so
the build warns about dead code until wave 3.

Review findings fixed: a hint that is absolute or steps out of `--dir` is
dropped on read and write and never resolved; a name that would not round-trip
(edge spaces) or is too long is refused; a suffix from `unique` cannot push a
name past 255 bytes; an unparseable `.playlist` is reported by `broken` and can
be deleted; renaming a folder rewrites the hints of the custom playlists that
name it (`rehome`).

Still open for wave 3:

- A file in a folder earworm never edited has no sidecar entry, so a `~` id for
  it resolves only through its hint and breaks on the first rename. Adding such
  a track has to write the sidecar entry first.
- The `._` finding in history.md still needs measuring from a plain terminal.
- `Others` skips every dot-prefixed folder while `library()` still lists them,
  which also changed which folders `copy_known` may copy from.

## 3. Building custom playlists

Why here: it needs the storage model and the index.

- Create, rename and delete a custom playlist from the library.
  - Risk: `D` on a custom playlist deletes the list and never audio. Its menu
    must not offer the folder-delete row.
- Add tracks from `t` search results and from marks on a folder's track list,
  through a chooser. Commands carry ids, never row positions.
- A custom playlist screen with reorder, remove and jump-to-source. A missing
  entry shows `!`, like the preview. Its key predicate must be exclusive with
  `can_command`, `can_browse` and `can_find`.
- A library row with its own kind. If the icons plan's wave 3 has landed, it
  gets its own icon.
- `--resync` skips custom playlists without logging "no URL".
- Generate the `.m3u8` so `p` and space play it through cliamp.

Exit:

- A test that builds a playlist from two folders, edits a source track (which
  renames its file), and checks the playlist still resolves every entry.
- A test that deletes the playlist with `D` and checks no audio is gone.

Status 2026-10-07: built, in `src/custom.rs`, `src/playlists.rs`, the
`Kind::Custom` row, `View::Playlist` and the keys `N` (new), `P` (add), `J K x`
and Enter on the list screen. The two exit tests pass, and a third checks that
no folder command will act on a custom row's path. Not driven through a pty:
the harness lost keys here, so the screens were checked live once and the keys
through unit tests. `play` is untested because it reaches the real cliamp store.

## 4. Export

Why here: it exports what wave 3 builds, and uses wave 2's filename mapping.

- Choose a destination with a path prompt that also lists the mounts under
  `/Volumes`. Export selected library folders and custom playlists.
- **Decision: custom playlist layout on the device.** Recommended: a
  self-contained folder of copies, so every player can play it. The
  alternative is `../` references, which car stereos and simple players often
  don't follow. The cost is duplicate audio on the device.
- **Composed or decomposed names, per export.** This is the setting
  docs/history.md anticipated.
- **Incremental.**
  - Skip files whose size and modification time already match.
  - Copy through a scratch name and rename into place, as `copy_known` does.
  - Check free space before starting.
  - Report progress, and check for cancel between files.
- **No deleting on the device in v1.**

Exit:

- A test that exports two folders and one custom playlist to a temp dir.
- A second export copies nothing.
- A track named with `?` lands under a safe name, and the `.m3u8` names that
  safe name.

Status 2026-10-07: built, in `src/export.rs` with `E` on the library. Folders
and custom playlists, composed or decomposed per export, both playlist layouts
(`Folders` points from a root `.m3u8` rather than with `../`, which needs no
player support), incremental copy, free-space check through `df`, and the
`._` cleanup. Exit tests pass. Not tested: the free-space check and the
prompts. Not driven through a pty, and not run against a real device.

## 5. Mirroring and format on export

Why late: it deletes things and it multiplies the work. Both belong after
plain export has been trusted for a while.

- Mirror mode removes device files that are no longer in the playlist. Use
  `purge`'s rules: name every file in the confirmation and only touch paths
  under the export root.
  - Risk: deletion can't be undone.
- Transcode on export for devices that can't play opus, reusing `transcode`.

Exit: a mirror test that deletes only the files it named in its confirmation.

Status 2026-10-07: built in `src/export.rs`. Mirror lists and asks before
removing, and only files recorded as its own in a hidden `.earworm-export`
on the device (changed after the review of 2026-10-07); transcode reuses
`tag::transcode` and carries tags and cover by hand. Tests cover the mirror's
limits, the shared-folder layout and a conversion plus its skip on a second
export. Not run against a real device, and the prompts have no test.

## 6. Documentation

Why last: these docs describe how the earlier waves end up behaving.

- CLAUDE.md:
  - Why entries are ids and not filenames.
  - Why `../` is allowed in exactly one kind of `.m3u8`.
  - Why export maps filenames while `clean()` stays as it is.
  - Why `D` never touches audio on a custom playlist.
- README, a new docs/export.md, and docs/library.md.

Exit: every new CLAUDE.md rule names the test that pins it, or says plainly
that nothing tests it.

## Why this order

Getting storage wrong costs a migration, so it comes first. Wave 2 is shared
by both halves. Building comes before exporting because exporting needs
something to export. Deleting on the device comes last because it's the only
step that can't be undone.

## What can slip

Wave 5 can slip indefinitely: export without mirroring is still useful. Waves
1 to 4 are the core. Across plans, the icons plan's wave 3 should land before
wave 3 here, and this plan depends on the duplicates plan's wave 2 for the
library index.
