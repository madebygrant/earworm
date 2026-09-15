# Playing it through cliamp

With [cliamp](https://docs.cliamp.stream) running, `p` hands it the folder's
`.m3u8`. It works from the library too, so you can play a folder without
opening it.

earworm imports the playlist file rather than the folder, because that file is
its own answer about what's in the playlist and in what order, departures
already left out. cliamp reads the tags earworm wrote, so tracks show up as
`Caravan - Thelonious Monk` rather than as filenames.

Playlists land in cliamp as `earworm - <folder> (<tag>)`. A folder called
`Focus` would otherwise destroy a `Focus` playlist you'd built in cliamp by
hand, and the tag is a hash of the folder's path, so two `--dir` roots that
both hold a `Focus` get a playlist each. A re-sync refreshes the copy rather
than failing on a stale one.

## Transport

`space` on the library is play/pause, which cliamp applies to whatever it has
loaded, earworm's or not. The folder cliamp is playing is marked `▶`, or `⏸`
when loaded but stopped. When cliamp has a track loaded the bar names it:
`♪ cliamp ⏸  Vogel im Käfig`. The row says which playlist is on air, this says
which song.

earworm talks to a running cliamp over a socket and never launches it, so it
never takes the terminal. `♪ cliamp off` means installed but stopped, and `p`
is withdrawn rather than left there to fail. earworm re-checks every few
seconds while idle, so starting cliamp in another window brings the key back.
