# Behind the scenes

## How a track gets its tags

1. **Parse.** Split the video title on its separator. A dash means
   artist-first, a pipe or bullet means artist-last.
2. **Fingerprint.** `fpcalc` plus AcoustID, accepted at score 0.8 or better.
3. **Search.** Deezer, then Apple Music when Deezer has nothing plausible,
   matched on normalised artist and title.
4. **Ask.** Anything still unresolved gets a prompt listing the candidates,
   unless `--no-fix`.

The status column tells the four routes apart; the words are listed in
[keys.md](keys.md#statuses).

## What it writes

earworm renames a track it identified confidently, or that you typed the tags
for, to `NN - Artist - Title.opus`. Tracks it was unsure about keep the name
yt-dlp gave them, because an `unsure` name baked into a file looks more
decided than it is. It never writes over a file that already exists.

Renaming would break the next run's already-downloaded check, which works by
predicting the filename. So `.earworm` maps each video id to the file it ended
up in, which also survives a video being retitled on YouTube, and records the
playlist URL and last sync time for the library to read. Delete a track and it
downloads again. Delete `.earworm` and every renamed track downloads again.

The `.m3u8` names its tracks by filename, never by full path, and sits in the
same folder as them, so copying that folder to a phone or another machine
plays. Any sync or tag edit rewrites it.

Re-running is cheap. earworm skips ids it already has and reads its own tags
off the disk instead of looking them up again.

A file whose video has left the playlist gets a `gone` row. earworm never
deletes it on its own: it keeps remembering it in `.earworm` so a video that
comes back doesn't download twice, and leaves it out of the `.m3u8`, which
therefore matches the playlist rather than the folder. `D` deletes those
files, after a question that names every one.

That only works when the sync you just ran asked for the whole playlist and
got it. earworm stays quiet about departures when yt-dlp didn't finish
listing, or when you passed extra yt-dlp arguments: `--playlist-items 4` makes
every other track absent from the listing while it sits in the playlist
untouched, and yt-dlp exits 0 either way. Open a folder from the library, see
`gone` rows, and `D` isn't there until you press `S`.

The album tag gets the playlist name only when the lookup found no real album,
since the release a track actually came from is the better answer.

On exit earworm prints the folder it wrote to.

For the decisions behind format conversion, see
[format-conversion.md](format-conversion.md) — the design record, written when
the feature was built and kept as the reasoning.
