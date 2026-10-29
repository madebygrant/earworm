# UX and UI plan: the earworm desktop app

**Superseded 2026-10-29.** The app now follows
`docs/mockups/ideas/6-wall-and-room-icons.html`, built by
`2026-10-29-desktop-redesign-plan.md`. The screenshots this plan showed are
deleted; each now links to the mockup page it was captured from.

Written 2026-10-28. It covers how the Tauri app from
`2026-10-26-native-app-plan.md` looks and behaves; waves 6 to 8 there build
it. Every screen below links to its mockup page. Open
`docs/mockups/index.html` in a browser for the live versions: each is HTML,
themed and keyboard-driven where it matters, built on the tokens and styles
the Svelte app should copy.

## What was chosen, and from what

Seven directions were mocked and compared (`docs/mockups/directions/`). The
one taken is **G, the Phosphor sheet**:

- the TUI's look carried over: one monospace face, the header line with the
  run's settings, the rail with `●` `▤` `◆`, the key hints, the stripes;
- the open folder drawn as a contact sheet, one tile per track, with the
  TUI's list one key away;
- **graphite** as the default palette, a neutral grey ground that lets the
  covers carry the colour;
- **JetBrains Mono** for everything, chosen over six pairings that split
  names from data (`docs/mockups/directions/type-specimens.html`).

Rejected, so nobody reopens them without a reason: a warm green-and-amber
humanist look (A), a light cover-led look (B), a poster built from the
cover's colour (D), a kanban of verdicts (E). The sorting idea in E is worth
keeping as a filter, not a layout.

## Who it is for and what it does

People who download playlists and then rename the files by hand, many of whom
already live in the TUI. The app does two jobs: one link in, one tagged
folder out; then years of small fixes to that library. Every screen serves
one of the two. The GUI is a second front end on the same core, so it never
invents behaviour the TUI lacks; it shows the same state in a form a mouse
can reach.

## Rules carried over from the TUI

Each of these is already a rule in `docs/rules/` and holds here unchanged.

- **The word is provenance, the colour is confidence.** `ok`, `manual` and
  `on disk` share the cursor colour because each was checked. Nothing is told
  apart by colour alone: every tile carries its status word, a failed or
  missing file carries `!`.
- **Every key in `docs/keys.md` works, and a letter means one thing on every
  screen.** The mouse is a second way in, never the only one.
- **Esc unwinds and never quits.** Filter, then marks, then back. `q` asks
  while a download runs, and `qq` quits without reading the box.
- **The tally beats the hints** when the status bar runs out of room, and the
  header drops its right half whole rather than truncating it.
- **A prompt says what Esc does**, in words, on the prompt.
- **A theme changes colour, never layout.**

## Visual system

### Colour

Five themes, each the ten roles of `Palette` in `src/theme.rs` plus the two
gradient ends. `graphite` is new; the other four are the TUI's own values.
`^t` walks all five and saves the one you stop on, as in the TUI.

| Role | graphite | Used for |
|---|---|---|
| text | `#F1F2F4` | titles, values |
| accent | `#9DB7FF` | chrome, the run's verb, in-progress tiles, keys in hints, filled bands |
| cursor | `#7CDDB2` | the cursor ring and bar, `ok` `manual` `on disk` |
| warn | `#F5B867` | `guessed` `unsure` `no match`, missing files, log warnings |
| error | `#FF8F87` | `failed`, log errors |
| muted | `#A0A6B2` | sources, notes, `queued` `gone` `skipped` |
| unsure | `#B9BEC8` | "we looked and found nothing" |
| rule | `#4E525C` | borders, dividers, empty tile outlines |
| surface | `#262A31` | dialogs, tinted rows |
| ink | `#16181D` | text on an accent fill |
| near, far | `#1D2026`, `#121418` | the window gradient, 160°, stop 0.76 |

Measured with `node docs/mockups/contrast.mjs`, the same WCAG 2.1 formula as
`theme::contrast`: the lowest ratio is muted on surface at 5.89:1, and ink on
accent is 9.01:1. Every readable slot clears 4.5:1 on every ground.

With no theme saved, the app follows the system: `light` in light mode,
`graphite` in dark.

### Type

JetBrains Mono at 400 and 700, bundled with the app. One size scale:

| Size | Used for |
|---|---|
| 11 | marks on tiles, pill counts |
| 12 | captions, hints, the tally, the log |
| 13 | body, tile titles |
| 14 | list rows, the detail pane, dialogs |
| 22 | a screen's title |
| 28 | the first-launch link field |

No capitals for labels; the TUI's `PICK` and `FILTER` bands are lowercase
here. Sentence case for everything else.

### Shape and space

A 4px grid, 16px gutters. No rounded corners and no shadows; a dialog
separates from the gradient by its flat `surface` and a 1px `accent` border
with its title set into the top edge, as the TUI draws popups. Dividers are
1px `rule`.

