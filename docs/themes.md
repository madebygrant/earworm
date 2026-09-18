# Themes

[Back to the README](../README.md)

Four palettes ship. `^t` walks them with the screen in front of you, which is how anyone picks a
theme, and writes the one you stop on back to the config. `theme` in the config and `--theme` on the
command line name one directly.

| Theme   | For                                                                     |
| ------- | ----------------------------------------------------------------------- |
| `warm`  | the default. Cream and gold on a warm near-black gradient               |
| `light` | terminals with a light background, where the other three are unreadable |
| `cool`  | slate and steel. Warm's structure with the warmth taken out             |
| `neon`  | magenta and cyan over violet-to-black                                   |

`^t` works on every screen, including the help overlay and a prompt: the point of walking the
palettes is seeing them on whatever is in front of you. It is `^t` rather than `t`, which is the
library's track search, and rather than `T`, which searches again for one track.

## Repainting one colour

Any slot can be repainted on top of whichever theme is named, so changing one colour does not mean
restating ten.

```toml
theme = "neon"

[colors]
cursor = "#00ff88"
```

| Slot      | What draws in it                                            |
| --------- | ----------------------------------------------------------- |
| `text`    | track titles and anything the eye should land on first      |
| `accent`  | chrome: borders, bars, the spinner, and the mode band's fill |
| `cursor`  | where you are, and a tag something confirmed                |
| `warn`    | something to look at but not an error                       |
| `error`   | a failure                                                   |
| `muted`   | sources, notes and hints                                    |
| `rule`    | the lines between panes                                     |
| `surface` | the flat tone popups are raised with                        |
| `unsure`  | identified, but nothing was found to identify it as         |
| `ink`     | text on a filled band, read against `accent`                |

A value that is not `#rrggbb`, or a slot nobody draws in, stops startup and names what it should
have been. A colour that merely measures badly starts anyway and says so once, because it is your
screen. One colour is named outright; several are counted, and `--check` has the list either way.

`^t` walks the base palette only, so what you are looking at after a press is a clean built-in. The
`[colors]` table stays in the file and is applied again on top of whatever `^t` landed on at the
next start, which is why the flash mentions it while a table is there.

## Measured, not eyeballed

Every readable colour clears WCAG 4.5:1 against both ends of its own gradient and against the tone
popups are raised with, and still clears 3:1 after a 256-colour terminal has quantised it. `ink` is
measured against `accent` instead, because the band is the only thing it is ever drawn on. The tests
run those numbers on every palette, so a theme cannot ship unmeasured — which is how the warm
theme's red turned out to have been failing its own claim at 3.73:1 since the first commit, and why
`light`'s `unsure` went back for a second shade.

Under `NO_COLOR` every colour collapses to the terminal's own and earworm stays usable: the status
words, the `!` on a missing file and the `♪ cliamp ▶` transport carry what colour was saying.

A theme changes colour, never layout. The tests assert the frame is identical cell for cell across
all four.
