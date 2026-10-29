# Plan: rebuild the desktop screens to the wall-and-room mockup

**Source:** `docs/mockups/ideas/6-wall-and-room-icons.html` (the library, an open
folder and a run), the decisions of 2026-10-29 (icons for the mouse, key badges
only in the bar and `h` sheet, the five derived colours computed in Rust,
Manrope bundled), and the code at `7e9238c`. Line numbers are hints; re-check
before acting.

**Already there, and reused:** the reducer, store and key handling
(`crates/gui/src/lib/`); thumbnails on request (`Cmd::Thumbs` / `Msg::Thumb`,
`protocol.rs:472,690`); the palette sent as CSS variables with one derived
value (`vars`, `src-tauri/src/main.rs:109`); the contrast measurement with its
grounds (`theme.rs:360,415`); the session recorder and browser preview, which
is how every screen below gets checked; reduced-motion handling
(`base.css:141`).

**Dropped from the mockup, with reasons:**

- Per-row action buttons in the run table. Kept in the mockup for the mouse,
  but they duplicate the sheet; the sheet is on every screen, so the run gets
  it too (wave 4) and the row stays data.
- The library gauge's exact arc. It's a stat; wave 4 draws it with the ring
  the status counts already give.

**Decisions the plan needs (flagged in their waves):** what the confidence
column shows for tracks AcoustID didn't score; whether the rail's theme menu
survives anywhere besides `^t` and settings.

## 1. Foundations: type, tokens, derived colours, icons, a replay to check against [done 2026-10-29]

Why here: every later wave draws with these, and the derived colours need
their measurement before anything is painted in them.

- **Bundle Manrope** beside the mono: `@fontsource/manrope` in
  `package.json:14`, 400/600/700/800 imported in `main.ts:2`. The CSP's
  `font-src 'self' data:` already allows it (`tauri.conf.json:25`). The OFL
  licence ships with the bundle as the mono's does (`tauri.conf.json`
  resources).
- **A stylesheet of the new look:** `crates/gui/src/look.css` holding the
  mockup's `:root` tokens, type scale and the shared classes (card, pills,
  seg3, sheet, bar, wall, tile) lifted from the mockup's `<style>`. The old
  `tokens.css`/`base.css` imports stay for now (`main.ts:5-6`), so existing
  screens keep working while the new ones arrive.
- **The five derived colours in Rust**, beside `stop` in `vars()`
  (`main.rs:109`): `card2` (surface mixed 10% toward text), `glass` (surface
  at 82%), `accent2` (accent mixed 35% toward far), `glow` (accent at 22%),
  `scrim` (far at 78%). No `color-mix()` in the page: WebKit got it in macOS
  13.1 and the bundle allows 11.
- **Measure `card2` as a ground** in `theme.rs`: add it to what `grounds()`
  returns for the readable slots, since `muted` on it is the weakest pair in
  the mockup. `unreadable()` then reports it, and a test pins one palette's
  answer. Risk, visible: a custom `[themes.*]` that passed before may start
  warning.
- **Update `the_palette_paints_exactly_the_colours_the_stylesheets_use`**
  (`main.rs` tests) to read `look.css` and allow the five derived names the
  way it allows `stop`.
- **An `Icon.svelte`** that draws one symbol from the mockup's sprite, with
  the sprite moved to `src/icons.svg` and inlined once in `App.svelte`. Names
  as in the mockup (`library`, `run`, `album`, `playlist`, `custom`,
  `adopted`, `rename`, `cover`, …).
- **Confidence on the track** (cheap, core): `Track.score: Option<f64>` set
  from `Match.score` where `tag.rs:912` and `worker.rs:4271` fold it into the
  note today; the note keeps its text. TUI literals and `msgs.json`
  re-recorded. Decision: only AcoustID gives a score. The column shows a bar
  for scored tracks and the source word alone for the rest, unless Deezer and
  Apple matches should map to a fixed band.
- **Re-record the two preview sessions** (a 12-track run and the library
  afterwards) with the stub, and keep them under `target/sessions/` with the
  thumbnails beside them. Every wave's exit check is a preview screenshot
  against the mockup frame.

Exit: `pnpm dev` with `?file=<library session>&theme=warm` draws Manrope, the
five derived variables are on `:root` in the app (probe shows them),
`cargo test --workspace` and `pnpm test` pass with the new palette test, and
`unreadable()` names `card2` for a palette built to fail it.

## 2. The chrome: pill nav, crumb, bar and the sheet shell [done 2026-10-29]

Why here: the three big screens all sit inside it, and building it around the
existing screens first means each later wave swaps one region at a time.

- **`App.svelte` layout** becomes header (logo, crumb, pill nav, search, theme
  disc), a main region, the right sheet and the floating bar. The rail region
  goes; `Rail.svelte` stays mounted inside main until wave 3 takes its
  folders.
