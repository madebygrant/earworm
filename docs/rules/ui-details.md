# Verbs, text, subprocesses and drawing

## Verbs, modes and the bell

- **One verb per activity, in the header and on the row.** The header said
  `downloading` while the row said `getting`, and `reading` on a row was tag
  reading while `reading playlist` in the header was the scan. Fetch, tag and
  list are now one word each, and every status label still has to fit
  `STATUS_WIDTH`.
- **Following says so, and only while it is live.** `f` toggles silently and
  the cursor being dragged to whatever is downloading reads as a bug. The bar
  word and the flash share `working()` as their condition: once the run has
  settled nothing moves the cursor, so announcing that the mode stopped would
  land on the first `j` of every tag-fixing session and mean nothing.
- **The bell is decided on the frame the run settles, not while it is set.**
  `run_started.elapsed()` keeps growing, so a menu left open for a minute
  would ring for a run that took five seconds. `rang` re-arms when `done`
  clears, because a session syncs more than one playlist.
- **A bell, not OSC 9.** An escape sequence the terminal does not know prints
  its own text into the middle of the frame, and a notification is not worth a
  torn screen.
- **`intro = false` is `intro_done`, not a second flag.** Everything else that
  ends the intro says it is over; a parallel "disabled" flag would be a second
  answer to one question.
- **Enter at the pick with nothing marked refuses.** It means exactly what Esc
  means, and it is one stray `a` from being an accident, so it says what both
  keys do and leaves the question open.
- **`M` is track-list only.** Nothing is tagged at the pick, so every artist
  is empty there and the key could only ever refuse. It matches artists
  case-insensitively: case is a tagging accident, not a different artist.
- **The finish menu's format row needs `config_file`.** Without one the choice
  lasts until the tool closes, which is not what "next time" says.
- **Every error a key can reach ends with the next step.** `failed: track 12
  was never downloaded` is true and leaves the reader to work out that `r` is
  the answer. The separator is the same middle dot the rest of the copy uses.

- **The bar's track title lives in `extra`, not in the hints.** It is the one
  thing on that row whose length earworm does not choose, and the hints proper
  never give way to the tally. Capped as well, so a long title cannot spend
  the whole row before it is dropped.
- **`read_status` is kept apart from `probe`.** `probe` reaches a socket, so a
  test of it would be a test of whether the machine running the suite happens
  to have cliamp up. The parsing is text in, `Playback` out, tested against
  the payload cliamp actually emits.
- **No state on screen is told apart by colour alone.** Statuses are words, a
  missing file carries `!`, the log hint puts `!` in front of itself when
  there is a real error rather than only burning amber, and the bar says
  `♪ cliamp ▶` or `⏸` rather than leaving the transport to the colour. A
  terminal without colour, and a reader who cannot separate amber from dim,
  see the same states everyone else does.
- **A playlist's folder is `%(playlist)s`, chosen again on every download.**
  So renaming it is not a display change: without the `#name` header the next
  sync writes the upstream title again and the whole playlist lands beside the
  renamed folder. `manifest::set_name` records it, `open_shelf` and `resync`
  read it onto `cfg.folder`, and `output_template` is where it takes effect.
  Every path that starts a run sets or clears `cfg.folder`: a new URL must
  clear it, or that playlist is pulled into the last one's folder.
- **Confirming the name a folder already has writes the header.** It is the
  only way a folder renamed in a file manager can be pinned, and it is the one
  answer to that prompt that is not a change. Already pinned says so rather
  than going quiet.
- **Every template field a single video lacks carries a default.** yt-dlp
  prints `NA` for `playlist` and `playlist_index` there, which named the
  folder `NA` and the file `NA - Title`, and made the `@D` and `@P` lines
  unparseable: `parse::<usize>()` failed on `NA`, so the track never reached
  `Downloaded`. The folder is `%(playlist,title)s`, the number is
  `%(playlist_index|01)02d` and the two print templates use `|1`. The default
  is `01` and not `1` because yt-dlp substitutes it verbatim, without the
  `02d` applied. `a_single_video_gets_an_index_and_a_folder_from_its_own_title`
  answers like yt-dlp does, so reverting the `|1` fails it. The scan's own
  print was left alone: its index already falls back to the track's position.
  Measured on a real video; a folder already named `NA` keeps working because
  the manifest still maps the id to its file.
- **A recorded name goes into a yt-dlp template, where `%` opens a field.**
  `folder_field` escapes it, or a playlist called `100% Hits` writes somewhere
  nobody asked for.
- **`manifest::write` and `write_synced` carry the name through.** A tag edit
  or a sync that dropped the header would quietly un-rename the folder at the
  next download.
- **The rename drops cliamp's stored copy first.** The stored name is built
  from the folder's name and the hash of its path, so after the move nothing
  names the old entry and it would sit in cliamp's store for good.
- **The `.m3u8` is renamed with the folder and never rewritten.** It is named
  after its folder, `last_playlist` finds it that way, and its entries are
  relative, so the move itself leaves them valid.
- **`O` reveals, `o` orders.** The sort came first and a key that means two
  things on one screen is worse than a new one.
