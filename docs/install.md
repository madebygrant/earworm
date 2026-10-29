# Installing earworm

## Tools

Four binaries, two of them optional. earworm looks for each on PATH, then in
`/opt/homebrew/bin`, `/usr/local/bin` and `~/.local/bin`, which is where a
desktop app started from Finder finds them without your shell's PATH. One
somewhere else can be named in the config:

```toml
[tools]
ffmpeg = "~/bin/ffmpeg-7/ffmpeg"
```

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

## The desktop app

Each [release](https://github.com/madebygrant/earworm/releases) has a `.dmg`
for Apple Silicon Macs, and an AppImage and a `.deb` for Linux. The app uses
the same tools, config and library as the terminal one, so install the tools
above first.

### First launch on macOS

The app is not signed with an Apple Developer ID or notarised, so macOS
blocks it the first time you open it. On macOS 15 and later, right-clicking
and choosing Open no longer gets past this. Use one of these instead:

- Open it once, dismiss the warning, then go to System Settings > Privacy &
  Security and click Open Anyway next to earworm.
- Or clear the download flag macOS put on it, from a terminal:

  ```sh
  xattr -dr com.apple.quarantine /Applications/earworm.app
  ```

Either way you only do it once. The app has no auto-updater, because
updating itself would need a signed update. When a newer release is out,
the app says so once, naming the releases page. Turn that off with
`update_check = false`.

### Linux

The AppImage runs as it is: `chmod +x earworm_*.AppImage` and start it. The
`.deb` installs with `sudo apt install ./earworm_*.deb`, which brings in
WebKitGTK. Tools installed with Homebrew on Linux are found in
`/home/linuxbrew/.linuxbrew/bin` without your shell's PATH, as on a Mac.
Opening a folder in the file manager goes through `xdg-open`.

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
