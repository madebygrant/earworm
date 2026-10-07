# Wave plan: pick tracks when a custom playlist is made

Written 2026-10-07 against `feat/create-new-playlists`. Treat line numbers as
hints and re-check them before acting.

The request: when a user creates a custom playlist, after they enter the name,
show every track, filter it with fuzzy find, and let them select several
tracks at once.

**Already true, or found while planning:**

- `N` asks for a name, saves an empty list, and flashes "t finds tracks and P
  adds them" (`custom::new`).
- Tracks are added one at a time: `P` on the search screen (`View::Found`)
  adds the row under the cursor, and `P` on a track list adds the marked rows.
  Every add goes through a chooser asking which list (`custom::add`).
- The search is a substring match on filenames (`App::found_rows`,
  `src/app.rs:2086`).
- An empty query shows nothing. CLAUDE.md documents this as a rule ("An empty
  query is no results, not every track"), and this request reverses it for the
  new screen.
- Every library file is already in memory as `Shelf.files`, so listing them
  all costs no disk.

**Decided:**

- The matcher is `nucleo-matcher`, the one Helix uses (0.3.1 on crates.io: 4.7M
  downloads, 1.9M in the last 90 days, last released Feb 2024). It ranks,
  handles Unicode, and returns the matched positions the highlight needs.
  `fuzzy-matcher` has more downloads but no release since 2020.
- Notes on its API, read from the 0.3.1 source:
  - `Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart)`.
  - `pattern.indices(Utf32Str::new(haystack, &mut buf), &mut matcher, &mut at)`,
    with `Matcher::new(Config::DEFAULT)`.
  - Indices are per grapheme and not sorted or deduplicated.
- Esc on the picker after `N` keeps the empty list (recommended, not yet
  confirmed).

## 1. Matching, selection and the add path, built and tested without a screen

Why first: each is a decision the screen depends on, and each can be tested on
its own as plain code.

- **What a row shows.** The filename stem, composed, with its folder beside
  it. Filenames already carry the number, artist and title. Tags would cost a
  worker round trip or a full tag read when the screen opens.
- **A pure `picker::rows(library, query) -> Vec<Hit>`** with the shelf, file,
  score and sorted match positions.
  - An empty query returns every track in library order.
  - A typed query returns ranked results, with a stable sort so equal scores
    keep library order and the list does not shuffle between keystrokes.
  - It leaves out custom rows (their tracks are already in their folders) and
    missing files (they cannot be added).
  - It ignores the `a` kind filter, which narrows the library screen and not
    this one.
  - The query and the label are composed, like every other comparison here.
- **Marks are a list of `(folder path, filename)` in the order marked, never
  row positions.** Ranking reorders rows on every keystroke and a library
  refresh shifts every position. This is the same rule as `App.marked` and the
  search cursor. The mark order is the order the tracks go on the list.
- **`Cmd::AddToPlaylist` takes an optional target list name.** With a target,
  `custom::add` skips the chooser. Without one it asks as it does today, so
  `P` is unchanged.
- **Tests:**
  - An empty query lists every addable track in order.
  - Fuzzy ranking with gaps and out-of-order characters, case ignored.
  - An accent spelled two ways matches both ways.
  - Highlight positions name the matched characters.
  - A 5,000-file library matches within a loose bound, since this runs on the
    UI thread at every keystroke.
  - Marks keep their order and survive a new query and a refresh that moves
    every row.
  - An add with a target writes to that list with no question asked.

Exit: those tests pass, and `nucleo-matcher` is the only new dependency.

## 2. The picker screen

Why here: it is the screen over wave 1's parts, so the keys and the drawing
are the only new work.

- **A new `View`** with its own `can_*` predicate, exclusive with
  `can_command`, `can_browse`, `can_find` and `can_playlist`, as CLAUDE.md
  requires. It gets the same exclusivity test the other four have.
- **Keys.** The query box is always focused, as in any fuzzy finder, so
  letters type.
  - Arrows (and ^n/^p) move.
  - Tab marks and moves down, as fzf does.
  - ^a marks everything showing, or unmarks it when all of it is marked.
  - Enter adds the marked rows, or the cursor row when nothing is marked.
  - Esc closes without adding.
  - Risk: with typing always on, `j`, `k`, `q` and `h` are letters here. Esc
    then `q` still quits, and the band should say what Esc does.
- **Drawing.**
  - A band holding the query and `12 marked  ·  340 of 2,104`.
  - Rows show a mark glyph, the label with matched characters highlighted,
    and the folder dimmed.
  - The highlight uses weight or underline as well as colour, per the "no
    state told apart by colour alone" rule.
  - Only the visible rows are drawn, scrolled with the existing `scroll_to`.
  - Every width goes through `cols()`, since the labels hold CJK.
- **Risk: the empty-query rule.** On this screen an empty query shows
  everything. CLAUDE.md gets a line saying why this screen is the exception.
- **Tests:** TestBackend tests for the band, the mark, the highlight and a
  narrow terminal. Key tests confirm Tab and ^a mark by identity across a
  change of query.

Exit: the screen is reachable from a test `App`, and Enter sends one command
carrying every marked track in mark order.

## 3. Wiring it into `N`

Why here: this is the user-visible flow, and it is only safe once the picker
exists and adds can go to a named list.

- **After the name:** `custom::new` saves the list and sends a message that
  opens the picker with that list as the target. The picker is UI state, so
  the worker is not blocked on a channel the way the pick gate is, and nothing
  waits on an answer the UI might never send.
- **On Enter:** the marked tracks are added, adoption included as today, and
  the list's own screen opens. The flash says how many were added and how many
  were already there.
- **On Esc:** the empty list stays, and the flash says how to fill it later,
  since deleting it would throw away the name just typed.
- **Optional, not in the request:** a key on the list's own screen (for
  example `a`) that opens the same picker for an existing list. It is cheap
  once this wave exists.

Exit:

- A key test runs `N`, a name, two marks across two different queries, then
  Enter. The list holds both tracks in mark order and its screen is showing.
- A second test covers Esc leaving an empty list behind.

## 4. Making everything agree and saying so

Why last: it describes what the earlier waves leave behind.

- **`P` on the search screen** adds marked rows too, if marks are added there
  as well. Otherwise the two screens disagree about what `P` does.
- **Help.** The picker's own help table lists its keys. The library help's `N`
  row says it now picks tracks. The footer hint stays as it is.
- **Docs.**
  - CLAUDE.md: the empty-query exception, identity-keyed marks, the
    exclusivity predicate, and why the picker is UI state and not a worker
    question, each naming its test.
  - `docs/library.md` and the README bullet describe the new flow.
- **A live check** through a pty or by hand. The harness here has dropped keys
  before, so this may be by hand again.

Exit: every new CLAUDE.md rule names its test, and the help and docs describe
the flow as built.

## Why this order

The matcher and the identity-keyed marks are where this would go wrong
quietly: rows reranking under a cursor, or marks that point at the wrong file
after a refresh. So they come first, as plain tested code. The screen is then
only drawing and keys over parts already known to work, and wiring it into
`N` is a few lines once adds can go to a named list.

## What can slip

Waves 1 to 3 are the feature. The `P` consistency in wave 4 can wait, since
the search screen keeps working one row at a time. The docs should not slip,
given how much of this codebase's reasoning lives in CLAUDE.md. Reopening the
picker for an existing list is optional and can come any time after wave 3.