- **Pill nav** from the mockup's `nav.pills`: Library, Run, Artists,
  Playlists, Settings. "Run" is the tracks view of the folder a run is in;
  it's the current pill when `view === "Tracks"` and a run is going or just
  finished, else Library. Clicking a pill presses the key the terminal would
  (`esc`, `b`, `N`, `,`), through `press` in `keys.ts`.
- **The crumb** replaces `Header.svelte`'s left half:
  `Library / Krautrock · tagging 7 of 12`, from `app.model.stage` and the
  open folder. The run's settings brief moves off the header into wave 4's
  gauge card.
- **The bar** replaces `Footer.svelte`: key badge and word per hint, the
  progress line during a run, the tally at the right. Hints come from the
  same per-view lists `Footer.svelte:25` has now, reformatted. Never an icon
  in it.
- **The sheet shell:** a `Sheet.svelte` with slots for the picture, title,
  reason, fields and actions, taking over from `Detail.svelte`'s frame while
  wave 3 fills it. Hides below 1100px as the pane does (`desktop.md`, the
  plan's layout rule).
- **Search and theme buttons:** the magnifier opens the `t` search; the
  half-disc walks the theme (`walkTheme`). Decision: the rail's theme
  `<select>` (`Rail.svelte:103`) has nowhere to go. Settings' theme row
  already lists the names; drop the select.

Exit: the app opens in the new chrome with today's screens inside it; every
pill and the two buttons do what their key does; the bar's hints match
`keysheet.ts` per view; no icon in the bar carries a letter.

## 3. The library and the open folder as walls, with the sheet filled [done 2026-10-29]

Why here: the most-used screen, and the one the chrome exists for. It needs
wave 1's icons and derived colours and wave 2's sheet.

- **Library wall** (`Wall.svelte` over the shelves): one tile per folder, a
  2×2 mosaic for a playlist from its first four tracks' thumbnails (asked
  through `Cmd::Thumbs` per shelf, with `thumbsAsked` extended to shelves),
  one sleeve for an album, a mosaic with the `custom` icon for a custom
  playlist, a dashed tile with the diamond for a folder earworm didn't
  download. Kind as an icon pill top-right, `n missing` as a word pill
  top-left in `warn`. The cursor is the accent outline.
- **Filters as icon groups:** kind (`a`) and sort (`s`) as `seg3` groups,
  filter box at the right; the keys stay the keys, the group only shows
  state.
- **Folder wall:** the same component over the open folder's tracks, with the
  status pill top-left; dashed numbered tiles for anything without art;
  `failed` tiles outlined in `error`. `v`, `w` and `n` as the three groups.
  The `w` list view keeps `TrackRow.svelte` restyled as the mockup's table
  rows.
- **The sheet, filled:** a folder shows mosaic, name, the kind sentence, then
  icon-labelled fields (tracks, synced, url, folder, m3u8) and Open as the
  primary button with sync, play, show, rename as icon buttons with captions.
  A track shows its sleeve or the "no sleeve until it's confirmed" box, name,
  the reason line in the status's colour, fields (artist, album, video,
  file), Search again as primary and edit, cover, swap as icon buttons.
  Buttons press keys. Reasons come from core's status table, as
  `Detail.svelte` does now.
- **The empty states** from the plan's rules (no folders yet, nothing matches)
  in the wall's area, same words as today.
- Retire `ShelfView.svelte`, `Tile.svelte`, `Rail.svelte` and `Detail.svelte`
  only once both walls and the sheet cover what they did. Risk, visible: the
  rail's folder list is gone; folders are the wall, and `Esc` from a folder
  returns to it.

Exit: the preview's library session matches the mockup's first two frames at
1440 and at 1100 wide (sheet hidden); `j`/`k` move the outline and the sheet
follows; `enter` opens, `esc` returns; thumbnails for every folder arrive once
(probe counts the asks); 23+ page tests pass.

## 4. The run as tiles, a table and a gauge [done 2026-10-29]

Why here: it reuses wave 3's table rows and sheet, and the confidence column
needs wave 1's score.

- **Four stat tiles** over the table: fetched (bars), confirmed (sparkline),
  worth a look and failed naming the track and its reason. Counts from
  `view::counts` over IPC as today.
- **The table:** number, sleeve, title and artist, confidence (segmented bar
  from `score`, or nothing), status word in its slot colour with `tagging` in
  accent, source in mono, length. The cursor row's accent edge. No action
  buttons.
- **The right column:** a "This run" card with the arc gauge (fetched of
  total), the settings brief from `Msg::Settings.brief` where the header had
  it, and the four small counts; the yt-dlp log as a card with the hide icon
  (no `l` badge on it).
