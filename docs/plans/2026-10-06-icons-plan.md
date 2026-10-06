# Wave plan: icons instead of text indicators

Written 2026-10-06 against `feat/loudness-tags`, with uncommitted changes.
Treat line numbers as hints and re-check them before acting.

The request: replace text indicators with icons (a CD for an album),
investigate icon libraries for ratatui, and give downloaded playlists a tag
too.

**Already true:**

- Some indicators are already glyphs: `▶` and `⏸` on library rows
  (`src/ui.rs:894`), `♪ cliamp` in the status bar, and `!` on a missing file
  in the preview (`src/ui.rs:1027`).
- The text indicators are the two filled pills: `album`, drawn by `kind_pill`
  (`src/ui.rs:781`), and `local`, in the sync column (`src/ui.rs:926`).
- Playlists get no tag on purpose. "Silence is the playlist answer" is a
  documented rule (`src/ui.rs:772`, CLAUDE.md under "Albums and playlists").
  This request reverses it.
- ratatui 0.30 has no icon set. Its `symbols` module covers borders, bars,
  braille and markers.

**Icon libraries, checked 2026-10-06:**

- The crates that exist are lookup tables of Nerd Font codepoints:
  [nerd-font-symbols](https://docs.rs/nerd-font-symbols),
  [nf-icons](https://docs.rs/nf-icons) (a compile-time macro over Nerd Fonts
  3.4.0), [nerd_font](https://docs.rs/nerd_font),
  [nerd_fonts](https://docs.rs/nerd_fonts) and
  [rnk-icons](https://docs.rs/rnk-icons).
- None of them can draw an icon. They give you a Private Use Area character,
  which shows as a box unless the terminal font is a patched Nerd Font. The
  font decides whether this works, not the crate.
- Four or five glyphs don't justify a dependency. Recommended: `const` glyphs
  in the code.
- [tb-tui-common](https://docs.rs/tb-tui-common)'s `font_probe` guesses
  whether a patched font is installed from how far the cursor moves. Most
  terminals advance by the character's declared width, not by what the font
  draws, so it probably can't see a missing glyph. Measure it before relying
  on it.

## 1. Glyph measurements and decisions

Why first: the chosen glyphs set the column widths, the config key and what
the tests assert. Changing any of those after shipping costs a second layout
change.

- **Investigation: candidate glyphs.** Render each candidate in the terminals
  in use, in both truecolor and 256-colour mode, and record the results in
  docs/history.md.
  - Nerd Font: `nf-md-album`, `nf-md-playlist_music`, `nf-md-folder_music`.
  - Plain Unicode that most fonts have: `◉` or `◎` for an album, `≡` or `♫`
    for a playlist, `⌂` for local.
  - Emoji such as 💿: expected to be rejected. Colour emoji ignore the
    foreground colour, which breaks themes, `NO_COLOR` and the 256-colour
    quantiser. They are also two cells wide, and terminals disagree about the
    emoji variation selector.
- **Investigation: widths.** Check that `cols()` (unicode-width) matches what
  the terminal actually draws for each chosen glyph. The non-Mono Nerd Fonts
  draw some icons wider than one cell, which spills into the next column. If
  that happens, the slot needs a trailing space.
- **Investigation: `font_probe`.** Run its method against a terminal with and
  without a patched font. If both answers are the same, detection is off the
  table.
- **Decision: icon tiers.** Recommended: a config key
  `icons = "text" | "symbols" | "nerd"`, defaulting to `symbols`. That tier
  works with any font, and `nerd` is opt-in. A setting rather than detection,
  given the probe doubt above.
- **Decision: what "downloaded playlists should also have a tag" means.**
  There are two readings: the kind (playlist versus album), or where it came
  from (downloaded versus `local`). Recommended: a kind icon on every row
  (album or playlist), with `local` keeping its place in the sync column. A
  folder can be both an album and local, which is why the two never shared a
  slot.
  - Risk: this reverses a documented rule.
- **Decision: which indicators change.** Recommended: the library's kind and
  local markers only. Status words on the track list stay as words. "Colour is
  confidence, the word is provenance" is a CLAUDE.md rule, and an icon can't
  name more than a dozen statuses readably.

Exit: a measured glyph table in docs/history.md, and the three decisions
written down here.

## 2. The setting, with no visual change

Why here: landing the setting first, with `text` drawing exactly what it draws
today, keeps wave 3's diff to drawing code only.

- Add an `icons` key to `FileConfig`, `Config` and `App`. Every command-line
  flag can only switch something off, so decide whether this needs a flag at
  all. Recommended: config only, like `theme`.
- Add it to `Config::describe()` and the help overlay's settings tail, so
  `Msg::Settings` carries it.
- Add a `--check` row naming the tier, because "why is there a box on my
  screen" is the question that row would answer.
- Put the glyph table in one place, `ui.rs` or `theme.rs`, keyed by tier and
  indicator.

Exit: the full test suite passes unchanged with `icons = "text"`, and
`--check` prints the tier.

## 3. Library rows draw icons

Why here: this needs the glyph widths from wave 1 and the setting from wave 2.

- **Kind slot on every row.** It replaces the 8-column album slot that only
  appears when some row is an album (`KIND_WIDTH` at `src/ui.rs:25`, the
  per-draw rule at `src/ui.rs:865`). An icon slot is 2 to 3 columns, so a
  library that has albums gains name width and one that has none loses 2 to 3
  columns.
  - Risk: this changes the layout of every library.
- **Local marker.** Draw `local` as an icon in the sync column under
  `symbols` and `nerd`, and as the pill under `text`.
- **Colour.** Draw the icon in a measured palette slot. The contrast tests in
  `theme.rs` have to cover that slot on every ground it sits on. The glyph
  carries the meaning, so colour alone still tells nothing apart.
- **Folder-name markers.** `without_marker` strips `[album]` from the name
  only (`src/ui.rs:852`). Once playlists have an icon, `[playlist]` is said
  twice too, so strip both.
  - Risk: visible change to folder names on screen.
- **Tests.** Per tier, check that the glyph lands in the slot cell and the
  columns after it stay aligned. Compare cell by cell, because a two-cell
  glyph breaks string positions (CLAUDE.md, "A `TestBackend` row is cells").
  - The existing pill tests (`src/ui.rs:3348`, `src/ui.rs:3576`, and the
    `WARM.ink` / `WARM.accent` fill test) become `text`-tier tests.
  - `a_theme_changes_the_colours_and_never_the_layout` must still pass.

Exit: all three tiers draw a 40-folder fixture with the columns aligned.
Break the slot width and watch the alignment test fail.

## 4. Explaining and documenting

Why last: an icon doesn't explain itself the way a word does, and the docs
describe what wave 3 leaves behind.

- An icon legend in the library's help. It follows the existing order in
  which rows give way on a short terminal: the legend goes first.
- CLAUDE.md: rewrite "Only an album is marked", "The album pill's slot is
  reserved per draw" and the `without_marker` rule. Add the tier rule and why
  emoji were rejected.
- README and docs/library.md: the icons and the `icons` key.

Exit: every rewritten CLAUDE.md rule names its test, or says that nothing
tests it.

## Why this order

The glyph table and the tiers are what's costly to change, so they come
first. The setting lands with no visual change, so wave 3 is the only one that
moves layout. Docs come last because they describe that result.

## What can slip

The `nerd` tier can slip entirely: `symbols` covers the request with no font
dependency. Waves 1 to 3 are the core. The legend in wave 4 can't slip past
the release that adds the icons.

Wave 3 should land before wave 3 of the custom playlists plan
(`2026-10-06-custom-playlists-plan.md`), so the custom playlist row gets its
icon in the same slot rather than another text pill.
