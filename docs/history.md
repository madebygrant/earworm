# What proved it

Evidence behind rules in CLAUDE.md, and trade-offs taken with eyes open. It
lives here rather than there because CLAUDE.md is read at the start of every
session and this is read when somebody doubts a rule, which is rarer and worth
the extra click. Nothing here is a rule. Every rule stayed where it was.

## Decomposed filenames in the `.m3u8`

Confirmed in VLC on iOS with one Korean track listed four ways in one file:
decomposed plays, composed does not, and percent-encoding changes neither,
which is what rules out the `.m3u8` URI rules as the cause. APFS ignores the
difference on lookup, so nothing about this reproduces on the machine that
writes the file.

The cost is the opposite case. On a byte-exact filesystem holding composed
files, Linux or Android, decomposed is the wrong form and the entries name
nothing there. Chosen rather than overlooked, on one user who is on Apple
hardware, and the point at which this becomes a setting rather than a
constant.

## The half-converted folder

Reproduced on a real folder: six of twelve converted, six dead entries, six
departures.

## `lookup::stop` and the latch

Verified by hand, like `ytdlp::stop`, which has no test either: 417ms and a
SIGTERM against a 60s bound, and the retry refused in microseconds.

## `counts()` and the hand-kept `TALLY`

The first attempt kept a hand-kept `TALLY` array and read it from `counts()`,
which still compiled with a variant missing and still dropped a whole column.
Proved by adding one.

## Measuring every palette

The first version of `theme()` ran `unreadable()` only on the `[colors]` path,
so a `[themes.*]` palette, just as unmeasurable by the suite, could be
illegible with nothing said.

## `save_key` and the first table

Shipped with `[colors]`, and found by a round-trip test written for something
else.

## The conversion table

The first version of it had `opus` and `m4a` both wrong, on the assumption
that a container YouTube serves is a container yt-dlp picks. `yt-dlp -f
bestaudio --print %(acodec)s` is what settled it.