- **The bar during a run:** progress line in the middle,
  `⟳ following · tagging <track>` at the right, the run's hints at the left.
- **Follow mode and the status cross-fade** as `Run.svelte` has them, carried
  over; reduced motion keeps the final state only.
- Retire `Run.svelte`'s old layout and `Header.svelte` once the crumb and
  gauge card carry everything they showed.

Exit: the run session at `upto=58` and `upto=84` matches the mockup's third
frame; the confidence column shows a bar only where `score` is set; the
progress line and tally update per message with no re-render of the table
per log line (the untracked-effect rule in `desktop.md`).

## 5. The screens the mockup leaves out, in the same chrome [done 2026-10-29]

Why here: each is a smaller job once the chrome, walls and sheet exist, and
they follow those patterns rather than inventing their own.

- **First launch:** the link field as the hero inside the chrome, with the
  tools table and settings under it, the way `FirstLaunch.svelte` has it,
  restyled; the bar shows `enter sync · ⌘v/^v paste · esc quit`.
- **Dialogs and the pick gate:** `Dialog.svelte`, `Pick.svelte`,
  `Quit.svelte`, `Discard.svelte` on the mockup's card and button styles; the
  pick gate as a sheet over the wall with Download and Take none; the cover
  question's picture grid kept.
- **Artists and the artist page:** a wall of artist tiles (initials, starred
  first) and the page with releases as tiles and the three sections under
  the icon groups; the sheet shows the artist's folders and new releases.
  Locked stays as `discovery.md` says.
- **Playlists:** the custom playlist screen and the picker in the table and
  sheet styles; `N` still goes through the picker's own keys.
- **Search (`t`)** as the table style with the query in the filter box.
- **Settings** as the mockup's own `settings.html` had it, restyled:
  two-column rows with the source word, Enter switches.
- **The `h` sheet** restyled, with the icon rule written into
  `keysheet.ts`'s comment: key badge and word, never an icon.
- **Toast and the achievement popup** on the card style.

Exit: every view in `View` draws in the chrome; `h` on each shows its keys;
the pick gate, the tag form and the cover question each answer through the
dialog in a driven app (`EARWORM_DRIVE`); `svelte-check` is clean.

## 6. Retire the old look [done 2026-10-29]

Why here: only now is it known what nothing reads any more.

- Drop the `docs/mockups/tokens.css` and `base.css` imports from
  `main.ts:5-6` and move what `screens.css` and `app.css` still need into
  `look.css`.
- Delete the retired components and any class only they used; delete
  `docs/mockups/shots/` and mark `docs/plans/2026-10-28-desktop-ux-plan.md`
  superseded at its top with a pointer to the chosen mockup.
- Point the palette test at `look.css` alone; point `preview.ts`'s `?tokens`
  at the new stylesheet's graphite.
- `pnpm build` and `pnpm tauri build`: the `.app` carries Manrope and both
  licences.

Exit: `git grep "docs/mockups/tokens"` finds nothing under `crates/gui`; the
bundle lists both font licences; all suites pass; the preview still replays
both sessions.

## 7. Describe the result [done 2026-10-29]

Why last: it documents the state the other waves leave.

- `docs/gui.md` rewritten against the new screens, screenshots retaken from
  the preview into `docs/images/desktop/`.
- `docs/rules/desktop.md`: the stylesheet rule (the mockups' CSS is no longer
  imported), the derived colours and where they're measured, the icon rule
  ("never a key badge on an icon", with why), the sheet rule (always on the
  right, always the cursor's item).
- `docs/keys.md`: anything the pills and buttons changed (nothing should,
  since they press keys).
- `CLAUDE.md`'s `crates/gui` paragraph: the chrome's files.

Exit: every new rule names its test or says nothing tests it; `docs/gui.md`'s
pictures are of the shipped page.

## Why this order

Type, tokens, derived colours and icons are what every screen is drawn with,
and the derived colours need their measurement before a theme can be trusted
on them, so they go first and alone. The chrome goes next because it's the
frame the three mockup screens drop into, and building it around today's
screens means the app never stops working between waves. The library wall is
the screen the redesign is for and the sheet is shared by everything after
it, so it precedes the run, which borrows its rows; the leftover screens
follow the patterns those two set. Deleting the old stylesheets and
components waits until nothing reads them, and docs describe what's left.

## What can slip

Waves 1 to 4 are the redesign; without them nothing has changed. Wave 5 can
ship screen by screen, and the artists and playlists screens can wait a
release in the old style inside the new chrome, since wave 2 keeps them
working. Wave 6 can slip but not for long: two stylesheets and two tile
components in the tree is where the next bug hides. Wave 7's screenshots can
wait; its rules can't, since the icon rule and the derived colours are the
kind of thing the next change undoes.