## Layout

[The library screen: the rail of folders on the left, the open folder as a
sheet of tiles in the middle, the folder under the cursor in the detail pane
on the right, the tally and key hints along the
bottom](../mockups/library.html)

Four regions, fixed: the header (40px), the rail (232px), the detail pane
(340px) and the status bar (30px). The middle takes the rest.

- **The rail** is the library. It holds folders, or artists after `b`, with a
  filter, the sort (`s`) and the kind filter (`a`) above the list and a theme
  dropdown at the foot, labelled with `^t`, which moves it too. The open
  folder is the filled row; a folder syncing shows `⟳`, one with departures
  shows its count and `!` in warn.
- **The middle** is the open folder: tiles, or rows after `w`.
- **The detail pane** is the track or folder under the cursor in full: the
  cover, one sentence saying what the status means, the tags, the path, and
  the keys that fix it.
- **The status bar** is the tally on the left, hints on the right, as in the
  TUI.
- Below 1100px the detail pane goes, as the TUI's does below 100 columns.
  Below 760px the rail goes and the header drops its right half.
- The window title follows the run: `earworm · Krautrock · 9/14`, then
  `finished`.

## Tiles

A tile is a square frame, then a caption line (`03 ok`), title, artist. The
frame says the state before the word does:

| State | Frame |
|---|---|
| has art (`ok`, `on disk`, `guessed` and the rest) | the cover, pixel-sharp |
| `queued` | dashed rule outline, the track number |
| `fetching`, `fetched`, `tagging` | accent outline, a scan line moving down it |
| `failed` | error outline, `!` |
| missing file | dashed warn outline, `!` |
| `gone` | the cover, greyed and dimmed |
| cursor | 2px cursor ring outside the frame |
| marked | a `✓` in the corner |

**The one piece of motion that matters:** when a track settles, its cover
arrives at 4×4, then 8×8, then full size over 340ms, the way a terminal would
draw it. Status words change without animation. The scan line runs only on
working tiles. `prefers-reduced-motion` turns both off and shows the final
cover.

Covers are drawn with `image-rendering: pixelated` only in the mockups, where
the art is generated. Real covers render normally at the tile's size from the
embedded art, never upscaled past 1000×1000.

## Screens

Each is a page in `docs/mockups/`; the picture is that page as captured.

1. **First launch** ([`first-launch.html`](../mockups/first-launch.html)). The link field is the hero, 28px,
   with the line "Or drop a folder of music you already have, or a markdown
   file of links, anywhere on this window." Under it, `--check`'s table: each
   tool found with its version, or missing with the install command to copy.
   The rail says "No folders yet. A synced playlist lands here."

   [The first launch: a large link field over the tools table, an empty rail](../mockups/first-launch.html)

2. **Library** ([`library.html`](../mockups/library.html)). A folder opened from disk, at rest. Missing
   files are tiles in their own right, so a gap in an album is visible at a
   glance. The detail pane describes the folder (kind, count, last sync, URL,
   playlist file, the missing files by name) with `S` `p` `O` `e` `D`.

   [The library: Ambient 1975 open as tiles, three missing files as dashed tiles with !, the folder described in the pane](../mockups/library.html)

3. **A run** ([`run.html`](../mockups/run.html)). Live. Follow mode drags the cursor to the working
   tile and the bar says `⟳ following`; any movement key ends it and says
   so. The progress bar is block characters, the only progress widget in the
   app. `l` opens the yt-dlp log along the bottom third; its hint burns error
   only when the log holds an ERROR.

   [A run: the progress bar, settled covers, the fetching tile with its scan line, queued tiles as numbers, the log pane open](../mockups/run.html)

4. **Pick gate** ([`pick.html`](../mockups/pick.html)). Always rows, never tiles, since nothing has
   a sleeve yet. A filled `pick` band counts the selection and names the keys.
   `[x]` boxes, likely twins marked `≈ in Chill Evenings` with the copy
   explained in the pane, and a long upload flagged by its length. A button
   names its result, "Download 13 tracks", beside "Take none".

   [The pick gate: a filled band counting the selection, ticked rows, a twin marked, the Download 13 tracks button](../mockups/pick.html)

5. **Prompts** ([`prompts.html`](../mockups/prompts.html)). The finish menu with the review row first
   when there is something to review, the two-box form the run asks, the
   four-box `e` form, a link prompt for a folder with no URL, a refused link
   with its reason, and the `D` confirmation naming every file.

   [Six prompts: the finish menu, the two-box and four-box tag forms, a link prompt, a refused link, the delete confirmation](../mockups/prompts.html)

6. **Track detail** ([`detail.html`](../mockups/detail.html)). The pane in all nine states, each with
   its sentence and its keys.

   [The detail pane in nine states, from ok to undone](../mockups/detail.html)

