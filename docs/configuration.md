# Configuring earworm

## The config file

`~/.config/earworm/config.toml`, or `$XDG_CONFIG_HOME/earworm/config.toml`.
Every key is optional; nothing is here you need.

```toml
dir = "~/Music/playlists"
format = "opus"    # or m4a, mp3, flac, vorbis, alac
convert = false    # bring tracks already on disk to `format` when they sync

parse = true       # split artist and title out of the video title
lookup = true      # confirm against AcoustID, Deezer and Apple
apple = true       # fall back to Apple Music when Deezer finds nothing
cover = true       # fetch cover art and embed it in the tracks
m3u8 = true        # write the playlist file
fix = true         # prompt when a track can't be resolved
rename = true      # rename files from the corrected tags
album = true       # fill an empty album tag with the playlist name
pick = true        # stop to choose tracks before downloading
intro = true       # the opening animation
notify = true      # ring the bell when a long run finishes
update_check = true # ask GitHub about a newer release, at most once a day

theme = "warm"     # or light, cool, neon — see docs/themes.md

[colors]           # repaints slots on top of `theme`; every key optional
cursor = "#00ff88"

extra = ["--sleep-requests", "1"]   # always passed to yt-dlp
acoustid_key = "..."                # or use ACOUSTID_API_KEY
```

## Flags

| Flag | Effect |
| --- | --- |
| `-d`, `--dir` | output directory (default `~/Music`) |
| `-f`, `--format` | opus, m4a, mp3, flac, vorbis or alac (default opus) |
| `--theme` | warm, light, cool or neon (default warm) |
| `--no-convert` | leave tracks already on disk in the format they have |
| `-P`, `--no-parse` | keep YouTube's own artist/track, skip title parsing |
| `-L`, `--no-lookup` | skip AcoustID/Deezer/Apple, keep parsed tags |
| `--no-apple` | skip the Apple Music fallback, Deezer only |
| `-C`, `--no-cover` | don't fetch or embed cover art |
| `-M`, `--no-m3u8` | don't write the playlist file |
| `-F`, `--no-fix` | never prompt; unresolved tracks keep parsed tags |
| `-R`, `--no-rename` | keep yt-dlp's filenames instead of renaming from tags |
| `-A`, `--no-album` | don't fill an empty album tag with the playlist name |
| `--no-pick` | download the whole playlist without stopping to choose |
| `--no-intro` | skip the opening animation |
| `--no-notify` | don't ring the bell when a long run finishes |
| `--no-update-check` | never ask GitHub about a newer release |
| `--resync` | sync every playlist already under `--dir` |
| `--list` | print the playlists already under `--dir` and exit |
| `--check` | check the tools, the key and the config, then exit |
| `--config PATH` | read this config file instead of the default one |
| `--no-config` | ignore the config file entirely |

`-L -F` is the quick one. Download and tag from titles, no network lookups, no
questions.

## Precedence, and one direction only

Flags beat the file, the file beats the defaults. Every flag is a negation, so
the command line can only switch something off. To run with a feature the file
disables, edit the file.

`extra` from the file comes before anything after `--` on the command line, so
a repeated yt-dlp option takes its command-line value.

An unknown key is a startup error, so a typo tells you rather than silently
changing nothing. A key in the file sits there in plain text, so
`chmod 600 ~/.config/earworm/config.toml` on a shared machine.

## Environment

- `ACOUSTID_API_KEY` overrides `acoustid_key` for one run.
- `NO_COLOR` set to anything non-empty drops colour, and the mode bands switch
  to reversed video so they stay visible.
- `COLORTERM=truecolor` says the terminal can do 24-bit colour; without it
  earworm assumes 256 and maps the palette to the nearest available, since a
  truecolor terminal shown 256 colours loses a little smoothness in the
  gradient while the other way round loses the text.

## The update check

Once a day earworm asks GitHub whether a newer version exists and says so on
the intro and in `--check`. Equal versions, older ones, network failures and
`--no-update-check` all say nothing. Updating is
`git pull && cargo install --path .`.
