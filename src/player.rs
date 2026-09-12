use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Result, bail};

use serde_json::Value;

use crate::app::{Playback, Player};
use crate::lookup;

const BIN: &str = "cliamp";

/// `load` and `--version` reach the running instance over a socket, so they
/// answer at once or never.
const IPC: Duration = Duration::from_secs(2);
/// Importing reads the tags off every file, which for a long playlist is
/// genuinely slow, and killing it part-way would leave cliamp's store torn.
const STORE: Duration = Duration::from_secs(30);
/// The opener hands off to a desktop process and returns; on a machine with
/// no session behind it, `xdg-open` is the one that sits there instead.
const OPEN: Duration = Duration::from_secs(5);

/// The `.m3u8` earworm writes beside a folder's tracks.
pub fn playlist_file(folder: &Path) -> PathBuf {
    let name = folder.file_name().unwrap_or_default().to_string_lossy();
    folder.join(format!("{name}.m3u8"))
}

/// FNV-1a, written out rather than taken from `DefaultHasher`, whose output is
/// explicitly not stable between releases: a changed hash would orphan every
/// playlist already sitting in cliamp under the old name.
fn tag(path: &Path) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in path.as_os_str().as_encoded_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{:04x}", hash & 0xffff)
}

/* Prefixed because importing deletes whatever already holds the name, and a
   bare folder name would reach a playlist the user built by hand. Suffixed
   because two `--dir` roots can both hold a `Focus`, and without the path in
   the name the second would quietly replace the first. An ASCII colon is the
   one separator cliamp rejects outright. */
fn stored(folder: &Path) -> String {
    let name = folder.file_name().unwrap_or_default().to_string_lossy();
    // Resolved so a symlinked and a direct route to one folder agree.
    let real = folder.canonicalize().unwrap_or_else(|_| folder.to_path_buf());
    format!("earworm - {name} ({})", tag(&real))
}

fn cliamp(bin: &str, limit: Duration, args: &[&str]) -> Option<(bool, String)> {
    let out = lookup::run_bounded(
        Command::new(bin)
            .args(args)
            // Or the child takes keystrokes out from under the event loop.
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
        limit,
    )?;
    // Failures go to stderr, and they are the half worth repeating.
    let text = if out.status.success() {
        String::from_utf8_lossy(&out.stdout)
    } else {
        String::from_utf8_lossy(&out.stderr)
    };
    Some((out.status.success(), text.trim().to_string()))
}

/// Whether `cliamp` is on PATH, which decides if the key and the finish-menu
/// row are offered. Resolved once: `finish` asks on every pass of its loop and
/// the answer cannot change within a session.
pub fn installed() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    *FOUND.get_or_init(|| cliamp(BIN, IPC, &["--version"]).is_some_and(|(ok, _)| ok))
}

/// What cliamp is doing. About 11ms, so cheap enough for the poll behind it.
pub fn probe() -> Playback {
    if !installed() {
        return Playback::default();
    }
    read_status(&cliamp(BIN, IPC, &["status", "--json"]).map_or_else(String::new, |(_, s)| s))
}

/* Kept apart from the process so the shape cliamp actually emits can be
   tested as text: `probe` reaches a socket, and a test of it would be a test
   of whether this machine happens to have cliamp up. */
fn read_status(text: &str) -> Playback {
    /* Parsed rather than exit-code checked: a stopped cliamp prints its "not
       running" line instead of JSON, so failing to parse is the same answer
       and one probe covers both questions. */
    let Ok(state) = serde_json::from_str::<Value>(text) else {
        return Playback {
            player: Player::Stopped,
            ..Playback::default()
        };
    };
    Playback {
        player: Player::Running,
        folder: folder_of(state.pointer("/track/path").and_then(Value::as_str)),
        /* cliamp's own title, which is the tags it read rather than the
           filename: for a track earworm wrote, that is the corrected name. */
        track: state
            .pointer("/track/title")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|t| !t.trim().is_empty()),
        playing: state.get("state").and_then(Value::as_str) == Some("playing"),
    }
}

