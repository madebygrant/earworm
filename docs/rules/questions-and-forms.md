# Questions, forms and quitting


- **`App.confirm` is the UI asking itself, `Prompt` is the worker asking.**
  Nothing is blocked on a confirm, so it carries no reply channel: the run
  keeps going behind it. It is handled ahead of every screen's keys, or the
  answer also reaches the list underneath.
- **Only a named key confirms a quit.** `y`, `q`, Enter and ^c mean yes and
  everything else means no. `q` meaning yes is deliberate: `qq` quits without
  anyone reading the box, which is what stops the guard becoming a tax on the
  people who meant it.
- **`working()` is the one predicate behind Esc and `q`.** Two notions of "in
  flight" would make the safe key safe on a different set of screens from the
  one that asks. It is `busy` or an unsettled run that has tracks, so it is
  false before the first list arrives.
- **`App.fields` is the only text state, and a single prompt is a form with
  one box.** Keeping a separate `input` beside the form's values meant two
  sources of truth and a sync on every Tab. `input()` is the focused box;
  `box_mut` is what every editing key writes through.
- **`^u` clears the focused box, never the form.** The other box holds an
  answer being kept.
- **Only the focused box draws the block.** Two cursors in one popup and
  neither of them is where typing lands.
- **`edit` and `tag::manual` ask the same form.** Both used to ask artist
  then title in sequence, so the second question covered the answer to the
  first at the moment you wanted to compare them. They no longer ask the same
  boxes: `edit` asks four and `manual` two, because during a run the album
  comes from the lookup and the question is which track this is.
- **`^s` finds its two boxes by label, never by position.** They sit at 0 and
  1 in both forms, so a positional swap goes on working right up until
  somebody reorders them, and a test over either form passes either way.
  `swappable` is what the key and the hint advertising it both read, so the
  two cannot disagree about whether this form has them.
- **A write writes every tag, so a caller changing one carries the rest.**
  `tag::Fields` is the whole set and `set_fields` writes all of it, which is
  what `held_fields` exists to supply: without it a swap or an `A` across
  twenty tracks stripped the album and year off all of them, and the row
  looked right because the row has no column for either.
- **An empty album or year removes the tag rather than writing a blank one.**
  lofty writes an empty album as a present-but-blank frame, which reads back
  as a tagged file whose album happens to be `""` and stops `tag_tracks`
  filling it in from the playlist name.
- **lofty 0.25 has no year of its own.** `date()` reads the recording date and
  falls back to the bare `Year` field; `set_date` writes a `Timestamp` whose
  month and day are `None`. A year that will not parse refuses the edit rather
  than writing nothing over what the file had.
- **`Msg::Meta` carries album and year, not `Msg::Update`.** They change only
  when somebody edits them, where `Update` is sent for every status a run
  passes through. Same reason `Path` is its own message. Every path that
  fills them in has to send it, which is what `offer_meta` is for: the
  worker's `Track` is not the one `draw_detail` reads, and `tag_tracks`
  filling in the album without saying so left the pane blank after a run and
  populated after reopening the same folder from the library.
- **The header's right half is dropped whole, never truncated.** Half a
  progress bar is a lie about how far along the run is. The left half names
  what you are looking at, so it is what stays.
- **`App.run_started` is set by `Msg::Tracks`, not by `App::new`.** `started`
  includes the intro and however long the URL prompt sat there, which would
  make the first estimate of every run enormous.
- **The estimate is coarse and only ever approximate.** Tracks already on disk
  settle the instant the list arrives, so a part-downloaded folder reads
  optimistic until real work lands; `eta` waits for three settled tracks and
  hides anything under thirty seconds rather than pretending to a precision it
  does not have.
