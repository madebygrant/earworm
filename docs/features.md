# Features

yt-dlp downloads the audio. earworm does the rest, which is what separates a
download from a music library. Everything here works the same in the
terminal and in the [desktop app](gui.md).

## Tags you can trust

- **Tagging that checks itself.** Artist and title are parsed out of the video
  title, then confirmed against AcoustID fingerprints, Deezer and Apple Music.
  A track that survives none of that is marked `guessed`, and earworm asks
  about it rather than writing a tag it can't back up.
  [How it decides](internals.md).
- **Cover art embedded at a sane size.** YouTube serves 2048² thumbnails for
  its auto-generated albums. earworm keeps them out of your files: what gets
  embedded is capped at 1000×1000.
- **Albums get one sleeve, playlists get twelve.** An official album release is
  recognised on sight, and anything else you settle by putting `[album]` or
  `[playlist]` in the folder's name. earworm reads that marker and never
  writes one. [Details](album-or-playlist.md).
- **Loudness and lyrics.** ReplayGain and R128 tags so tracks play at one
  volume ([loudness](loudness.md)), and synced lyrics from LRCLIB
  ([lyrics](lyrics.md)).
- **Album uploads cut into tracks.** A one-video album with chapters becomes
  one file per song. [Details](chapters.md).

## A library that keeps up

- **Re-runs skip work you already did.** Each playlist folder keeps an
  `.earworm` manifest with its URL and current filenames. Sync it again and
  earworm fetches only the new tracks, reports the ones that left the
  playlist, and reads the tags it already wrote instead of looking them up.
- **Fix anything weeks later.** Every folder opens from disk, offline. Edit
  tags, swap artist and title, search across every folder you own, rename
  a playlist and have the rename survive the next sync.
  [Details](library.md).
- **Music earworm didn't download is welcome too.** Copy a folder of audio
  into your library and it appears with the rest, opens from its tags, and
  takes every editing key. Give it a URL later and earworm works out which
  files it already has before it fetches anything. [Details](found-folders.md).
- **A song on two playlists downloads once.** A video another folder already
  holds is copied across instead of fetched again, and the pick screen marks a
  likely duplicate with `≈`.
- **Change format without throwing away what you have.** Tracks already on
  disk convert on their next sync in whatever way costs the least re-encoding.
  Why some never re-encode at all is [documented](formats.md), because
  the answer surprised me too.
- **Retries and undos.** `r` re-downloads whatever failed. `u` puts back the
  last tag edit.

## Listening

- **Plays through [cliamp](https://docs.cliamp.stream).** `p` hands over the
  playlist in the right order, tags as track titles. [Details](playback.md).
- **Custom playlists.** Build a list from tracks in any of your folders with
  `N` (which opens a fuzzy track picker) and `P`, reorder it, and play it in
  cliamp. It follows a track when you edit it, and deleting a list never
  touches audio.
- **Export to a device.** `E` copies a folder or a custom playlist to a stick,
  card or synced folder, with names every filesystem accepts and a playlist
  file beside the audio. A second export copies only what changed, and nothing
  on the device is deleted unless you ask, and then every file is named first.
  It can re-encode for players that cannot read opus.
  [Details](library.md#exporting).
- **Artists, and what you're missing of them.** `b` lists every artist your
  tags name. Star the ones you follow, open an artist to see their
  discography marked owned, partial or missing, their top tracks and who's
  related, and hear about their new releases each time earworm starts.
  [Details](discovery.md).

## Looks

- **Five themes, measured rather than eyeballed, plus your own.** `^t` walks
  them with the screen in front of you and remembers the one you stop on.
  Every shipped colour clears WCAG 4.5:1 on its own background, including the
  light theme for terminals the others are unreadable on, and a
  [palette of your own](themes.md) gets the same numbers run over it.
- **Icons for the library.** `●` marks an album, `▤` a playlist and `◆` a
  folder earworm did not download. `icons = "nerd"` draws larger Nerd Font
  glyphs, and `icons = "text"` goes back to words.
