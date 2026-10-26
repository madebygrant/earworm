# Artists and discovery


- **An artist is their lead credit, cut only at "feat", "ft" or "featuring"
  with a name after it.** `&`, `,`, `/` and "with" are inside band names
  (AC/DC, Simon & Garfunkel, Nurse With Wound), and `split_artists` cutting
  there named artists who do not exist. So `lookup::lead_artist` and never
  `split_artists`, which stays for the downgrade guard. The name after is what
  keeps "Little Feat" whole. A guest verse is not one of the guest's albums,
  which is why "A feat. B" counts towards A only
  (`a_band_name_with_an_ampersand_slash_comma_or_with_is_kept_whole`,
  `a_featured_credit_counts_towards_the_lead_artist_only`,
  `a_band_ending_in_feat_is_kept_and_featuring_is_a_credit`).
- **`artists::key` is the one identity, and a name of punctuation keeps
  one.** It is `lookup::normalise`, so "Björk" and "bjork" are one row, one
  star and one query match. `normalise` keeps only alphanumerics, so "!!!"
  became the empty key and matched every other such name; the fallback is the
  trimmed lowercase name
  (`case_and_accents_do_not_split_an_artist_and_the_usual_spelling_is_shown`,
  `a_name_of_punctuation_alone_is_an_artist_and_not_every_such_artist`,
  `an_artist_query_without_accents_finds_the_merged_row`). It merges "M.I.A."
  with "Mia": a rare clash, taken to keep accents and case from splitting
  artists, and the Deezer id is what tells two such acts apart.
- **The index counts a recording once and never counts a playlist as an
  album.** A song on three playlists is a copy in each folder, so a copy known
  by video id or ISRC adds its folder and not a track; a `~` id is only unique
  in its folder and is never compared across them. With `album` on, a track the
  lookup found no album for is tagged with the playlist's name, so an album tag
  equal to a non-album folder's name is dropped. Albums group on `album_key`,
  so a remaster is not a second album
  (`a_song_copied_into_two_playlists_is_counted_once`,
  `a_playlists_own_name_in_the_album_tag_is_not_an_album`,
  `editions_of_one_record_are_one_album`).
- **The index is read off the tags with no cover and no properties, once.**
  `tag::brief` skips both, which is most of a read. Measured on a 298-track
  library at about 250ms cold. It keeps each track's title and album, copies
  included, because reading them again from the artist's folders on every Tab
  re-read whole mixed playlists for one song in each; a few hundred bytes a
  track is cheaper. Folders and their sidecars come from one walk,
  `worker::shelves`, not `library` and a second `manifest::load`
  (`owned_titles_are_the_artists_tracks_on_that_album`).
- **`serve` keeps the index for the session, and `index_use` says when it
  goes.** Syncs, edits, deletes and renames all change tags, and a list of the
  commands that do is a list a new command would be left out of, so anything
  not named is `Drop`. `GetAlbum` and `GetTop` are `DropAfter`: they read it
  for the owned titles and drop it once the download is done, or a downloaded
  album still reads missing. `serve` only acts on the answer, before and after
  its match (`only_the_artist_screens_own_reads_keep_the_artist_index`).