/* cliamp names the track and never the playlist, so the folder is what says
   which library row is on air. A radio stream's path is a URL and belongs to
   no folder, which the leading slash is enough to tell apart. */
fn folder_of(path: Option<&str>) -> Option<PathBuf> {
    let path = path.filter(|p| p.starts_with('/'))?;
    Path::new(path).parent().map(Path::to_path_buf)
}

/// Play/pause, which cliamp applies to whatever it has loaded. Nothing is
/// reported back: `serve` re-probes the moment it finishes with a command, so
/// the indicator is the answer.
/* The platform's own opener, which is the only thing that knows what the
   user's file manager is. Not on the UI thread: it is a subprocess like every
   other, and `xdg-open` on a machine with no desktop session can sit there. */
pub fn reveal(folder: &Path) -> Result<String> {
    let Some(bin) = opener() else {
        bail!("no file manager to open this with  ·  the path is in the header");
    };
    reveal_with(bin, folder)
}

/// The platform's opener, or `None` where there is not one to name.
fn opener() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("open")
    } else if cfg!(target_os = "linux") {
        Some("xdg-open")
    } else {
        None
    }
}

/* Takes the binary, like the cliamp calls do: a test that ran the real one
   would open a window on the machine running the suite. */
fn reveal_with(bin: &str, folder: &Path) -> Result<String> {
    let name = folder
        .file_name()
        .map_or_else(|| folder.display().to_string(), |n| n.to_string_lossy().into());
    match lookup::run_bounded(
        Command::new(bin)
            .arg(folder)
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
        OPEN,
    ) {
        None => bail!("{bin} did not answer  ·  the path is in the header"),
        Some(out) if !out.status.success() => {
            let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let detail = if said.is_empty() { format!("{bin} refused it") } else { said };
            bail!("{detail}");
        }
        Some(_) => Ok(format!("opened {name} in the file manager")),
    }
}

pub fn toggle() -> Result<()> {
    toggle_with(BIN, IPC)
}

fn toggle_with(bin: &str, limit: Duration) -> Result<()> {
    match cliamp(bin, limit, &["toggle"]) {
        None => bail!("cliamp did not answer  ·  check it is still running"),
        Some((false, why)) => bail!("{why}"),
        Some((true, _)) => Ok(()),
    }
}

/// Whether a playlist can be handed over right now, for callers with no reason
/// to hold the rest of the snapshot.
pub fn running() -> bool {
    probe().player.ready()
}

/* The stored name carries both the folder's name and a hash of its path, so
   a rename leaves the old entry in cliamp's store for good: nothing else will
   ever name it again. Best effort, like the delete inside `load_with`: cliamp
   not being there is the ordinary case, not a failure worth reporting. */
pub fn forget(folder: &Path) {
    if !installed() {
        return;
    }
    let _ = cliamp(BIN, STORE, &["playlist", "delete", &stored(folder)]);
}

pub fn load(folder: &Path) -> Result<String> {
    load_with(BIN, STORE, IPC, folder)
}

/* `import` refuses a name it already holds rather than replacing it, so a
   second sync of the same playlist would fail on an unchanged copy. Deleting
   first is what makes this a refresh, and the delete failing is the ordinary
   first-time case rather than an error. */
