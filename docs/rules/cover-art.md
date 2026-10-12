# Cover art


- **`--embed-thumbnail` belongs with the other cover arguments.** It sat in
  the unconditional block, so `--no-cover` embedded YouTube's thumbnail in
  every track while its own help promised nothing was embedded. It stays on
  by default because it is what carries art for a track the lookup could not
  place, which on a folder of Art Tracks is all of them.
- **The embedded cover is capped at `COVER_MAX`, which is Deezer's
  `cover_xl`.** The number is "no larger than the best source earworm would
  pick for itself" rather than a taste. YouTube serves an Art Track's
  thumbnail at 2048 square: on a real folder that was 2.9MB of art on 3.5MB of
  audio, the same image in all twelve files. `cap_cover` measures with
  `jpeg_size` and returns early when it is already small, so it costs one tag
  read per new track and nothing on a later pass. A picture that is not a
  JPEG cannot be measured and is left alone, which is right: the only ones
  that are not come from the `c` picker, where somebody chose the file.
- **`shrink` goes through files, not pipes.** `run_bounded` polls `try_wait`
  and never writes to the child, so a piped stdin nobody closes leaves ffmpeg
  waiting for input that is not coming. It also checks the output is a JPEG
  before writing it back, or a truncated resize replaces good art with
  something no player will show.
- **A pass deletes only the folder image it wrote.** Every track carries the
  art embedded, so `cover.jpg` is a leftover once the pass finishes, but one
  the user dropped in by hand or chose with `c` is not: `folder_covers` reads
  the folder before the download and `drop_folder_cover` keeps what it saw.
  Deleting somebody's file does not come back and nothing on screen would say
  it happened.
- **`c` cannot be undone from disk, so the restore row refetches.** Choosing a
  cover embeds it over every track's own art rather than beside it, so there
  is nothing left on disk to put back and `restore_art` looks each track up
  again. It takes the first candidate that downloads, in the order
  `cover_options` lists them, because an automatic pick disagreeing with the
  top row of that same menu would be a second opinion about which art is best;
  `find_map` and not the first URL, so a release group with no front image
  falls through to Deezer instead of costing the track its sleeve. The row is
  offered with no folder image there too, since the embed is the half worth
  undoing, and the label drops the removal clause rather than promising to
  delete a file that does not exist. Nothing caps what it fetches: `COVER_MAX`
  is Deezer's `cover_xl`, which is the largest thing that list returns. Its
  targets are filtered on `is_file` and not on a recorded path, or a `Gone`
  track costs a lookup and a failed write and then lands in the same count as
  a track whose art could not be found.
- **The restore asks before it rewrites, where the rest of that menu does
  not.** Every other row embeds a picture the user has just looked at; this
  one embeds one per track that nobody has seen, chosen by whichever candidate
  downloads first, and `u` holds tag values rather than art, so there is no
  way back. The confirmation is `purge`'s shape for the same reason. Its
  refusal reads "change nothing" rather than naming the art, because the row
  has two halves and abandoning it abandons both: the early return sits above
  the removal, so a refusal leaves the folder image where it was. The question
  is skipped when no track has a file to rewrite, since nothing is then at
  risk and the row does exactly what its label says.
- **Every artwork download is paced, and the pacing is on `get_bytes`.** The
  metadata queries had limiters and the image that followed had none, which
  cost nothing while every caller fetched one image per keypress. `tag_tracks`
  fetches one per track and so does `restore_art`, so a folder of forty
  arrived as forty back-to-back requests, and the Cover Art Archive URL
  redirects to archive.org. It sits on `get_bytes` because every caller of
  that function is downloading artwork, which makes it one decision rather
  than one per call site.
- **`Msg::Artwork` is the only thing that can tell the detail pane its cover is
  stale.** The pane decodes a cover once and holds it against the path it came
  from, and both `cover` and `restore_art` change the bytes behind a path that
  does not move, so no other message on that list means anything has happened.
  Without it the pane goes on drawing the sleeve that was just replaced, which
  makes the command look as though it did nothing at all.
- **A folder image that was there before the download is the user's, and
  `written` is what protects it.** `apply` writes `cover.jpg` from the first
  track to resolve art, so on a folder somebody had already put art in, the
  bytes were gone before `drop_folder_cover` ever ran: that function keeps the
  *name* from deletion and nothing kept the contents. `pipeline` built
  `written: false` where `retry` built it from `has_cover`, and two answers to
  one question is an oversight, not a decision. It is `!theirs.is_empty()` now, the
  same answer `folder_covers` already took before the download, so there is
  one derivation rather than two.
