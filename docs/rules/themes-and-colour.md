# Themes and colour depth

## Themes and colour depth

- **The theme registry is runtime state; `BUILT_INS` is only its seed.**
  `Themes` holds the four that ship plus every `[themes.<name>]`, and the four
  functions that used to scan the const — `named`, `name`, `next`, `names` —
  are methods on it. The reason is that a user theme's name is a runtime
  `String` where `Palette::name()` returned `&'static str`. Keeping the
  registry out of `Palette` is what lets a palette stay a plain `Copy` value a
  draw can hold: the name belongs to the list a theme was found in, not to the
  colours.
- **The theme is tracked by name beside the palette, not derived from it.**
  `App.theme_name` and `Config.theme_name` exist because a `[colors]` table
  makes the palette match no entry in the registry, and the walk still has to
  know where it is. The name is the base the overrides were painted on, which
  is both what `^t` goes on from and what gets written back. This is what
  retired the old "custom" palette: the walk used to match by colour value, so
  a repainted palette matched nothing and dropped back to `warm` rather than
  carrying on from where it was.
- **A key earworm saves goes above the first table, never at the end.** TOML
  puts top-level keys before any table, so `save_key` appending
  `format = "flac"` to a file holding a `[colors]` or `[themes.*]` table wrote
  it *into* that table. It parsed: a string in a `BTreeMap<String, String>` is
  a valid colour as far as serde is concerned, so the re-parse guard waved it
  through, the save reported success, nothing was recorded and the next start
  refused the file naming a colour called "format". Shipped, and found by a
  round-trip test written for something else, in docs/history.md. The scan
  counts brackets
  rather than reading the first character, because a multi-line string can
  hold a line that reads exactly like a header and inserting a key into the
  middle of somebody's value loses their data rather than just misplacing
  earworm's.
- **A theme name has to survive being written back.** `^t` puts it in the file
  as `theme = "<name>"`, and TOML has no escape in what `save_key` produces,
  so a quote or a backslash makes a theme that can be selected and then never
  remembered. Refused at the definition, where the message can name the
  problem, rather than at the save, where `save_key`'s re-parse catches it and
  reports "the edited config no longer parses" about a config that is fine.
  Spaces are allowed, because the rule is the real constraint and not a tidier
  one. This is the first caller to pass user text to `save_key`: `format`
  comes out of `FORMATS`.
- **A defined theme may not take a shipped name, and its `base` may only be a
  shipped one.** Shadowing `warm` would make the built-in unreachable with
  nothing saying so, and a config that redefines it is far likelier to be a
  mistake than an intent. A `base` that could name a peer could name a cycle,
  and resolving that is more machinery than starting from `cool` is worth.
  Both are refused at startup, naming the alternatives.
- **Defined themes walk in `BTreeMap` order, which is by character code.** The
  docs say alphabetical because that is what it is for the lowercase names
  anyone writes, but `Zulu` precedes `aurora`. File order would need `toml`'s
  `preserve_order` feature and would move the walk whenever the file was
  tidied.
- **A theme with no `base` must give every field, and the list is asked of the
  palette.** `define` filters `slots()` plus `GROUNDS` against the table rather
  than naming the twelve again, so a field added to `Palette` is one a baseless
  theme is immediately told to supply rather than one it quietly inherits from
  warm.
- **Every palette is measured, not only the repainted ones.** `unreadable()`
  runs on whatever comes out of `theme()`, because a `[themes.*]` palette is
  as unmeasurable by the suite as a `[colors]` one. It once ran on only the
  second, which docs/history.md records. The four that ship cost nothing
  there, because their ratios are a test.
- **`set_field` takes twelve names, `slots()` lists ten.** The gradient's two
  ends are colours a theme names like any other, but nothing is ever read *in*
  them, so they are grounds for the legibility checks rather than slots. One
  setter for both `[colors]` and `[themes.*]`, so the two take the same names
  and refuse the same way, and `field_names()` is asked of `set_field` so the
  error cannot offer a name that paints nothing.
