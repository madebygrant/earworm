# Dependencies, `--check` and the update check

## Dependencies and --check

- **`DEPS` is the one table, and the README's requirements list restates it.**
  A binary's name is not always its package: `fpcalc` comes from
  `chromaprint`, which is exactly the lookup a bare "not found on PATH" leaves
  the reader to do by hand.
- **Only `ErrorKind::NotFound` earns an install line.** A permission error
  told to `brew install` sends the reader after the wrong problem, which is
  worse than the vague message it replaced.
- **`report` takes `Facts` and touches nothing.** Probing PATH and walking
  `--dir` happen in `main::check`, so the report is testable as the text it
  is rather than as whatever the machine running the tests has installed.
- **An optional tool missing is not a failed check.** `ready` is what sets
  the exit status and only required tools clear it, so `--check` in a script
  means "can earworm work", not "is everything installed". The row still
  carries a cross and what you lose without it.
- **`version_of` takes one word after the tool's own name.** ffmpeg follows
  its version with a copyright line and fpcalc with a build string, and three
  of the four restate their own name before the number.

## The update check

- **A probe that can nag must not reach a frame.** The GitHub probe runs on
  its own thread in `main` and answers through its own channel, drained with
  `try_recv` between frames; nothing in the render loop or the worker
  channel knows it exists. Its one answer per session is a flag, not
  `app.update.is_none()`: a probe that found nothing has *sent* nothing, so
  without the flag the disconnected channel is re-polled every frame.
- **The notice needs a surface that is not the intro.** `intro = false` is a
  setting and `update_check` is a separate one, so an intro nobody draws must
  not take update notifications with it: the probe otherwise ran, wrote its
  cache and threw the answer away. `main` says it once as a flash when the
  answer lands and the intro is not up to say it, rather than parking it on a
  bar it would nag from for the rest of the session.
- **`None` from `update::available` is three answers at once** — check off,
  cache stale and network dead, already current — so there is no false row
  in `--check` and no second line in the intro. Silence is the no-news
  answer, and adding a row would have to invent a question.
- **A stamp ahead of the clock is a clock that moved.** It counts as just
  written, because the cache is the only throttle there is: subtracting and
  giving up made the file unreadable instead, and every start went back to
  GitHub for as long as the clock stayed behind. `read_cache_at` and
  `write_cache_at` take the path so this is testable without setting
  `XDG_CACHE_HOME`, which in a threaded test runner is a process-wide change
  — the same trap the colour depth documents.
- **`hint_for` is kept apart from the build flag.** `option_env!` resolves at
  compile time, so only one branch exists in any given binary and a test of
  `hint()` alone passes by agreeing with however the machine was built.
- **The cache is the throttle.** Tag plus unix stamp in
  `~/.cache/earworm/latest`, MAX_AGE 24h; the network is touched only when
  the file is missing or old. `--check` is the one synchronous caller, and
  it is script-facing and bounded, which is why it may spend the probe
  timeout.
- **The version comes from `env!("CARGO_PKG_VERSION")` only.** `compare` is
  semver-lite and tolerant (`v` prefix, missing pieces, suffixes) because
  the tag on GitHub is whatever the release was tagged; the two never share
  a literal.
