# Folders earworm didn't download


- **A folder under `--dir` is a playlist if it holds audio, not only if it
  holds a sidecar.** `contents` is the one derivation, read by both `library`
  and `open_shelf`, and it drops a directory with neither audio nor a `#url`.
  The sidecar wins only for a folder that has a URL: there it carries the
  playlist's order and the files a sync found missing, and a file the user
  dropped in is the sync's business to place. Without a URL it is only a tag
  ledger, recording which files happened to be there the last time somebody
  edited a tag, so the directory is authoritative and the sidecar supplies ids
  for what it already knows. Preferring it outright meant a song copied into an
  adopted folder never appeared anywhere and nothing would ever pick it up,
  because no sync can run against a folder with no URL. `audio_files` also
  skips anything whose name starts with a dot: macOS writes `._01 - A.opus`
  beside every file on a FAT or exFAT volume, which carries the audio
  extension and is not audio. `Shelf.url` is therefore
  `Option`, and the three answers on the row are `synced 3d ago`, `never
  synced` and a `local` pill: collapsing the last two leaves somebody
  pressing `R` and wondering why one row never moves. The third is filled
  rather than dim because it is the row whose keys behave differently and it
  was the quietest thing on the screen, and it lives in the sync column rather
  than beside the name, where it would want a slot of its own and could land
  next to the album pill on a folder that is both.
- **`D`'s rows carry their answers, because the menu is a row shorter for a
  folder earworm did not download.** `Remove` is the enum and `rows` is built
  from it, so the third row does not mean "delete" on one folder and "hide" on
  another. `D` used to refuse such a folder outright, on the grounds that the
  only row left would be deleting somebody's music: what that actually did was
  leave no way to get the row off the library at all, since any folder of
  audio is on it. The answer is a menu without the row that cannot apply, not
  no menu. The delete row names its count in *tracks*, because that is what
  the number counts: `remove_dir_all` takes the folder, `cover.jpg` and the
  sidecar with it, so "6 files" was the one row on that menu understating
  itself. The closing flash reads `chosen` too, and did not: as `choice == 1`
  it said "forgot" after a hide on a found folder and "deleted" after a hide
  on a synced one, which is a claim that earworm destroyed a folder still
  entirely on disk. The test missed it by asserting `any()` over the flashes,
  and `any` cannot see a message that is there and wrong.
- **`#hidden` is how a folder leaves the library with its files intact.**
  `manifest::hide` writes it and nothing clears it: the way back is removing
  that *line*, and the flash says so, because a row that vanishes with no way
  back is a folder somebody has lost. The line and not the file, which is what
  it said first: a found folder's sidecar holds nothing else, but a folder
  earworm downloaded keeps its URL, its rename and every id in that same file,
  and throwing it away re-downloads the whole playlist beside the copies on
  disk. One message has to be true of both. It is in the sidecar rather than the config
  because it is a fact about that folder and moves with it, and because an
  array in the config would need a writer `save_key` does not have. `library`
  asks before the stat pass, since a hidden folder is one nothing draws.
  `worker::hidden` and the `--check` row are the other half of it: the only
  word a hidden folder ever gets is one flash at the moment it happens, and a
  `--resync` passing over four folders with nothing saying why is the album
  plan's "decide silently" arriving by another road. `--list` was left alone
  on purpose, because it is script-facing and a new line shape there breaks
  whatever is parsing it.
  Hiding deliberately does *not* call `player::forget`: the files are all
  still there and still in the order cliamp imported them, so a playlist that
  plays should go on playing. That line has no test, because `player::forget`
  takes no binary to stub and does nothing without cliamp installed.
- **`forget` no longer takes the row off the list.** It drops the sidecar and
  keeps the audio, and a folder of audio is on the list either way, so what it
  loses is its URL. The flash says so. What it is not is the way to take a row
  off the library: `hide` is, and the two are one keypress apart on the same
  menu, so the flash has to say which of them happened.
- **A `~` id is one earworm invented, and no listing will ever name it.**
  `manifest::local_id` builds it from the filename at adoption and
  `manifest::is_local` is the check. It sits on the id rather than in a
  header, because a folder can be half reconciled: eight files matched and two
  not, beside each other, where a header is one bit for the whole folder that
  would have to be cleared at exactly the right moment or left on for good.
  Two readers need the guard and one does not: `note_departures` walks the
  manifest and would call every copied-in file departed off a complete
  listing, proven, and offer the lot to `D`; `write_archive` hands ids to
  yt-dlp, which knows nothing of them. `ytdlp::scan` indexes the manifest *by*
  video id, so a `~` key is unreachable from there and a guard would be dead
  code. `a_local_id_is_never_matched_archived_or_called_departed` is the whole
  trap in one test, and the order matters: `note_departures` runs before the
  local row joins the list, or the plain "not in the current tracks" filter
  excludes it and the test proves nothing.
- **`open_shelf` skips `mark_departed` when there is no `#url`.** That
  function reads the `.m3u8` as earworm's own last answer about the playlist,
  and in a folder earworm did not download it is whatever file the user
  brought, listing whatever they chose. Believed, it calls everything the file
  leaves out `Gone`: never renamed, never re-tagged, dropped from the next
  playlist written.