- **A palette is a set of roles, not a set of pigments.** The slots were the
  colours themselves — `CREAM`, `GOLD`, `TEAL` — which reads fine on one warm
  dark theme and lies on every other: a light theme's "cream" is near-black
  ink. `Palette` names each slot for what it is *for*, because that is the
  part that survives a theme change, and `slots()` lists every one of them so
  a check that must cover the palette cannot quietly miss one. Adding a field
  without adding it there is the mistake that list exists to make hard, and
  `slot_mut` is the second half: every slot earworm holds is drawn in
  something, so every one can be repainted and `slot_names` is asked of
  `slot_mut` rather than written out again.
- **The palette lives on `App`, not in a `OnceLock`.** `^t` changes it between
  frames, and a global resolved once is the one shape that cannot. Every draw
  function either holds an `App` and does `let p = app.theme;` or takes
  `p: Palette` — by value, because `Palette` is `Copy` and the functions
  holding an `&mut App` cannot also hold a borrow of a field on it.
- **The gradient is part of the palette, and so is the intro's fade.** A light
  theme is not a light palette over a dark ground: `near`, `far` and `stop`
  move with the ink or nothing is legible. `glow` was a hardcoded pair — the
  warm theme's rule and a gold brighter than any slot in it — which on `light`
  would have faded dark into dark; it runs `rule` to `accent` off whatever
  palette it is handed.
- **`ink` is read against `accent`, and nothing else is.** It is the text on a
  filled band and the band is filled with `accent`, so measuring it over the
  gradient would pass while it was invisible on the one thing it is drawn on.
  `Palette::grounds` is what answers "what is this slot read against", and
  both the warning and the test ask it rather than listing grounds of their
  own.
- **Every palette is measured, not eyeballed.** `contrast` is WCAG 2.1, and
  `every_palette_is_legible_on_its_own_ground` runs it over every built-in,
  every readable slot and every ground. theme.rs had carried that claim in a
  comment since the first commit, and the claim was false: the old red
  measured 3.73:1 over `near`, on the one colour a failed track is read in. A
  theme cannot now ship unmeasured, and `light`'s `unsure` went back for a
  second shade because of it.
- **The drift check is the default palette's alone; the contrast check is
  every palette's.** `the_default_palette_survives_the_256_colour_cube` is a
  claim about hue, and a saturated red or violet has no near neighbour in a
  cube whose steps are 40 apart: `light`'s error moves 40 and `neon`'s rule
  47, and no shade of either does better. Holding all four to it would mean
  shipping only near-greys. What must survive on all four is legibility, which
  `every_palette_stays_legible_after_quantising` measures after the same trip.
- **A theme changes colour, never layout.** The palette is threaded through
  about a hundred call sites by hand, and the way that goes wrong is a slot
  whose colour also decides a width.
  `a_theme_changes_the_colours_and_never_the_layout` compares every cell's
  glyph across all four and asserts the colours differ in the same pass, or a
  bug that painted every theme warm would pass it.
- **An unknown theme, base, colour or field stops the start; one that merely
  measures badly does not.** Both errors name what it should have been, because a theme that
  quietly does not apply is indistinguishable from a theme system that does
  not work. A bad ratio is the user's screen and their eyes, so it is a remark
  said once and printed by `--check`, where somebody working out why a theme
  looks wrong will actually find it. `theme_notice` is one flash however many
  slots are bad: the flash queue is three deep, so eight repainted colours
  would have shown four and dropped the rest with nothing saying any were
  missing, and eight in a row is nagging even where they all fit. It waits for
  `!app.intro()` for the same reason the update notice does — the flash is
  drawn in the header and the intro does not draw one — and is kept apart from
  the loop so it is testable as the text it is.