- **Favourites live in `.artists` under `--dir`, never in the config.** The
  config is written only by `^t` and the format keys
  (`docs/rules/themes-and-colour.md`). A `#` line is a header from a later
  version and is written back, or an older build would erase it; a name
  starting `#` or `\` is written behind a `\`. Every read-modify-write holds
  `STORE`, because the new-release check writes from its own thread
  (`a_header_from_a_later_version_survives_a_star`,
  `a_name_starting_with_a_hash_or_backslash_round_trips_and_is_not_a_header`).
- **A star holds the name first and the Deezer id once a page resolves it.**
  Starring is offline. The id is what keeps a later session on the same act
  among several that share a name, so a resolve stores it on a cache hit too
  (`a_resolved_id_survives_a_reload_and_a_later_star`,
  `an_artist_starred_after_their_page_was_opened_still_gets_their_id`).
- **The artists screen has its own query, not the library's `filter`.**
  `snap` checks every cursor against `filter`, so one shared box moved the
  library cursor to a folder matching an artist query
  (`an_artist_query_leaves_the_librarys_filter_and_cursor_alone`,
  `an_artist_query_that_matches_nothing_is_named_in_the_pane`).
- **`o` narrows the library by folder, never by text.** Filenames are
  rewritten by `clean` (AC/DC is `AC-DC`) and "Can" is inside "Scandal".
  `by_artist` holds the folders the index read the artist's tags in; it is a
  narrowing like `only`, cleared with the others and named in the band
  (`o_on_an_artist_narrows_the_library_to_the_folders_their_tags_are_in`,
  `a_library_narrowed_to_an_artist_names_them_in_the_band`).
- **Deezer's artist is the most followed exact name.** Search returns every
  act with a similar name and three bands are called Can. A related artist is
  opened on the id Deezer gave, not a fresh search by name
  (`the_artist_is_the_most_followed_exact_name_and_never_a_lookalike`,
  `a_related_artist_resolves_to_the_id_it_came_with`).
- **A release matches a held album on `album_key` equality, never
  containment.** Containment made a self-titled "Can" own "Can Delay". Edition
  words are whole words, or "(Monolith)" lost its "mono", and a part that says
  live, acoustic, instrumental or demo is never an edition, or "(Live
  Version)" owned the studio album. A title of punctuation keeps a key, as an
  artist's does, or Sigur Rós's "()" matched every album-less track
  (`deluxe_and_remaster_suffixes_on_either_side_still_match`,
  `a_self_titled_album_matches_only_itself`,
  `a_live_album_is_not_the_studio_album`,
  `an_edition_word_inside_another_word_is_not_an_edition`,
  `an_album_titled_in_punctuation_keeps_a_key_of_its_own`).
- **Partial needs a length, and an unknown length is not whole.** Deezer's
  album list has no track count, so one request per held release asks for it,
  off the worker thread in `count_later`, which kept the keys dead after Esc
  when it ran inline. A held release with no length reads `● n here` and can
  still be fetched (`a_held_release_with_no_count_can_still_be_fetched`,
  `a_count_fills_its_release_on_the_open_page_only`).
- **Every fixed header word yields to a message.** The library, search, list
  and artist screens drew their own word and hid what their keys said, so a
  refusal read as a dead key
  (`a_message_from_a_key_shows_in_the_header_over_the_screens_own_word`).
- **Fetching anything from the artist page needs the YouTube Music unlock,
  checked three times, and is a secret until then.** Locked, Enter on a
  release or the top tracks does nothing and says nothing, and neither the
  status bar nor the help mentions getting or stopping a search: the unlock is
  a cheat code, and a refusal naming it gives it away. This is the one place
  the "a silent key reads as broken" rule in `docs/rules/keys-and-screens.md`
  gives way. The worker checks before it searches, and the URL goes
  through `settle`, which is why an album's link keeps the
  `music.youtube.com` host (`settle` only refuses that host). `README.md` and
  `docs/keys.md` stay quiet about it too; `docs/discovery.md` is the one place
  that explains it
  (`enter_on_a_missing_album_asks_the_worker_only_once_unlocked`,
  `a_locked_page_lists_and_never_offers_a_download`,
  `a_locked_album_says_so_and_never_searches`,
  `locked_top_tracks_are_never_searched`).
- **An album is found by search then resolve, and must be by the artist.**
  The `#albums` search lists `MPREb_` ids with no title, and each resolve gives
  the `OLAK5uy_` playlist, its title and its first track's channel. The title
  has to match on `album_key`, and the channel has to credit the artist by a
  whole name (`lookup::credits`), or another act's "Greatest Hits" is
  downloaded. Containment let "Candlemass" stand for Can. A channel of `NA` is
  not held against it. Measured hit rate and timing are in docs/history.md
  (`an_album_line_matches_the_release_and_keeps_the_music_host`,
  `an_album_by_another_artist_is_not_the_one_asked_for`,
  `a_credit_is_a_whole_name_and_never_a_name_inside_another`,
  `the_album_search_resolves_candidates_until_one_is_the_record`).
- **A failed search is not "no such album".** yt-dlp's last stderr line goes
  to the log and the flash points at `l`
  (`a_failed_album_search_says_so_and_logs_why`).
- **Nothing downloads without a confirm naming what was found.** Title,
  channel and track count, because a live album or a fan upload can match
  (`an_unlocked_album_is_confirmed_before_it_is_settled`).
- **A found name is cleaned like an imported one.** The album title comes
  from YouTube and the top-tracks name from the tags, and `folder_field` only
  escapes `%`, so "AC/DC - Top Tracks" nested a folder in `AC` and a title
  starting `../` wrote outside `--dir`. Both go through `import::folder_name`
  (`found_names_become_one_folder_inside_the_library`).
