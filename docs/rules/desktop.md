# The desktop app

- **The page keeps no rule of its own.** What a list shows, what a marking
  key takes, each status's word and whether it wants a look all come from
  core: the list rules as `view` commands, the status table in `start`'s
  reply. A copy in TypeScript would hold until the next change to the Rust
  one and then quietly disagree, which is the failure wave 5 moved the rules
  into core to prevent. The reducer asks for anything it needs from core as
  an effect (`GateMarks`) rather than working it out
  (`asks_core_for_the_pick_gate's_marks_rather_than_choosing_them` in
  `reducer.test.ts`).
- **The worker starts when the page asks, not when the window opens.** `start`
  is called after the page has subscribed to `earworm://msg`; a worker
  started in `setup` sends the library before anything listens and the wall
  stays empty. `boot` runs once, since a second listener applies every
  message twice (`listens_once_however_often_it_is_mounted` in
  `store.test.ts`).
- **A reload replays the history; it never cancels a question.** The worker
  sent the library once, so a reloaded page that started blank would show
  no folders until something sent it again, and cancelling its open
  questions answered the first URL prompt with Esc, which quits the worker.
  The shell keeps every message, numbered, and `start` hands back all of
  them but the questions the page already answered
  (`a_replay_leaves_out_the_questions_already_answered`). Live messages that
  arrive while `start` is answering wait, and those the history held are
  dropped by number
  (`applies_the_history_then_the_held_messages_it_did_not_hold`). The cost is
  memory: a long session's yt-dlp log is kept twice.
- **A search clears `stop` when it is sent, as the terminal's does.** Nothing
  else clears it, so one Esc on a search would end every later one at once
  (`a_search_clears_the_stop_and_nothing_else_does`).
- **The reducer is tested against what the worker sends.** `msgs.json` is
  written from `protocol::test_support::every_msg`, the samples the JSON
  round trip uses, and `the_recorded_messages_are_every_message_the_worker_sends`
  fails when it is stale (`EARWORM_RECORD=1 cargo test -p earworm-gui`
  rewrites it). The reducer test then requires a handler for every recorded
  variant and no handler for one that does not exist, so a new `Msg` fails
  on both sides until the page handles it.
- **The palette is core's, sent as CSS variables.** `look.css`'s `:root`
  holds graphite, or light on a light system, so the window does not open in
  the wrong ground; `paint` overwrites it with the palette `Config::build`
  resolved, so `theme`, `[themes.*]` and `[colors]` apply as they do in the
  terminal. The variables painted are exactly the ones `:root` defines, its
  values are core's, and any other variable is a measure `body` sets
  (`--sheet`) or an alias `.look` defines, read only under `.look`, where it
  resolves
  (`the_palette_paints_exactly_the_colours_the_stylesheets_use`). Whether
  a theme is light is read off its ground, not its name.
- **`look.css` is the page's only stylesheet.** The mockups' `tokens.css`
  and `base.css` styled the same class names (`.sheet`, `.tile`, `.log`,
  `.bar`) for the old screens, so every new rule had to be shielded with
  `all: revert`, and a rule meant for one log leaked onto the other once the
  shield moved. `main.ts` imports `look.css` and the `@fontsource` faces, and
  no file under `src/` brings in styles of its own: no other `.css` import,
  `<style>` block or `@import`. The mockups stay under `docs/mockups/` as
  pictures
  (`the_page_imports_look_css_and_the_fonts_and_no_other_stylesheet`).
