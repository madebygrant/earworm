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
  started in `setup` sends the library before anything listens and the rail
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
  else clears it, so one Esc on a search would end every later one at once.
- **The reducer is tested against what the worker sends.** `msgs.json` is
  written from `protocol::test_support::every_msg`, the samples the JSON
  round trip uses, and `the_recorded_messages_are_every_message_the_worker_sends`
  fails when it is stale (`EARWORM_RECORD=1 cargo test -p earworm-gui`
  rewrites it). The reducer test then requires a handler for every recorded
  variant and no handler for one that does not exist, so a new `Msg` fails
  on both sides until the page handles it.
- **The palette is core's, sent as CSS variables.** The mockups' stylesheets
  are imported unchanged for layout and type; `paint` overwrites their
  colour variables with the palette `Config::build` resolved, so `theme`,
  `[themes.*]` and `[colors]` apply as they do in the terminal. The variables
  painted are exactly the ones tokens.css defines
  (`the_palette_paints_exactly_the_colours_the_stylesheets_use`), and whether
  a theme is light is read off its ground, not its name.
- **`^t` and the theme picker save through `config::keep_theme`, and say so
  through `Kept::said`.** The same calls the terminal's `^t` makes, so a secret theme is never written and
  `--no-config` keeps the file out; why this write skips the worker is in
  `themes-and-colour.md`.
- **A config that does not build still opens the window.** Launched from the
  Finder there is no terminal to print to, so the error is the window's
  content, painted in the first built-in.
- **The fonts are bundled.** JetBrains Mono comes from `@fontsource` into the
  build; a desktop app that fetched its type would draw in a fallback
  offline.
- **Debug builds print a `probe` line per change.** The page reports the
  rail, the theme and the computed accent to stderr, which is how a check
  reads the window where screen capture is not allowed. Release builds
  compile the command to nothing.
