# Playing through cliamp


- **`cliamp playlist import` refuses a name it already holds,** exit 1, without
  replacing or duplicating. So a refresh is delete-then-import, and the delete
  failing is the ordinary first-time case rather than an error worth reporting.
- **A custom list's cliamp key is the stem of its own file.** `playlists::key`
  is built from `file_for`, not the raw name: a list called `../Focus` used to
  hand cliamp the name of the folder Focus, and `p` replaced that folder's
  playlist. It also makes the colon rule below hold for lists
  (`a_lists_cliamp_key_stays_inside_the_store_whatever_it_is_called`).
- **The stored name is `earworm - <folder> (<tag>)`, never the bare folder
  name.** That delete is unconditional, so a bare name would destroy a
  same-named playlist the user built in cliamp by hand, and cliamp's store is
  flat, so two `--dir` roots holding a `Focus` would otherwise share one entry.
  An ASCII colon is rejected by cliamp as an invalid playlist name, which is
  why the separator is a dash.
- **`tag` is FNV-1a written out, not `DefaultHasher`,** whose output is
  explicitly not stable between Rust releases. A changed hash orphans every
  playlist already in cliamp under the old name, so the value is pinned in a
  test against an independent implementation.
- **Two timeouts, not one.** `load` and `--version` reach the socket and answer
  at once or never; importing reads the tags off every file and is genuinely
  slow, and killing it part-way would leave cliamp's store torn.
- **`player::installed()` is cached and `player::running()` must not be.** What
  is on PATH cannot change within a session; whether cliamp is up changes from
  another window while earworm watches, including while the finish menu sits
  open, which is why that menu re-probes on every pass.
- **The finish row and the `p` key answer the same question.** Both require
  `running`, not merely installed. They drifted apart once because `finish`
  runs before `serve` and so cannot read `App.player`, and the result was the
  same action hidden on one screen and offered on the next.
- **`space` is library-only.** On the track list it marks a row, and that is
  what `targets()` and every bulk command read, so the transport key had to go
  where `space` was still free. That is also the screen showing what cliamp is
  doing.
- **The stub tests set their own timeouts.** macOS charges for the first exec
  of a newly written executable, which is what every stub is, and borrowing
  cliamp's 2s `IPC` bound made them fail about one run in three. Those tests
  check argv and control flow, never latency.
- **cliamp names the track, never the playlist.** `status --json` carries
  `track.path` and no playlist name at all, so the library row is matched by
  the folder that path sits in. A radio stream's path is a URL, which is why
  only a leading slash counts as a folder.
- **`status --json` is parsed, not exit-code checked.** A stopped cliamp prints
  its "not running" line instead of JSON, so a failed parse is the same answer
  and one probe covers both whether it is up and what it is playing.
- **`serve` polls cliamp between commands rather than on a thread of its own.**
  `recv_timeout` is what makes that free, and `serve` idling is exactly when
  `p` is live: during a run the keys are dead, so a fresher answer would be one
  nobody could act on. `Watch` is what keeps that to one `Msg::Player` per
  change rather than one every three seconds for the life of the session.
- **A key that can only fail is worse than no key.** `p` is drawn and handled
  only while `can_play()`, and the status bar carries `♪ cliamp off` so its
  disappearance has a reason on screen rather than looking like a bug.
- **`cliamp load` needs a running instance** and exits 1 with "cliamp is not
  running" otherwise. That is not a failure: the import has already happened,
  so `player::load` reports where the tracks landed instead. earworm never
  launches cliamp, because cliamp is a TUI and starting one would mean handing
  over the terminal earworm owns.
- **The `.m3u8` is the input, never the folder.** It is earworm's own answer
  about what the playlist holds and in what order, departures already left out;
  `playlist create` over the directory would lose the order and sweep up
  `cover.jpg`.
- **`finish`'s menu rows carry their answers.** The cliamp row is only there
  when cliamp is, so a bare index would mean a different thing depending on
  what is installed.