fn load_with(bin: &str, store: Duration, ipc: Duration, folder: &Path) -> Result<String> {
    let playlist = playlist_file(folder);
    if !playlist.is_file() {
        bail!("no .m3u8 in this folder  ·  sync it again without --no-m3u8");
    }
    let Some(file) = playlist.to_str() else {
        bail!("playlist path is not valid UTF-8  ·  rename the folder and sync again");
    };
    let name = stored(folder);

    let _ = cliamp(bin, store, &["playlist", "delete", &name]);
    let imported = match cliamp(bin, store, &["playlist", "import", "--name", &name, file]) {
        None => bail!("cliamp did not answer  ·  check it is still running"),
        Some((false, why)) => bail!("{why}"),
        // cliamp counted the tracks it took, which is better than a guess.
        Some((true, said)) => said,
    };

    /* Not running is the ordinary case rather than a failure: the tracks are
       imported either way, so say where they landed. The stored name and not
       the folder's, since that is what to look for in cliamp. */
    match cliamp(bin, ipc, &["load", &name]) {
        Some((true, _)) => Ok(format!("playing \"{name}\" in cliamp")),
        _ if imported.is_empty() => Ok(format!("imported \"{name}\"; start cliamp to play it")),
        _ => Ok(format!("{imported} start cliamp to play it")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /* The stubs are executables written moments before they run, and macOS
       charges for the first exec of a new one. That has nothing to do with how
       long cliamp takes to answer, so these must not borrow its bounds. */
    const SLACK: Duration = Duration::from_secs(60);

    /// A stand-in `cliamp` that logs its argv and exits by the rules given as
    /// `case` lines, so the call sequence can be asserted without cliamp.
    /* The payload cliamp actually emits, kept verbatim: the folder says which
       library row is on air and the title is the only thing that says which
       song, which is the question anyone has of a player. */
    #[test]
    fn a_status_payload_gives_up_the_folder_the_title_and_the_transport() {
        let json = r#"{
          "ok": true,
          "state": "paused",
          "track": {
            "title": "Vogel im Käfig - Attack on Titan Jazz",
            "path": "/Users/me/Music/Attack on Titan/05 - Jazz - Vogel im Käfig.opus",
            "duration_secs": 457,
            "index": 4
          },
          "position": 54.781,
          "total": 16
        }"#;
        let now = read_status(json);
        assert_eq!(now.player, Player::Running);
        assert!(!now.playing, "paused was read as playing");
        assert_eq!(now.folder.as_deref(), Some(Path::new("/Users/me/Music/Attack on Titan")));
        assert_eq!(now.track.as_deref(), Some("Vogel im Käfig - Attack on Titan Jazz"));

        let playing = read_status(&json.replace("\"paused\"", "\"playing\""));
        assert!(playing.playing);

        /* A stopped cliamp prints its own line instead of JSON, and that is
           the same answer as a failed parse. */
        let stopped = read_status("cliamp is not running\n");
        assert_eq!(stopped.player, Player::Stopped);
        assert_eq!(stopped.track, None);

        // A stream has a URL for a path and no folder behind it.
        let stream = read_status(r#"{"state":"playing","track":{"title":"BBC 6","path":"https://stream"}}"#);
        assert_eq!(stream.folder, None);
        assert_eq!(stream.track.as_deref(), Some("BBC 6"));

        // Nothing loaded at all, which is not the same as nothing playing.
        let idle = read_status(r#"{"state":"stopped"}"#);
        assert_eq!(idle.player, Player::Running);
        assert_eq!(idle.track, None);
    }

    /* Driven through a stub, because running the real opener would put a
       window on the screen of whoever is running the suite. What matters is
       that the folder is what gets handed over, and that a refusal comes
       back as the opener's own reason rather than as silence. */
    #[test]
    fn revealing_a_folder_hands_it_to_the_platforms_opener() {
        let (bin, log) = stub("reveal", "exit 0");
        let folder = bin.parent().unwrap().join("Focus");
        std::fs::create_dir_all(&folder).unwrap();

        let said = reveal_with(&bin.display().to_string(), &folder).unwrap();
        // Pipe-delimited by the stub, since a path can carry spaces.
        assert_eq!(calls(&log), vec![format!("{}|", folder.display())]);
        // Named by the folder, since the whole path is already on screen.
        assert!(said.contains("opened Focus in the file manager"), "{said}");

        let (angry, _) = stub("revealfail", "echo 'no application knows this' >&2; exit 1");
        let err = reveal_with(&angry.display().to_string(), &folder)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no application knows this"), "{err}");

        /* Every platform earworm runs on has one; the message only exists so
           a third never silently does nothing. */
        assert_eq!(opener().is_some(), cfg!(any(target_os = "macos", target_os = "linux")));
        let _ = std::fs::remove_dir_all(bin.parent().unwrap());
    }

    /* Nothing on this path can be fixed by reading it twice: both messages
       have to name what to do about them. */
    #[test]
    fn the_handoff_errors_say_what_to_do_about_them() {
        let dir = std::env::temp_dir().join(format!("earworm-nom3u8-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Focus")).unwrap();
        let err = load_with("cliamp", IPC, IPC, &dir.join("Focus"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("sync it again without --no-m3u8"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn stub(tag: &str, body: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("earworm-player-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("cliamp");
        let log = dir.join("log");
        let mut file = std::fs::File::create(&bin).unwrap();
        // Pipe-delimited: the playlist name has spaces in it, so a
        // space-joined line cannot be split back into arguments.
        let log_line = format!("printf '%s|' \"$@\" >> \"{0}\"; echo >> \"{0}\"", log.display());
        write!(file, "#!/bin/sh\n{log_line}\n{body}\n").unwrap();
        drop(file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (bin, log)
    }

    /// A folder shaped the way earworm leaves one: tracks plus `<name>.m3u8`.
    fn folder(under: &Path) -> PathBuf {
        let at = under.join("Focus");
        std::fs::create_dir_all(&at).unwrap();
        std::fs::write(at.join("Focus.m3u8"), "#EXTM3U\n01 - A.opus\n").unwrap();
        at
    }

    fn calls(log: &Path) -> Vec<String> {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /* The delete has to come first and its failure has to be ignored: import
       refuses a name it already holds, and on a first run there is nothing to
       delete. Both halves look like mistakes on their own, which is exactly
       why they need pinning down. */
    #[test]
    fn refreshes_by_deleting_before_importing() {
        let (bin, log) = stub(
            "refresh",
            r#"case "$1 $2" in
  "playlist delete") echo 'no such playlist' >&2; exit 1 ;;
  "playlist import") echo 'Imported 24 tracks.'; exit 0 ;;
esac
exit 0"#,
        );
        let dir = bin.parent().unwrap().to_path_buf();
        let said = load_with(&bin.to_string_lossy(), SLACK, SLACK, &folder(&dir)).unwrap();

        let calls = calls(&log);
        assert_eq!(calls.len(), 3, "{calls:?}");
        let name = stored(&folder(&dir));
        assert_eq!(calls[0], format!("playlist|delete|{name}|"), "{calls:?}");
        assert!(calls[1].starts_with(&format!("playlist|import|--name|{name}|")), "{calls:?}");
        assert_eq!(calls[2], format!("load|{name}|"), "{calls:?}");
        assert!(said.contains("playing"), "{said}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /* The name is namespaced so the unconditional delete can only ever reach
       earworm's own copy: a bare folder name would destroy a playlist of the
       same name that the user built in cliamp by hand. */
    #[test]
    fn never_touches_a_playlist_under_the_bare_folder_name() {
        let (bin, log) = stub("namespace", "exit 0");
        let dir = bin.parent().unwrap().to_path_buf();
        load_with(&bin.to_string_lossy(), SLACK, SLACK, &folder(&dir)).unwrap();

        for call in calls(&log) {
            for field in call.split('|') {
                assert_ne!(field, "Focus", "cliamp was pointed at the bare name: {call}");
            }
            assert!(call.contains("earworm - Focus ("), "{call}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /* cliamp not running is not a failure: the import already happened, so the
       tracks are there and saying nothing worked would be wrong. */
    #[test]
    fn a_stopped_cliamp_still_reports_where_the_tracks_went() {
        let (bin, _log) = stub(
            "stopped",
            r#"case "$1" in
  load) echo 'cliamp is not running' >&2; exit 1 ;;
esac
exit 0"#,
        );
        let dir = bin.parent().unwrap().to_path_buf();
        let said = load_with(&bin.to_string_lossy(), SLACK, SLACK, &folder(&dir)).unwrap();
        assert!(said.contains("start cliamp"), "{said}");
        assert!(said.contains("earworm - Focus ("), "{said}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A refused import is a real failure, unlike a refused delete or load.
    #[test]
    fn a_failed_import_is_reported_with_what_cliamp_said() {
        let (bin, _log) = stub(
            "badimport",
            r#"case "$1 $2" in
  "playlist import") echo 'invalid playlist name' >&2; exit 1 ;;
esac
exit 0"#,
        );
        let dir = bin.parent().unwrap().to_path_buf();
        let err = load_with(&bin.to_string_lossy(), SLACK, SLACK, &folder(&dir)).unwrap_err();
        assert!(err.to_string().contains("invalid playlist name"), "{err}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /* Two `--dir` roots can both hold a `Focus`, and cliamp's store is flat, so
       without the path in the name the second folder played would silently
       replace the first rather than sitting beside it. */
    #[test]
    fn two_folders_of_the_same_name_get_different_playlists() {
        let root = std::env::temp_dir().join(format!("earworm-collide-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (a, b) = (root.join("one"), root.join("two"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();

        let (first, second) = (stored(&folder(&a)), stored(&folder(&b)));
        assert!(first.starts_with("earworm - Focus ("), "{first}");
        assert_ne!(first, second, "both folders claim one playlist");
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* Pinned against an independent FNV-1a, because the name has to survive
       between releases: a changed hash orphans every playlist already sitting
       in cliamp under the old one, and nothing would say why. */
    #[test]
    fn the_path_tag_is_the_same_hash_it_has_always_been() {
        assert_eq!(tag(Path::new("/music/Focus")), "7fe2");
        assert_eq!(tag(Path::new("")), "9dc5");
    }

    /// cliamp counted the tracks it actually took, which beats a guess.
    #[test]
    fn reports_what_cliamp_said_it_imported() {
        let (bin, _log) = stub(
            "counted",
            r#"case "$1 $2" in
  "playlist import") echo 'Imported 24 tracks into "X".'; exit 0 ;;
esac
case "$1" in
  load) exit 1 ;;
esac
exit 0"#,
        );
        let dir = bin.parent().unwrap().to_path_buf();
        let said = load_with(&bin.to_string_lossy(), SLACK, SLACK, &folder(&dir)).unwrap();
        assert!(said.starts_with("Imported 24 tracks"), "{said}");
        assert!(said.contains("start cliamp"), "{said}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /* One call and no arguments, because cliamp owns what "resume" means for
       whatever it has loaded. Getting this wrong is silent: the key would
       appear to do nothing rather than report anything. */
    #[test]
    fn toggling_asks_cliamp_once_and_says_nothing_on_success() {
        let (bin, log) = stub("toggle", "exit 0");
        let dir = bin.parent().unwrap().to_path_buf();
        toggle_with(&bin.to_string_lossy(), SLACK).unwrap();
        assert_eq!(calls(&log), ["toggle|"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A refused toggle carries cliamp's own words, not a guess at the cause.
    #[test]
    fn a_refused_toggle_is_reported() {
        let (bin, _log) = stub("badtoggle", "echo 'cliamp is not running' >&2; exit 1");
        let dir = bin.parent().unwrap().to_path_buf();
        let err = toggle_with(&bin.to_string_lossy(), SLACK).unwrap_err();
        assert!(err.to_string().contains("not running"), "{err}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /* cliamp reports whatever it has loaded, and that is as often a radio
       stream as a file. A URL has no folder, and treating one as a path would
       mark whichever library row happened to sit at its last segment. */
    #[test]
    fn only_a_local_track_belongs_to_a_folder() {
        assert_eq!(
            folder_of(Some("/music/Focus/01 - A.opus")),
            Some(PathBuf::from("/music/Focus"))
        );
        assert_eq!(folder_of(Some("https://radio.cliamp.stream/lofi/stream")), None);
        assert_eq!(folder_of(Some("relative/01 - A.opus")), None);
        assert_eq!(folder_of(Some("")), None);
        assert_eq!(folder_of(None), None);
    }

    /* The .m3u8 is the whole input, so its absence has to stop this before any
       subprocess runs: a folder synced under `--no-m3u8` arrives here. */
    #[test]
    fn refuses_a_folder_with_no_playlist_file() {
        let err = load(Path::new("/nonexistent/Nope")).unwrap_err();
        assert!(err.to_string().contains("no .m3u8"), "{err}");
    }
}