- **Adoption is `save_manifest` running for the first time in a folder with no
  URL, and it is said once.** A hidden file appearing in somebody's music
  folder is not something they should find out by looking, and a line on every
  edit is nagging. The URL is what tells it apart from a fresh download, whose
  sidecar is no news.
- **`#m3u8` records the playlist file earworm wrote.** A synced folder's
  `.m3u8` is earworm's outright, so a sync rewrites it every pass. A folder
  somebody copied in may have brought one, and it may be named exactly what
  earworm would name its own, after the folder. Without the record the first
  edit either refuses for good and lets the playlist go stale, or replaces a
  file the user wrote with nothing saying so. `write_playlist` asks only when
  `cfg.url` is empty, and records the header at the moment of writing.
- **`write_manifest` records `cfg.folder` as `#name`; `set_name` is the only
  other writer, and it is `e`.** Attaching a URL pins the folder so the download lands there rather than under
  the playlist's upstream title, and the header is what makes the next sync
  land there too. Written by whichever manifest write is first, so it arrives
  with the `#url` from the run that earned both. `sync_open` deliberately
  writes nothing itself: a second step afterwards is one a failed sync has to
  remember not to take, and a sidecar appearing in a folder earworm did not
  download after a typo'd URL is the one case the adoption flash cannot cover.
  Never over a name already there, which is one somebody chose with `e`.
- **A sync takes the `.m3u8` over; adoption does not.** `write_playlist`'s
  `#m3u8` guard is asked only while `cfg.url` is empty, so the first sync after
  a URL is attached replaces a playlist file the user wrote. That is deliberate
  (the playlist upstream is then what the folder holds, and in what order) and
  `docs/library.md` says so before the section on attaching, because it is the
  one moment in this feature where earworm overwrites something it did not
  write.
- **A rename moves the `#m3u8` header with the file.** Left naming the old
  filename, `write_playlist` in a folder with no URL finds a playlist it does
  not recognise and refuses for good: earworm stops updating its own playlist
  after a rename, silently, for the life of that folder.
- **`write_manifest` drops a kept entry naming a file a track now holds.**
  Keyed on the id alone, a match that rewrites a `~` id to a video id leaves
  both lines in the sidecar for one file, and the next sync reads the stale
  one back as a file the playlist does not have.
- **`Pass` carries `gate` and `attaching` as named fields.** Two loose bools
  at six call sites is one transposition away from `--resync` reconciling
  folders nobody is watching. `Pass::attaching()` forces the gate on whatever
  asked for the sync, which is the one thing that overrides the `false`
  literal `resync` passes: the matching is a guess made from tags the user
  typed, and somebody reading "12 to download" against a folder they know
  holds twelve is the last defence. `resync` never reaches it anyway, because
  it skips a folder with no URL outright and logs why.
- **`reconcile` matches by predicted filename, then title, then duration, and
  duration only breaks a tie.** Every other song is three minutes, so a
  duration match on its own would hand a video whatever file happens to be the
  same length. Two files a rule cannot separate match nothing, which costs a
  download where the wrong file costs the user their own music renamed to
  something it is not. Both name comparisons go through `normalised`, composed
  and lowercased, for the reason `mark_departed` does: matched raw, every
  accented or Hangul title misses while the ASCII ones still match.
  `strip_number` takes a leading run of digits *and a separator*, so `1979`
  and `99 Luftballons` keep theirs. The fixtures for this have to be numbered
  where the playlist does not put them, or rule 1 covers rule 2 and the test
  passes with the title matching deleted.
- **A `Local` row is numbered past the playlist on the reopen path too.**
  `open_shelf`'s renumber excluded only `Gone`, so a local file carrying the
  user's own `05 -` competed for index 5 with the playlist's actual track 5,
  and whichever sat first in the sidecar won. The real track then landed at 11,
  sorted to the bottom, and the first edit renamed the file to that position.
  `reconcile` had this right and the read-back path did not, which is the shape
  of every bug this feature has had: the status is set correctly by the sync
  and then lost on the way back off disk.
- **`open_shelf` gives a local id back its `Local` status, and only in a
  folder that has a URL.** Reopening an attached folder reads its files out of
  the sidecar, and a `~` entry there is one the sync could not place. Read
  back as `Have` and `listed`, `mark_departed` finds it missing from the
  `.m3u8` the sync correctly left it out of and calls it `gone`: the row then
  claims a video left a playlist the file was never in. The URL is the other
  half of the test, because in a folder with no playlist behind it every id is
  local and every file genuinely belongs to it. `mark_departed` skips a
  `Local` track for the same reason, or it overwrites the answer the line
  above just gave. Both halves have a test and both were broken to prove it.
- **`Status::Local` is not `Gone`.** `Gone` says a video left the playlist and,
  once proven, lets `D` delete the file. Neither half is true of a file the
  user put there: nothing left, and it is the one thing in the folder earworm
  has no claim on. It is settled, `listed = false`, skipped by `tag_tracks`
  beside `Gone` and `Skipped`, and unreachable from `can_purge`, which reads
  `Gone && departure_proven`. Its legend row is its own rather than a fourth
  word on the "not touched this run" row, which is three slots wide and would
  push every gloss nine columns right on every terminal.