- **`[colors]` is a diff on whatever `theme` names, and it outlives `^t`.**
  Changing one colour must not mean restating twelve, and it composes with a
  defined theme rather than competing with it. The table stays in the file and
  is applied again on top of whatever the walk lands on, so the screen now and
  the screen next launch are different colours, and `cycle_theme` says so
  rather than naming the base alone. In the future tense: after the walk the
  live palette *is* a clean theme, so "still repaints it" would be a claim
  about the colours in front of you and those are the ones it is not
  repainting.
- **`^t` writes the config itself, and it is the only thing on the UI thread
  that writes a file.** The worker owns every other one. Routing this through
  the command channel would mean the save waited for the run to end — `serve`
  is not reading that channel during a pipeline — and never happened at all
  for anyone who quit first, which is the one outcome a remembered setting
  exists to avoid. It is one line of a small local file rather than the
  blocking IO that rule is about. What it costs is that the config now has two
  writers: `save_key` is read-modify-write, so a `^t` landing between the
  worker's read and its write in `save_format` or `save_convert` is a theme
  that is silently not remembered. `write_atomically` rules out a torn file
  and not a lost update. Accepted rather than missed: the window is the
  milliseconds around answering the format menu, the loss is one setting
  rather than any data, and a lock over a file written twice a session is
  more machinery than the failure is worth.
- **The theme is the one setting that is not in `Config::describe()`.** Every
  other one is fixed for the run, so a string built once at startup is a fair
  copy of it. `^t` changes this one from inside the tool and nothing re-sends
  `App.settings`, so a copy of the name in there reads `warm` on a cool screen
  from the first press onwards — in the help overlay *and* in the header's
  right half, which draws the same string on the library screen. `draw_help`
  reads it off the palette it is drawing in instead. This is the
  `Msg::Settings` trap by a different road, and it was shipped once.
- **`^t` sits ahead of every screen's key handler, and the intro still eats
  it.** Each screen has its own handler, so a theme key wired into one would
  be dead on the other five with nothing saying why, and the point of walking
  the palettes is seeing them on whatever is in front of you. `^t` rather than
  `t`, which is the library's track search, or `T`, which is a second lookup.
- **The theme's help row is the first thing to give way.** The track list's
  key table is already 21 rows and a 24-row terminal has no room for a 22nd,
  so it goes before the legend and before the settings tail. Of everything on
  that table it is the one that only changes colour, and `--check` and the
  settings tail both still name the theme, and on a terminal short enough to
  drop the tail as well, `--check` does; a key that acts on the music has
  nowhere else to be found. Adding a row here is what makes the rule bite, and
  the failure is a test somewhere else entirely — this one broke
  `the_play_key_and_its_indicator_follow_cliamp`.

## Colour depth and the terminal

- **The palette stays one set of RGB numbers; `recolour` maps the finished
  buffer.** Three palettes would be three things to keep in step, and every
  widget would have to ask which one it was drawing into. It is the last thing
  `draw` does, after every widget has had its say.
- **Truecolor is opt-in through `COLORTERM`, and 256 is the assumption.** A
  truecolor terminal shown 256 colours loses smoothness in the gradient; a
  256-colour terminal shown truecolor loses the text. `depth_from` is kept
  apart from the environment so the decision is testable as a decision.
- **The quantiser weighs the 216-cube against the 24-step grey ramp.** Most of
  this palette is a near-grey, and the cube's steps are 40 apart: cream
  rounding to a cube corner stops being the brightest thing on screen.
- **`NO_COLOR` is presence, not truth.** Any non-empty value means no colour,
  `NO_COLOR=0` included; only an empty value does not count.
- **Without colour a band is reversed video, not a fill.** The band is the one
  thing on screen that must not be missable. The gradient is skipped entirely
  and the popup keeps only its border, which becomes the whole separation.
- **`NO_COLOR` set while running the tests fails about a dozen of them.** They
  assert on `theme::WARM.accent` and friends, and the depth is a process-wide
  `OnceLock`. Test `depth_from` and `shade_at`, never the global.