7. **Artists** ([`artists.html`](../mockups/artists.html)). `b` turns the rail into the artist list,
   starred first, `+2 new` where a release is waiting. The page lays releases
   out as tiles, marked `● owned`, `◐ 2 of 6` or `○ missing` with the missing
   ones greyed. Shown locked: no hint, key or button mentions downloading.

   [An artist page: the artist list in the rail, releases as tiles marked owned, partial or missing, one release open in the pane](../mockups/artists.html)

8. **Settings** ([`settings.html`](../mockups/settings.html)). Every row with its value, where it came
   from and what it does, grouped. A row a flag holds is in warn, and the pane
   says to start without the flag rather than pretending to switch it.

   [Settings: every row with its value, source and meaning, the pick row held by a flag](../mockups/settings.html)

9. **Overlays** ([`overlays.html`](../mockups/overlays.html)). The key sheet grouped by job with the
   status legend under it, the quit question, `t` search across the library,
   an achievement as a corner toast, and the console as one line along the
   bottom that names nothing it unlocks.

   [The key sheet, the quit question, library search, an achievement toast and the console line](../mockups/overlays.html)


## Interaction

- **Keyboard first.** Every hint is a key and every key works with the mouse
  out of reach. In the tiles, `j` `k` move a row and `←` `→` a tile, so the
  list's keys keep their meaning.
- **Focus** is a 1px accent outline, always visible.
- **Clicking** a tile moves the cursor there and ends follow mode, the same as
  a movement key.
- **Dropping** a link anywhere starts a sync; dropping a folder on the rail
  adopts it; dropping a markdown file imports it. Each shows a dashed accent
  outline over the target while dragging.
- **Right-click** on a tile or folder opens a menu of the same actions the
  pane shows, each with its key.
- **The bell** is the system alert, after a run longer than half a minute.
- **Copy** uses the TUI's words and no others: sync, pick, retry, undo,
  folder, playlist, album, sleeve. A button names what happens ("Download 13
  tracks") and the result uses the same verb.

## Decisions made while planning

These were not asked about; each is easy to reverse.

- **`w` switches tiles and list.** It is free on every screen in
  `docs/keys.md` (`w`, `x`, `y`, `z` are). It is a GUI-only key until the TUI
  has a tile view, and should be added to `docs/keys.md` when the GUI ships.
- **The light partner for graphite is the TUI's `light`**, not a new light
  graphite. It is measured already, and a second new palette would need its
  own numbers.
- **The pick gate forces rows** and returns to the previous view after.
- **Artist releases use tiles**, since a discography is a wall of sleeves.

## Open questions

- **Does `graphite` ship in the TUI too?** Decided in wave 2 of the desktop
  plan, before the theme data moves to core. It would be a fifth entry in
  `BUILT_INS`, and `every_palette_is_legible_on_its_own_ground` would cover
  it with no new test. The 256-colour drift check is the default palette's
  alone, so graphite would not need to pass it unless it became the default.
- **Real covers in tiles** are settled: wave 3 of the desktop plan adds a
  message carrying a cached thumbnail's path, so the UI never reads an audio
  file.
- **YouTube thumbnails at the pick gate.** yt-dlp's flat listing has them,
  which would let the gate show tiles. Fetching them is network work the gate
  does not do today.

## Implementation notes for waves 6 to 8

Where each part of this plan lands in `2026-10-26-native-app-plan.md`:
graphite in wave 2, the thumbnail message in wave 3, tokens, fonts and the
theme dropdown in wave 6, the first seven screens in wave 7, the rest in
wave 8, the font licence in wave 9, `w` and the GUI rules in wave 10.

- `docs/mockups/tokens.css` is the reference, but the app generates its CSS
  variables from core's palettes at start. The role names are the `Palette`
  field names, so `[themes.*]` and `[colors]` from the config apply with no
  renaming.
- `docs/mockups/base.css` is the component sheet; its class names are a
  suggestion, its values are the spec.
- The run screen's repaint rule is in `docs/mockups/run.html`: a tile
  repaints only when its status changes, which is also what keeps the cover
  animation from restarting.
- The keys each mockup handles (`j` `k` `←` `→` `w` `l` `f` on the run,
  `space` `a` `i` on the pick gate) are the ones to wire first.

## Verification

- `node docs/mockups/contrast.mjs` exits 0.
- Every mockup page opens in Chrome at 1440 wide in graphite with no console
  errors. The run screen was also checked at 880 wide in light: the detail
  pane hides and nothing scrolls sideways. The other pages have not been
  checked narrow or in the other four themes yet.
- Read each page's hints against `docs/keys.md`: every key shown exists and
  means the same thing on every screen, apart from `w`, which is new.
