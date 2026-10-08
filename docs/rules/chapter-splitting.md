# Splitting a video by its chapters


- **The choice is asked, never assumed, and only with a gate.** A video with
  two or more chapters is not an album: `Me at the zoo` has three in nineteen
  seconds, and tutorials and podcasts carry timestamps. `settle_split` asks
  only when `pass.gate` is on, the video's file is not already there, and the
  folder holds no chapters of it. `--resync` and `--no-pick` pass no gate, so
  the unattended answer is the video as it was, the same reasoning as the
  `false` literal `resync` passes.
- **The choice is a `#split` header, written before the first cut, and the
  `vid#NN` entries count too.** The entries alone were the record, which left a
  window: stopped after the download and before the first cut was recorded,
  the folder held the whole video and nothing said it was to be split, so the
  next sync read it as a video that was never split and kept it as one track
  for good, where this section used to claim it asked again
  (`the_choice_to_split_is_recorded_before_anything_is_cut`). The header is
  carried by every writer through `Headers::of`. `manifest::is_split` reads
  both. A folder that holds the whole file is never asked again, which is also
  how a declined split stays declined.
- **A chapter is matched to its file by the span it was cut from, not by its
  position.** An id is `{video}#{nn}`, so a chapter the creator inserts moved
  every later position onto the wrong file: the video was fetched again, cut at
  the new last position, and the full file then deleted. `#chapter` headers
  record each cut's start and end (`set_chapter`, after each cut), and
  `chapter_tracks` gives a chapter the file whose span is within a second of
  its own, whatever id that file was cut under; a position whose recorded span
  no longer matches is cut again and the old file is left alone, since deleting
  it would be deleting a track somebody may have edited. A folder split before
  the spans existed has none and keeps trusting the position, and records the
  current ones from the first sync after. A listing that shrinks leaves the
  dropped chapters' entries and files where they are
  (`a_chapter_inserted_upstream_takes_the_file_its_span_matches_and_cuts_only_the_new_one`,
  `a_position_whose_span_moved_is_cut_again_and_the_old_file_is_left_alone`).
- **A chapter id is `{video}#{nn}`, and `is_chapter` checks the whole shape.**
  A bare `#` matched `~Song #2.mp3`, a local id, and one such file made
  `retry` skip its download for every track in a synced folder. It is guarded
  like a `~` id. No listing names it, so `note_departures` skips it: a listing that holds the
  video would otherwise call every chapter departed, prove it, and offer the
  album to `D`. It is not guarded in `write_archive` because chapter tracks
  never reach it: yt-dlp is only ever given the one video.
- **yt-dlp is not used to split.** `--split-chapters` keeps the whole file, and
  no hook reports the chapter files (docs/history.md has the four stages
  tried), so earworm would be predicting names yt-dlp has already sanitised.
  It cuts with ffmpeg itself, and names the files.
- **A cut is a copy except for flac and alac, and the length is checked.** A
  flac with no seek table ignores the range under a copy and writes the whole
  file, exiting 0. `tag::cut` re-encodes the lossless formats and compares the
  result's length with the chapter's, so that case is an error and not a
  full-length track. `a_chapter_cut_is_the_right_length_in_every_container`
  fails if the re-encode is taken out.
- **A cut is written under `SCRATCH` and renamed into place, and the manifest
  entry is written after each chapter.** A kill in the middle of ffmpeg leaves
  a hidden scratch file that `sweep_scratch` clears at the start of the next
  cut, and nothing at the name a later sync reads as a finished chapter. The
  entry is the only record the folder was split: written once at the end, a
  run stopped half way left the full file and some cut files with no entries,
  so the next sync read the video as whole and the chapters as somebody's own
  music. What is still not covered is a chapter cut and recorded but stopped
  before the lookup: it reads as `Have` and is never identified, which is what
  any downloaded track in that state does.
- **The full file goes last, and only if no chapter failed.** `fetch_split`
  removes it once every track has a file of its own that is not the video's
  (`whole_is_done`). A failed chapter can be recorded against the video's own
  path, which is on disk and is not its cut, so the test is on `Failed` as well
  as on the file. A chapter the pick gate left out is not a reason to keep it:
  kept, the whole video sat in the folder for good, in no sidecar and no
  playlist, and every gated sync asked about that chapter again. It downloads
  again if it is ever wanted, as an unpicked track does anywhere
  (`the_full_file_stays_while_a_chapter_failed_and_goes_when_the_rest_are_done_or_unpicked`).
  A chapter whose name would equal the video's gets `(chapter)` added in
  `chapter_tracks`, because the cut reads the file it would write over.
- **A split folder stays split when the chapters vanish upstream.**
  `settle_split` reads `manifest::is_split` before it looks at the listing's
  chapters: with none, it rebuilds the tracks from the sidecar's `vid#nn`
  entries (`held_chapters`), so the video is not fetched again beside the cuts
  and the `.m3u8` keeps every chapter. A folder never split with no chapters is
  the plain video. Only when the listing is that one video: a playlist whose
  first track happens to be a split video's id keeps its own tracks
  (`held_chapters_replace_a_single_video_and_never_a_playlist`).
- **A cut carries nothing of the video but audio and its picture.** `-map_metadata
  -1` leaves the chapter list, so `tag::cut` also drops chapters, and
  `fetch_split` copies the full file's picture onto each cut: it is the fallback
  art for a chapter the lookup cannot place. A cut that fails keeps its reason:
  `tag_tracks` leaves a `Failed` track with no file alone, or "no file" replaces
  `cut: ...`, and the cut error also goes to the log.
- **The kill window is closed by the header.** The one still open is stopped
  after the download and before `set_split` runs, which is a few microseconds
  after the file lands; the next sync then sees a whole video it asks about
  again. `settle_split` also sweeps a killed cut's scratch file whenever the
  folder is split, not only when something is cut.
- **A split promotes the kind only when nothing has answered.** A `[playlist]`
  name or an earlier header is somebody's choice, and the name wins on
  reopening anyway, so overriding it made the run and the library disagree.
- **The video is one row while it downloads.** yt-dlp reports progress and
  `@D` by the video's index, so on chapter rows it would mark chapter 1 as the
  whole download and hand it the full-length file's path. `fetch_split` sends
  the video, then the chapters once they exist.
- **`retry` does not download for a split video.** Its tracks have no download
  of their own, and running yt-dlp over them would fetch the whole video again
  and give its file to track 1. A sync cuts anything missing, from the full
  file that was kept for it.
- **`convert` does nothing for chapters.** `plans` is empty when split: a
  chapter has no video of its own to refetch, and a cut in the old format is
  not converted by yt-dlp.
- **Four things in `pipeline` carry this and nothing tests them.** The call
  to `settle_split`, `album` and `kind` being promoted when it returns
  `Some`, the empty `plans`, and `fetch_split` standing in for `ytdlp::run`.
  Reaching them needs a live scan, as with the three lines in the albums
  section. What is tested is one level down, and the whole path was run once
  by hand against a real six-chapter video: five tracks of the right lengths,
  the full file gone, and the folder read back as split.
- **The scan's `--print` carries the chapters, and the stubs agree.** One
  more field before the filename: `splitn(7)`. All three stub yt-dlps print
  it as `NA`, so a stub still printing six fields breaks where the format is.
