# Wave plan: artist discovery

Written 2026-10-26 against `main` at `9879a72` (v0.6.0), clean tree.
Treat line numbers as hints and re-check them before acting.

On approval, this file is saved as
`docs/plans/2026-10-26-artist-discovery-plan.md`, next to the other wave plans.

The request: music and artist discovery, starting with favourite artists and
the albums they have.

**Already true, or found while planning:**

- `lookup.rs` already calls Deezer search with no API key, through one shared
  `ureq` agent and a rate gate (`lookup.rs:39-48`, `get_json` at `:111`).
  Deezer's free API also has `/artist/{id}`, `/artist/{id}/albums`,
  `/artist/{id}/related` and `/artist/{id}/top`, so every wave below runs
  without a new service or key.
- `lookup::normalise` and `lookup::similar` (`lookup.rs:516-523`) already
  compare names loosely, and `split_artists` (`:579`) splits "A feat. B".
  Matching Deezer albums against tags reuses them.
- Every track's artist and album are in its tags; `open_shelf` reads them with
  no network. There is no library-wide artist index yet.
  `library_index` (`worker.rs:1671`) indexes by video id, not by artist.
- `Cmd::Url` (`app.rs:427`) asks for a URL and syncs it into a new folder.
  Getting a missing album is that flow with the URL already filled in.
- `import.rs` turns a list of YouTube links into a playlist. Top tracks as a
  playlist (wave 5) can reuse it.
- Taken keys on the library screen include `a` (kind filter), `f` (format),
  `S`, `t`, `D`. `b` and `*` are free everywhere.
- Listening history isn't available: plays happen in cliamp. Anything
  "because you played X" is out of scope.

## 1. Artist index and storage decisions [done 2026-10-26]

Why first: every later wave reads the artist index and the favourites file,
and the store format is the expensive thing to change later.

- **Decision: what identifies an artist.** Recommended: the Deezer artist id,
  with the display name beside it. A name alone breaks on "The Beatles" vs
  "Beatles" and on two artists with one name.
  - The id is unknown until a lookup. Store the name on star and resolve the
    id lazily on first open, writing it back.
- **Decision: where favourites live.** Recommended: one file under `--dir`
  beside the custom playlist store (read `docs/rules/custom-playlists.md`
  first), not in the config. The config is written only by `^t` and the
  format keys, and the rules say why.
- Build `artists()` in the worker: walk the library tags once, group by
  primary artist (first of `split_artists`), count tracks and distinct
  albums per artist. Cached for the session, rebuilt on any sync.
  - Risk: a big library makes this slow the first time. Read tags with lofty
    only, no audio decode, and measure on the real library.
- Unit tests: grouping over "A feat. B", "A & B", case and accent variants.

Exit: `cargo test` passes with the new grouping tests, and breaking
`split_artists` use in the grouping makes at least one fail.

## 2. Artists screen

Why here: it's useful on its own with no network, and it's the entry point
every later wave hangs off.

- `View::Artists` in `app.rs`, reached by `b` from the library. Rows: name,
  track count, album count, a star for favourites. Sorted favourites first,
  then by track count.
- `*` stars or unstars the artist under the cursor. Also `*` on a track row
  stars its artist.
- `/` filters, using the same filter rules as the library
  (`docs/rules/lists-and-search.md`).
- Enter on an artist opens the library narrowed to their folders, reusing
  the `t` search path. Wave 3 replaces this with the artist page.
- `Cmd::Artists` / `Msg::Artists(Vec<ArtistRow>)`; the worker builds, the UI
  draws (`docs/rules/threads-and-messages.md`).
- `draw_artists` in `ui.rs`, with `TestBackend` render tests.
- Help and status-bar hints per `docs/rules/keys-and-screens.md`.
- Hold `artists::index` in a session cache, rebuilt on any sync (moved here
  from wave 1, which had no caller for it).
- Remove the module-wide `allow(dead_code)` at the top of `src/artists.rs`
  once the screen calls it.

Exit: a `TestBackend` test renders the screen from a built `App`, and a pty
run against a rebuilt fixture stars an artist and the favourites file appears.

## 3. Artist page and discography

Why now: this is the feature asked for. It needs waves 1 and 2.

- `src/discover.rs`: `artist_id(name)`, `albums(id)`, `related(id)`,
  `top(id)`, each a thin call through `lookup::get_json`, plus a pure
  `owned(albums, library) -> Vec<AlbumRow>` that marks each album owned,
  partial (n of m tracks) or missing.
  - Album match: `normalise` both titles, then strip "(Deluxe)",
    "(Remastered 2011)" and similar suffixes before `similar`.
  - Risk: false "missing" on retitled reissues. Show partial/missing as a
    hint, never act on it without a key press.
