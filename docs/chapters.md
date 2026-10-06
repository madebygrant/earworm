# Splitting a video by its chapters

Some uploads are a whole album in one video, with a chapter for each song.
Give earworm that video's link and it offers to cut it into one track per
chapter.

## The question

When a single video has two or more chapters and nothing has been downloaded
for it yet, earworm asks before it fetches anything:

```
This video has chapters
  Split into 5 tracks
  Keep as one track
```

It asks because a table of contents is not an album. Tutorials, podcasts and
streams carry timestamps too, and cutting those into tracks is the last thing
anyone wants by default. Esc keeps it as one track.

It never asks during `--resync`, with `--no-pick` or `--no-fix`, or for a video
whose file is already in the folder. Those keep the video whole.

## What you get

A folder named after the video, marked as an album, with one file per chapter:

```
TUNE & PLAY/
  01 - ABD.opus
  02 - SUN KISS.opus
  03 - Echo.opus
```

The full-length file is deleted once every chapter has a file of its own. Each
chapter then goes through the usual lookup and tagging, so it ends up named
`01 - Artist - Title.opus` like any other track. The artist is the video's, and
the title is the chapter's.

You still get the pick screen, so you can leave chapters out. A chapter you
skip is not cut, and the full-length file stays until all of them are.

Chapters that yt-dlp invents to fill a gap before the first one, such as the
second at the start of an album upload, are dropped when they are under ten
seconds. A longer untitled stretch is kept as `Untitled Chapter N`.

## Cutting

earworm downloads the video once and cuts it with ffmpeg. For opus, mp3, m4a and
vorbis the cut is a copy, so there is no second round of lossy encoding. flac and
alac are re-encoded, which is lossless. Every cut is checked against the
chapter's length, and one that comes out the wrong length counts as failed.

## Syncing again

A folder that holds a video's chapters is split again on the next sync, with no
question: the chapter entries in its `.earworm` are the record that it was
split. Tracks already there are left alone, including ones you renamed.

A chapter whose cut failed, or one you skipped, is cut on the next sync from the
full-length file, which was kept for that. So is the rest of a split that was
stopped part way: each chapter is recorded as it is cut, and the next sync
carries on from the full-length file instead of starting again. `r` does not retry a cut. It only
tags again, and says so in the log.

If the uploader changes the chapters, the tracks already cut keep their names
and any new ones are added. Chapters are matched by position, so a reordered
list will not follow.
