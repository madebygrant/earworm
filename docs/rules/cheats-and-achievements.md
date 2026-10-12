# Cheat codes and achievements

## Cheat codes

- **A cheat code unlocks a feature that is off by default, for one session.**
  `cheats.rs` holds the codes, the features they gate and `Unlocked`; no
  unlock is written to disk. Finding one earns an achievement, and that is
  remembered by `achievements.rs`, which is a separate file.
- **A code is stored as its FNV-1a hash, never as the word.** A plain string
  shows up in `grep` and in `strings` on the binary, and so does any name
  built from it, so the popup and its identifiers don't say it either. The
  tests type a `cfg(test)`-only code instead, which means no test can notice
  a wrong stored hash: check a new one by hand in the console. `digest` is
  pinned to published vectors, because changing it breaks every code at once
  while the suite stays green.
- **A gated link is refused in `prompt_url`, the one place a URL is
  added.** One from the command line goes to that prompt already typed
  rather than failing, since a start that fails cannot be unlocked. Folders
  that already carry such a URL keep syncing, because `resync` and `S` never
  ask it.
- **The unlock is read off each answer, not when the prompt opens.** The
  console is used while the refusing prompt is still on screen, and a check
  taken once would refuse the same link again straight after the unlock.
- **`Unlocked` is atomics on an `Arc`, not a `Cmd`.** When the console is
  used the worker is blocked on a prompt's reply and nothing reads the
  command channel. It is shared state beside the two channels, like
  `cancel`, and never blocks.
- **The console opens on two keys in a row, never one.** Each key alone is
  somebody's quit reflex, and a single key opened the console on exactly
  the person trying to leave, showing them a secret. Any other key, or a
  pause, disarms it.
- **The console shortcut sits ahead of the prompts, and the console then
  takes every key.** Its text is its own rather than a box in `App.fields`, which
  the prompt underneath owns and must get back intact.
- **An unlock re-heads the prompt behind it.** The worker only sets the
  header on the next answer, so it would otherwise contradict the unlock.
  The header string lives in `cheats.rs` because both threads need it.
- **The celebration keeps its words over its art on a short terminal,**
  since `popup` clips from the bottom and the words are what say something
  changed.
- **A secret palette lives in `theme::SECRETS`, never `BUILT_INS`, and is
  never saved.** `BUILT_INS` is the walk before any code is typed. A saved
  name would stop the next start, because the config would name a theme
  nothing has unlocked yet, so `cycle_theme` says "this session only"
  instead. The measurement tests chain `SECRETS` onto `BUILT_INS`, so a
  secret palette is held to the same contrast and layout checks. Each one
  has its own code, and `Feature::theme` is what ties a code to a palette:
  unlocking one never adds the others.
- **`grant` reports only the call that turned a feature on,** so a repeated
  code is acknowledged rather than celebrated again.

## Achievements

- **An id is a file format.** `Achievement::id` strings are what the state
  file holds, so renaming one announces it again and orphans its old line.
  `ids_are_pinned_and_round_trip` spells them out for that reason.
- **The file is state, not cache.** It lives under `XDG_STATE_HOME`, because
  a cache is defined as safe to delete and deleting this re-announces every
  achievement. `config::xdg_home` is the one place the XDG-or-HOME fallback
  is written, shared with the update cache, and it returns `None` rather than
  a relative path: joined onto an empty `HOME` the file landed in whatever
  directory earworm was started from, which can be somebody's music folder.
  `None` turns the feature off for the session.
- **A write unions with the file, never rebuilds it from `ALL`.** Ids this
  build does not know belong to a newer one, and a downgrade must not eat
  them. Only `NotFound` means empty: any other read error writes nothing, so
  a glitch cannot replace earned history with one line, and `main` turns the
  feature off for the session rather than announce what cannot be recorded.
- **The write belongs to its own thread.** The worker does not read its
  channel during a run, which is exactly when a clean run is earned, and `^t`
  is already the UI thread's one documented file write. `main` drops the
  sender and waits up to a second, so one earned just before `q` still lands.
- **An id is recorded when the popup ends having been read, not when it
  starts.** A question can hide it a frame after it appears, and recording
  at the start then loses an achievement nobody saw. A dismissal or the
  popup running its course records it; hidden or closed with the app after
  `SEEN`, the same; hidden sooner, it goes back in `App.queue` for another
  chance. Quitting with one queued announces it next session, which is what
  "once ever" means. `App.reward` holding an id also keeps `achieve` from
  queueing it a second time.
- **The popup arrives without a keypress, so it waits for `calm()`.** A
  cheat popup follows Enter in the console; this can land while somebody is
  typing in a prompt, the filter, the pick or the console, over the intro or
  the help. It still swallows the key that dismisses it, as every overlay
  does, except `^c`: it arrives unasked, so quitting as one appears must not
  take two presses. A question that arrives hides it rather than letting it
  reappear when the question closes.
- **Everything earned at once is one popup.** A first start over a big library
  earns several in one check, and the flash queue's rule applies: merge, never
  drop. It lists three and counts the rest.
- **`App::new` leaves achievements off; only `main` turns them on.** On, every
  test that sends a library would find a popup eating its first key, the same
  trap `intro_done` documents.
- **"Clean sweep" needs real work.** A run counts only if it tagged a track
  (`Ok` or `Manual`): resyncing an unchanged folder leaves every track `Have`
  and nothing was done cleanly.
- **Statuses are this run's alone, so `achieve` runs before `Restart` clears
  them.** The word that says what a `Done` was is `resting`, not `stage`,
  which every flash overwrites, and `"opened"` is not a run.
- **The help section is the last thing to give way, and shows only when
  something is earned.** It needs the theme row, the legend and the tail all
  to have been kept: a bare `fits` would show it after the legend was
  dropped, since the block is shorter. A fresh run draws the help it always
  did, which is also what keeps the documented trap of adding help rows from
  breaking unrelated tests.
- **`--no-achievements` goes through `off`.** Off means no read, no writer
  thread, no popup and no help section; the file is left alone.