- **The desktop's derived colours are worked out in Rust, beside the
  palette, and measured there.** `card2` (the well inside a card: the active
  pill, a stat tile, a key badge), `glass` (the bar), `accent2`, `glow` and
  `scrim` are mixed by `vars` in `main.rs`, `card2` through
  `Palette::card2`, rather than with CSS `color-mix()`, which WebKit lacks
  before 16.2: the bundle runs from macOS 11, and a Mac on 11 or 12 that
  never took a Safari update has an older one. `theme.rs`'s
  `desktop_grounds` measures every readable slot on the card wells and on
  the bar composited over each end of the gradient
  (`every_palette_is_legible_on_its_own_ground`,
  `muted_is_measured_on_the_desktops_card_wells`). `card2` sinks `surface`
  halfway to `far` rather than lifting it toward `text`, which put warm's
  `muted` at 3.7:1. Text on `scrim` (a tile's status pill and caption) is
  not measured: on a sleeve it is over a picture, and on an empty tile
  nothing checks it. A `[themes.*]` theme is measured on the terminal's
  grounds only: nothing calls `desktop_unreadable` outside its test.
- **Never a key badge on an icon.** Icons are for the mouse: a pill, a
  toolbar button or a sheet button shows an icon or a word, and does what a
  key does, by pressing it or by setting the state the key walks to (the
  kind, sort and tiles/list buttons). Key badges appear only in the bar, the
  `h` sheet and a dialog's foot hints, each as a badge and a word. A badge on an icon
  reads as a second, different action, and the icon and the badge then
  disagree about what the button is. Nothing tests it.
- **The run is a chip, not a pill.** A run starts by syncing a link and is
  never gone to, so a Run pill was disabled nearly always and read as
  broken. The reducer keeps the last download in `run`: it starts at the
  pick gate or the first track in hand, which opening a folder never has,
  and `Done` closes it with its look and failed counts, since its tracks are
  gone once another folder is opened. The chip opens that folder again, as
  Enter on it in the library does, and a folder opened any other way lights
  Library (`remembers the last download, and opening a folder from disk
  neither starts nor ends it`, `starts the run at the pick gate, before any
  track is in hand`).
- **The sheet is on the right and shows what the screen's keys act on.**
  That is the item under the cursor, except in the playlist picker, where
  Enter acts on the whole list, so the sheet shows the list's order after
  Enter. Its buttons do what that screen's keys do (the artist page's `o`
  opens the artist's folders whichever release is under the cursor), so a
  sheet showing anything else would put a button on one thing that acts on
  another. A run in progress puts its own card there instead. Below 1100px
  wide the sheet hides; the pick gate's Download and take none move into its
  toolbar, and every other button is a key. Nothing tests it; the reducer
  tests have no DOM.
- **`^t` and the theme button save through `config::keep_theme`, and say so
  through `Kept::said`.** The same calls the terminal's `^t` makes, so a secret theme is never written and
  `--no-config` keeps the file out; why this write skips the worker is in
  `themes-and-colour.md`.
  (`a_kept_theme_is_written_unless_it_is_secret_or_the_file_is_out`).
- **A config that does not build still opens the window.** Launched from the
  Finder there is no terminal to print to, so the error is the window's
  content, painted in the first built-in. Nothing tests it.
- **The fonts are bundled.** Manrope (the page) and JetBrains Mono (key
  badges, tag sources, install lines and the log) come from `@fontsource` into the build, and
  `bundle.resources` puts both OFL licences in the app; a desktop app that
  fetched its type would draw in a fallback offline. Nothing tests it; the
  bundle was checked to carry both fonts and both licences.
- **A status's colour is core's `theme::status_slot`.** The terminal's
  `status_color` reads the same table, and the page gets the slot name with
  each status's word at start, which is also the CSS variable to paint it in.
  Colour is confidence, so a copy that drifted would put an unconfirmed tag
  in a confirmed colour. The slot is a name so CSS can read it, which means a
  misspelt one compiles (`every_status_is_painted_in_a_slot_the_palette_has`).
- **Text is typed into real inputs.** The prompts, the URL field and the
  filter box are `<input>`s, so the caret, selection and paste are the
  platform's; the terminal's caret code exists because a terminal has none.
  The keys the terminal adds on top (`^u` clears the focused box, `^s` swaps
  artist and title by label, a digit answers a choice only when that option
  is there) are the dialog's own (`answers a choice by digit only when that
  option is there`).
- **The first URL prompt is the first screen, not a dialog.** With nothing in
  the library the worker's first question is the link, so the field is the
  screen and `--check`'s tools are under it. A refused link comes back as the
  next question with the reason as its header. The page tells a refusal from
  the opening ask by having sent a link already, not by the value, since an
  empty link comes back empty. The format question asked straight after is a
  dialog (`makes the link question the whole screen only while there is
  nothing in the library`).
- **One filter, two screens, cleared by a new run.** The box over the
  library's wall narrows folders and the one over a folder's tracks narrows
  tracks, through the one
  `ui.filter`, as the terminal's box does. `Restart` clears it, for the
  reason `threads-and-messages.md` gives (`clears the page's own narrowing
  when a new run starts`).
- **The platform keeps its shortcuts and a focused control keeps its keys.**
  ⌘c is copy, not `c`, which opened the cover question; a bare Shift or Ctrl
  is no keypress, where it dismissed the quit question before the Y of
  Shift+Y; and a clicked
  button never keeps the focus, or the next space pressed it again. Checked
  through `EARWORM_DRIVE`, since the reducer tests have no DOM.
- **An effect reads only what it depends on.** `app.model` changes on every
  message, so an effect reading through it re-ran per log line: the spinner's
  interval never fired, the list snapped back to the cursor while somebody
  scrolled it, and the whole track list crossed IPC for every line of yt-dlp
  output. Take the field into its own `$derived` first. The reverse bites too:
  a dependency read inside `untrack` is never followed, so the library asked
  for no covers in a built app, where nothing else re-ran the effect once the
  folders scrolled into view. Read what the effect depends on before
  `untrack`, and untrack only the call. The covers are pinned by `asks for a
  folder's cover when it scrolls into view after the library has drawn` in
  `Library.test.ts`, which mounts the screen in jsdom with the observers
  driven by hand; the rest was checked in the running app.
- **A thumbnail is asked for once, and again after an `Idle` that left it
  unanswered.** The screens ask whenever a reply lands, so without
  `thumbsAsked` the files still on their way went out again; a batch the
  worker cut short at `stop` ends at `Idle`, which releases what it never
  answered (`asks again for a thumbnail a cut-short batch never answered,
  once the worker is idle`).
- **Debug builds print a `probe` line per change.** The page reports the
  wall, the theme and the computed accent to stderr, which is how a check
  reads the window where screen capture is not allowed. Release builds
  compile the command to nothing. Nothing tests it.
- **Debug builds record a session, and the browser replays it.** With
  `EARWORM_SESSION=<file>` the debug app writes every message, the boot data
  and the tools on the way out (before the stops, which would end the
  version probes). `pnpm dev` opened in a browser, with
  `?file=<session>&upto=<n>`, mocks IPC and replays the first `n` messages;
  `quit`, `logs`, `mode` and `tokens` set the page's own state, `tokens`
  keeping look.css's own colours for comparing with
  `docs/mockups/ideas/6-wall-and-room-icons.html`. A question in the history is open only when it is
  the last message applied, since the page's answers are not in it. Covers
  load through Vite's `/@fs`, so the thumbnail cache is in `server.fs.allow`;
  a session recorded with another `XDG_CACHE_HOME` needs the preview started
  with the same one. Never in a build. Nothing tests it.
- **A check presses keys through `EARWORM_DRIVE`, never the system's
  keyboard.** Debug builds with it set run each line appended to that file
  as JavaScript in this window's page. Synthetic keystrokes go to whichever
  app has the focus, which on a machine somebody is using is not this one:
  the first exit check typed into a browser that way. Release builds compile
  it to nothing, like `probe`. Nothing tests it.
- **New releases, the picker's marks and achievements are core's too.** The
  page asks `mark_new` and `unread` as reducer effects, as it asks for the
  pick gate's marks; it keeps the picker's marks as the ordered list and
  core's `Marks` answers which rows are marked and what Enter changes, since
  matching marks on composed names is a rule; and `earned` says what is new,
  with the page deciding only when the screen is calm enough for the toast
  (`asks core which releases are new rather than marking them itself`).
- **Locked, the artist page lists and never offers a download.** `app.music`
  comes from `start` and every cheat answer; off, Enter on a release or the
  top tracks does nothing and says nothing, and no hint names a fetch, for
  the reason `discovery.md` gives (`locked, offers no download and Enter on
  a release or the top tracks sends nothing`).
- **A screen's refresh runs untracked.** `pickRefresh` reads `screens.pick`
  before it awaits and writes it after, so called from an effect that
  tracked it, the effect was its own trigger and the window spun at full
  CPU. An effect names what it depends on and calls the refresh inside
  `untrack`.
  Nothing tests it; checked in the running app.
- **A question can carry pictures and a path kind.** `Prompt::Choice.pictures`
  gives the cover menu's candidates as URLs, drawn as a grid, and
  `Prompt::Input.browse` marks a path question so the dialog offers the
  system's file dialog beside the box. The terminal ignores both
  (`every_row_maps_back_to_its_own_source`). The CSP allows `https:` images,
  since the cover hosts redirect between each other.
- **`busy` goes up before the send, not after it returns.** A quick command's
  `Idle` reached the page before the send call came back, so setting `busy`
  afterwards left it on for good and every screen gated on it went dead
  (`is not left busy when the worker finishes before the send returns`).
- **Effects that change shared state run one at a time, on the state as it
  is.** `Unread` and `MarkNew` each computed from a snapshot and wrote back
  whole, so a page read while the startup check answered wiped every other
  artist's new releases (`keeps new releases for other artists when a page is
  read while the check answers`).
- **An achievement toast dismissed by a key is recorded.** Whoever pressed the
  key saw it, as in the terminal; putting it back to wait made it reappear at
  once and swallow the next key, over and over while somebody typed.
  Nothing tests it; checked through `EARWORM_DRIVE`.
- **The icons are generated from `crates/gui/app-icon.svg`.** `pnpm tauri
  icon app-icon.svg` in `crates/gui` writes them; only the six
  `tauri.conf.json` lists are kept, and the Android, iOS and Windows store
  sizes it also writes are not. Edit the SVG and regenerate rather than
  editing a PNG, or the sizes drift apart. Nothing tests it.
- **The bundle is ad-hoc signed, never unsigned.** Apple Silicon will not
  run a binary with no signature, and bundling rewrites the app after the
  linker signed it, so `bundle.macOS.signingIdentity` is `"-"` and the
  bundler signs the finished `.app`. There is no Developer ID, so Gatekeeper
  still blocks a downloaded copy once; `docs/install.md` says how to get past
  it. The release workflow checks `codesign -dv` says `Signature=adhoc`
  before it uploads anything, so an unsigned `.dmg` never reaches the draft.
- **The update notice points at the release page, and is asked for off the
  main thread.** An unsigned app cannot verify an update, so there is no
  updater, and the terminal's `cargo install` hint is wrong for a `.dmg`.
  `latest` runs `update::available` on a blocking thread, since a stale cache
  asks GitHub and a sync command would hold the window. The shell answers
  once a session, since a reload boots the page again
  (`the_update_news_is_given_once_a_session_and_never_when_the_check_is_off`),
  and the page waits for room in the flash queue rather than being dropped
  from a full one at a busy start (`waits for room to say a newer release is
  out rather than dropping it`).
