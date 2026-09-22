# Installing earworm

## Tools

Four binaries on PATH, two of them optional.

```sh
brew install yt-dlp ffmpeg chromaprint
```

| Binary | Homebrew formula | Used for |
| --- | --- | --- |
| `yt-dlp` | `yt-dlp` | downloading and extracting audio |
| `ffmpeg` | `ffmpeg` | conversion, cover art, resizing |
| `fpcalc` | `chromaprint` | acoustic fingerprinting (optional) |
| `cliamp` | `cliamp` | playing a finished playlist (optional) |

Then earworm itself:

```sh
cargo install --git https://github.com/madebygrant/earworm --tag v0.4.0
```

Drop the `--tag` to build from the newest code instead, and see
[releases](https://github.com/madebygrant/earworm/releases) for the current
version.

## Fingerprinting wants a key

Free, one line, from
[acoustid.org/new-application](https://acoustid.org/new-application):

```sh
export ACOUSTID_API_KEY=your-key
```

Without one, lookups fall back to Deezer and then to Apple Music, which match
a lot less well. Nothing else changes.

## Check the machine

`earworm --check` answers the requirements table for the machine you're on,
plus the config it reads and the format the next run would write. Paste it
into a bug report.

```
earworm 0.2.0

tools
  ✓ yt-dlp     2026.08.19
  ✓ ffmpeg     9.0.1
  ✗ fpcalc     not found · brew install chromaprint · no acoustic fingerprinting
  ✓ cliamp     v2.1.0

settings
  ✓ config     ~/.config/earworm/config.toml
  ✗ acoustid   no key · set ACOUSTID_API_KEY · lookups fall back to Deezer and Apple
  ✓ format     opus · writes .opus
  ✓ convert    off · tracks already on disk keep their format
  ✓ directory  ~/Music · 7 playlists
  ✓ hidden     deep-focus-chillstep  ·  on disk, on no screen, and passed over by --resync
```

The `hidden` row is only there when something is. `D`'s hide answer takes a
folder off the library and out of `--resync`, and this row is the only place
that says which folders those are.

It exits 1 when something earworm can't work without is missing and 0 when
only the optional tools are, so a script can act on it.
