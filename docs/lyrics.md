# Lyrics

With `lyrics = true`, earworm looks each track up on [LRCLIB](https://lrclib.net),
a free lyrics database, and embeds the words in the file. Players that show
embedded lyrics then show them with no extra files beside the music.

It is off by default, because it makes a request to a third party for every
track and rewrites files already on disk. Turn it on in the config:

```toml
lyrics = true
```

`--no-lyrics` switches it off for one run.

## What is sent

The artist, the title, the album when the track has one, and the track's length
in seconds. Nothing else, and no account or key. earworm identifies itself as
`earworm/0.1`.

## What is written

The words go in the container's own lyrics frame: a `LYRICS` comment in opus,
flac and vorbis, `USLT` in mp3, `©lyr` in m4a and alac.

Plain text only. LRCLIB also holds time-synced lyrics, but a plain lyrics tag
shows the timestamps as text in any player that does not read them, so earworm
writes the words and drops the times. Where LRCLIB has synced lyrics and no
plain version, the timestamps are stripped.

## How a track is matched

LRCLIB's exact lookup needs an album, which most playlist tracks do not have,
so a track with no album goes straight to a search by artist and title. A
result counts only if its length is within two seconds of the track's. That is
what keeps the words of a live version or a single edit off the studio track.
A track whose length is unknown is skipped for the same reason.

An instrumental gets nothing written.

## Reruns

A track that already has lyrics is never looked up, and what is there is never
replaced, including lyrics you typed or another tool wrote. A track LRCLIB had
nothing for is asked about again on the next sync, since the database grows.
Delete a track's lyrics tag to have it fetched again.

The sync's closing line says how it went, for example `lyrics found for 9 of 12
searched`. Only tracks that were looked up count, so ones that already had
lyrics are left out. A miss is a count and not a failure: no track is flagged,
and nothing is marked wrong.

Every sync does this, including for tracks already on disk, so a folder
downloaded before you turned it on is filled in the next time you sync it.
A request that fails, for want of a connection or a track, writes nothing and
is not reported as a failure.