- **Esc on the page stops a search, through `App.stop`, not `cancel`.**
  `cancel` is the quit, and the pipeline reads it as one. The UI clears `stop`
  as it sends the command, so an Esc cannot land between a worker-side clear
  and the search. The check runs between yt-dlp calls, and once more after the
  last, or a stop during it fell through to "no such album" or to the confirm
  (`a_stopped_search_downloads_nothing_and_says_so`,
  `esc_during_a_search_stops_it_and_then_leaves`).
- **The search calls pass `--no-warnings`.** `run_bounded` reads the pipes only
  once the child exits, so stderr filling its buffer would stall yt-dlp until
  the 30-second limit and read as "did not answer". Not tested.
- **A new album's folder is the clean title, and never another playlist's.**
  The `Link` name comes from the resolved title, not yt-dlp's `Album - Title`,
  and `import_folder` moves it to `Title (2)` rather than take over a folder
  with another URL. A folder already holding this URL is this playlist and is
  synced, so fetching an album twice does not make a second folder. That is also why a partial album lands in its own folder:
  the one holding the rest is usually another playlist, and syncing into it
  would rewrite its `#url`
  (`an_unlocked_album_is_confirmed_before_it_is_settled` pins the name,
  `an_import_never_takes_over_a_folder_it_did_not_make` the `(2)` and the
  reuse).
- **A partial album's gate opens with what is here unmarked, from the first
  frame.** The gate is forced on. `Msg::PickHeld` is sent before `Msg::Pick`,
  not after, or a key typed ahead answered the gate with everything marked.
  Matching is `title_keys`, as twins match, so "Song (Extended Version)" stays
  marked
  (`the_pick_gate_opens_with_owned_tracks_unmarked`,
  `owned_titles_are_found_among_the_tracks_to_download`).
- **Top tracks download as a playlist folder, not a custom playlist.** A
  custom playlist holds ids of files already on disk, which a missing track is
  not. Each title is one `#songs` search, which gives no artist, so each title
  match is read once more for its channel (about three seconds) and a cover is
  passed over. A song Deezer lists twice is one id. A second fetch puts the
  folder's earlier ids in the queue too, or the sync would call every track
  already there departed and offer it to `D`. The queue goes through `settle`
  like an album's
  (`a_song_by_somebody_else_is_passed_over`,
  `top_tracks_found_become_one_queue_named_for_the_artist`,
  `a_second_top_tracks_fetch_keeps_the_first_ones_in_its_queue`,
  `enter_on_top_tracks_sends_only_the_missing_and_only_once_unlocked`).
- **A top track is here on `song_key`, which drops mastering and nothing
  else.** "(Mono)" and "Remaster" are the song; "(Extended Version)" and
  "(Live)" are other recordings
  (`a_song_key_drops_mastering_and_keeps_other_versions`).
- **Tab asks for each list once and works while busy.** Switching lists needs
  no worker, so it is not gated on `busy`; `top_asked` and `related_asked`
  stop a fast Tab from queueing duplicates, the problem
  `docs/rules/threads-and-messages.md` describes for `busy`. An answer clears
  the flag, so a list that failed is asked for again, as its message says
  (`tab_switches_lists_while_the_worker_is_busy`,
  `tab_asks_for_top_tracks_and_related_once_each`,
  `tab_asks_again_for_a_list_that_failed`).
- **New releases are release ids not seen, never dates.** A "since" date
  missed releases Deezer adds after their date and reported a pre-order on
  every launch until its day. `.artists-seen` holds the ids per artist. An
  artist's first look only records what is out. A release stays new until its
  artist's page is opened, which marks it `✦ new` and records it seen; a
  pre-order waits for its date. The check works from the seen file as it was
  when it started, so a page read while it runs is kept out of its answer by
  `pages_read`, or those releases came back as new
  (`a_new_release_is_one_not_seen_and_already_out`,
  `opening_a_starred_artists_page_marks_what_is_out_as_seen`,
  `a_new_release_is_marked_on_its_page_and_then_no_longer_counted`,
  `a_late_check_does_not_bring_back_a_page_already_read`).
- **The startup check runs once per launch, when `serve` starts, on every
  starred artist, off the worker, and silent with nobody starred.** A cap left the rest unlooked at for good once the old
  date moved on. It pauses between artists so a page opened meanwhile gets its
  requests in, since both share Deezer's rate gate. With nothing starred it
  writes nothing, or every test `serve` and every unused `--dir` gained a file
  (`the_startup_check_writes_nothing_with_nobody_starred`). The network path
  of the check has no test.
- **Not covered:** a pty run that unlocks YouTube Music. The real code is a
  hash, so the download paths were checked by hand against a stub `yt-dlp` on
  `PATH`, and the screens through `TestBackend`.