- **`player::reveal` takes the binary, like the cliamp calls do.** A test that
  ran the real opener would put a window on the screen of whoever is running
  the suite. `opener()` is the platform choice, kept apart so it is testable
  as a choice.

## Text, subprocesses and keys

- **A column is not a character.** `cols()` is `unicode-width`, and every
  padding, truncation, wrap and width budget on screen goes through it. Padded
  to a character count, a column is ragged by the width of whatever CJK is in
  it: one Korean title threw the dim columns after it out by its own length,
  and the status bar, budgeting the same way, pushed `h keys` off the end of a
  row it believed still had room. The caret stays a char index, because typing
  moves by character and not by column.
- **`truncate` stops before a glyph that would straddle the edge,** so it can
  come back a column short rather than spilling into the next field. Callers
  pad to the column count, which absorbs it.
- **`take_cols` always takes at least one glyph.** A width of one against a
  two-column glyph would otherwise take nothing, and `wrap`'s loop would never
  end.
- **A `TestBackend` row is cells, not a string.** A double-width glyph fills
  one cell and leaves the next blank, so `row.find("…")` on the joined string
  is not a column and an assertion built on it cannot see this bug at all.
  Compare cell by cell, or assert on the ASCII half.
- **Byte indexing over a lowercased copy of a string is wrong.**
  `to_lowercase()` doesn't preserve byte length (U+212A shrinks, U+0130 grows).
  Work in chars.
- **`youtube_url` parses the authority, not the whole string.** A link passes
  only if the host itself is one of `HOSTS`, or `evil.com/youtube.com/watch?v=x`
  would.
- **`std::process::Command` has no timeout.** `lookup::run_bounded` polls
  `try_wait` and kills at a deadline. Use it for anything external. It does not
  set the stdio: the caller does, because a piped stream nobody drains fills
  its buffer and blocks the child forever.
- **ureq's global timeout defaults to `None`.** The shared agent in
  `lookup::agent()` sets it. Don't build a bare `Agent`.
- **The rate limit is paced before a request, never after it.** What these
  services ask for is a gap between requests, and one that took longer than
  the gap has already served it: `sleep` after every call paid it twice, and
  on a slow connection that was most of the tagging pass. `Limiter` holds the
  earliest moment the next request may leave, claimed under the lock and
  slept outside it, so the rate is the service's rather than one caller's and
  a pool over these lookups would still honour it.
- **Never embed an AcoustID key.** It comes from `ACOUSTID_API_KEY` only.

## Drawing and config

- **A mode band deliberately punches through the gradient, and must fill the
  row.** `draw_band` backs the filter and the pick. `Paragraph` styles only the
  cells it writes, so the padding is what covers the gradient: pad to the full
  width and drop the right-hand hint when both halves will not fit. A band that
  stops mid-row reads as a rendering fault, where a missing hint reads as a
  narrow terminal. The pick band replaces the filter band, so it carries the
  filter text too, or `a` acts on a narrowed list with nothing saying so.
- **The library preview draws from `Shelf.files`, never from the disk.**
  Presence is resolved in `worker::library`, in the stat pass that already
  counts `missing`, because the draw loop must not stat and a tag read per
  cursor move would need a worker round trip. Filenames carry the number and
  tags already, so the pane needs no tag read at all.
- **One derivation of the selected shelf.** `draw_library` binds `selected`
  once and hands it to the pane, the `ListState`, the `▌` and the scrollbar.
  Recomputing the clamp per widget is how the highlight and the pane come to
  answer two different questions.
- **The intro runs off `started.elapsed()`, never `tick`.** A tick is whatever
  the poll timeout and the message traffic make it, so an animation keyed to it
  speeds up exactly when the worker gets busy. `main` shortens the poll to 33ms
  only while `app.intro()`, and nothing else on screen moves between messages.
- **`App::intro()` holds over the library but never over a prompt.** The
  library is ready in milliseconds on a bare start, so yielding to it left the
  intro unseen by anyone who runs `earworm` alone, and nothing is waiting on
  the user behind a list. A prompt is the opposite: the worker is blocked on an
  answer, and a question nobody can see is not polish. Tracks, a finished run
  and the help modal end it too. Those conditions are also what keeps the intro
  out of the other draw tests, so loosening one breaks them somewhere
  unrelated; the library tests now set `intro_done` to say so out loud.
- **The intro swallows the key that skips it.** Skipping an animation must not
  double as a command aimed at a screen the user never saw.
- **`draw_background` must run first and stay first.** Every other widget
  styles only its foreground, which is what lets the gradient survive
  underneath. A widget that sets a background punches a hole, and `Clear` under
  a popup resets to the terminal's colour, which is why `popup` repaints
  `SURFACE`.
- **A `bool` clap flag can't tell "off" from "unset".** `Config::build` takes
  the raw `ArgMatches` and checks `value_source`, which is why `main` goes
  through `Cli::command().get_matches()` rather than `Cli::parse()`. A flag
  with no matching config key silently makes the file unable to set it. Every
  flag is a negation and goes through `off`, so the command line can only
  switch a feature off and the file is the only way to hold one on.