- Deezer's `record_type` gives album, EP, single, compilation. Default to
  albums and EPs; a key cycles to show singles too.
- `View::Artist`, opened by Enter on the artists screen. Header: artist name,
  fan count, your track count. List: year, title, type, owned mark.
  Esc returns to the artists screen at the same cursor.
- The worker fetches on open. Show a spinner row until `Msg::Discography`
  arrives; on no network, say so in one line and keep the screen usable.
- Cache responses in memory for the session so going back and forth costs
  nothing.
- Unit tests on `owned` with canned Deezer JSON: owned, partial, missing,
  deluxe and remaster variants, a self-titled album.

Exit: the `owned` tests pass and fail when the suffix stripping is removed;
the screen renders from canned data in `TestBackend`.

## 4. Getting a missing album

Why after 3: it acts on what wave 3 shows, and it's the first wave that
downloads anything.

- **Gate: the YouTube Music cheat.** The whole wave sits behind
  `Feature::Music` (`cheats.rs:8`), the same session unlock that lets
  `music.youtube.com` links through the URL prompt. Check
  `cfg.unlocked.has(Feature::Music)` in the worker, not only in the UI.
  - Locked: Enter on a missing or partial album says `cheats::LOCKED` in the
    status bar and does nothing else, and the artist page shows no "get"
    hint. Waves 1 to 3 stay fully usable.
  - Route the found URL through `settle` (`worker.rs:193`) so the existing
    `locked` check refuses it too, rather than a second copy of the check.
  - Test: with `Unlocked::default()`, the get command sends no download and
    the status says `LOCKED`; after `grant(Feature::Music)`, it proceeds.
    Break the gate and watch the test fail.
- **Investigation: finding the album on YouTube.** Options are YouTube
  Music's album playlist (`OLAK5uy_` ids) via `yt-dlp "ytsearch"` on
  music.youtube.com, or a plain search. Measure hit rate on ten albums from
  the real library before choosing. Record the result in docs/history.md.
- Enter on a missing album: the worker finds the playlist URL, then runs it
  through the same path as `Cmd::Url`, so the pick gate, tagging, covers and
  the manifest all behave as they do today.
  - Risk: the wrong album (live version, fan upload). Show the found
    playlist's title and track count in a `Confirm` before downloading
    (`docs/rules/questions-and-forms.md`).
- On a partial album, offer only the missing tracks, using the pick gate with
  the owned ones unmarked.
- `#kind` should come out as album; check `manifest::kind_of` agrees
  (`docs/rules/albums-and-playlists.md`).

Exit: a pty run against a rebuilt fixture, with a canned search result,
creates the new folder's `.earworm` with the found `#url`. Wait on the file,
not the screen.

## 5. Related artists, top tracks, new releases

Why last: each is a small extension of wave 3's page and none blocks another.
Any of them can slip.

- **Related.** A tab or key on the artist page lists Deezer's related
  artists with how many of their tracks you own. Enter opens their page.
- **Top tracks.** The artist's top 10, each marked owned or not; a key saves
  the missing ones as a custom playlist via the `import.rs` path
  (`docs/rules/custom-playlists.md`). Finding them on YouTube Music is
  behind the same `Feature::Music` gate as wave 4; locked, the list still
  shows.
- **New releases.** On start, check favourites for albums released since
  the last check, one request per artist through the rate gate, and show a
  count on the library header. Store the last-check date in the favourites
  file.
  - Risk: many favourites means many requests at start. Run in the worker
    after the library has drawn, and cap per session.

Exit: each part has its own `TestBackend` test, and new releases has a unit
test over canned dates.

## 6. Documentation

Why last: the docs describe how the earlier waves end up behaving.

- New `docs/rules/discovery.md`: the artist id decision, why favourites
  aren't in the config, why album matches never act on their own, why
  getting albums and top tracks needs the `Feature::Music` unlock, the
  YouTube lookup choice and its measured hit rate. Add its row to the
  CLAUDE.md table.
- README, `docs/keys.md` (`b`, `*`, the artist page keys), and a new
  `docs/discovery.md`.

Exit: every new rule names the test that pins it, or says nothing tests it.

## Why this order

Storage and the index come first because changing them later costs a
migration. The artists screen works offline and gives every later wave an
entry point. The discography is the requested feature. Downloading comes
after showing, because a wrong match should be visible before it costs a
download.

## What can slip

Wave 5 can slip entirely; waves 1 to 4 are the core. Wave 4's YouTube lookup
investigation can change its shape: if the hit rate is poor, it becomes
"copy a search link" instead of a download.
