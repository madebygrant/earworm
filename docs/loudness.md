# Loudness tags

Tracks from different channels and uploads come out at different volumes. With
`loudness = true`, earworm measures each track and writes how far it sits from
a reference level. Players that read those tags, which is most of them when
"normalise" or "ReplayGain" is on, then even the volume out. The audio itself
is never changed.

It is off by default, because it rewrites the tags of files already on disk.
Turn it on in the config:

```toml
loudness = true
```

`--no-loudness` switches it off for one run.

## What gets written

| Format | Tags | Reference |
| --- | --- | --- |
| opus | `R128_TRACK_GAIN` | -23 LUFS |
| everything else | `REPLAYGAIN_TRACK_GAIN`, `REPLAYGAIN_TRACK_PEAK` | -18 LUFS |

Opus players read the R128 tag and ignore ReplayGain, which is why opus gets a
different one. earworm measures with ffmpeg's `ebur128` filter, so ffmpeg has
to be installed, as it already must be.

Every sync does this, including for tracks already on disk. That is how a
folder you downloaded before turning it on gets filled in the next time you
sync it.

A track that already has a gain tag is left alone, so a second sync spends no
time measuring and a value another tool wrote stays. Delete the tag to have it
measured again. A file that is silent, or too short to measure, gets no tag and
a line in the log (`l`).

## Albums

A folder earworm treats as an album ([docs/album-or-playlist.md](album-or-playlist.md))
also gets an album gain, so the quiet track on a record stays quiet next to the
loud one. It is a length-weighted energy mean of the tracks' own loudness,
which is close to measuring the whole album but is not the same measurement.

The album gain is rewritten whenever it changes, so adding or removing a track
moves it on every file in the folder. opus has no album peak tag, so opus
albums get the gain only.

A playlist gets no album gain. Its tracks have nothing to do with each other.
