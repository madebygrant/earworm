use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::lookup;

const BIN: &str = "cliamp";

/// `load` and `--version` reach the running instance over a socket, so they
/// answer at once or never.
const IPC: Duration = Duration::from_secs(2);
/// Importing reads the tags off every file, which for a long playlist is
/// genuinely slow, and killing it part-way would leave cliamp's store torn.
const STORE: Duration = Duration::from_secs(30);

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

/// Whether the player is up, which `cliamp status` answers by its exit code
/// alone. Cheap enough to poll: about 11ms, against a socket that is either
/// there or not.
pub fn running() -> bool {
    installed() && cliamp(BIN, IPC, &["status"]).is_some_and(|(ok, _)| ok)
}

pub fn load(folder: &Path) -> Result<String> {
    load_with(BIN, folder)
}

/* `import` refuses a name it already holds rather than replacing it, so a
   second sync of the same playlist would fail on an unchanged copy. Deleting
   first is what makes this a refresh, and the delete failing is the ordinary
   first-time case rather than an error. */
fn load_with(bin: &str, folder: &Path) -> Result<String> {
    let playlist = playlist_file(folder);
    if !playlist.is_file() {
        bail!("no .m3u8 to play; run with --no-m3u8 off");
    }
    let Some(file) = playlist.to_str() else {
        bail!("playlist path is not valid UTF-8");
    };
    let name = stored(folder);

    let _ = cliamp(bin, STORE, &["playlist", "delete", &name]);
    let imported = match cliamp(bin, STORE, &["playlist", "import", "--name", &name, file]) {
        None => bail!("cliamp did not answer"),
        Some((false, why)) => bail!("{why}"),
        // cliamp counted the tracks it took, which is better than a guess.
        Some((true, said)) => said,
    };

    /* Not running is the ordinary case rather than a failure: the tracks are
       imported either way, so say where they landed. The stored name and not
       the folder's, since that is what to look for in cliamp. */
    match cliamp(bin, IPC, &["load", &name]) {
        Some((true, _)) => Ok(format!("playing \"{name}\" in cliamp")),
        _ if imported.is_empty() => Ok(format!("imported \"{name}\"; start cliamp to play it")),
        _ => Ok(format!("{imported} start cliamp to play it")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A stand-in `cliamp` that logs its argv and exits by the rules given as
    /// `case` lines, so the call sequence can be asserted without cliamp.
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
        let said = load_with(&bin.to_string_lossy(), &folder(&dir)).unwrap();

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
        load_with(&bin.to_string_lossy(), &folder(&dir)).unwrap();

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
        let said = load_with(&bin.to_string_lossy(), &folder(&dir)).unwrap();
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
        let err = load_with(&bin.to_string_lossy(), &folder(&dir)).unwrap_err();
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
        let said = load_with(&bin.to_string_lossy(), &folder(&dir)).unwrap();
        assert!(said.starts_with("Imported 24 tracks"), "{said}");
        assert!(said.contains("start cliamp"), "{said}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /* The .m3u8 is the whole input, so its absence has to stop this before any
       subprocess runs: a folder synced under `--no-m3u8` arrives here. */
    #[test]
    fn refuses_a_folder_with_no_playlist_file() {
        let err = load(Path::new("/nonexistent/Nope")).unwrap_err();
        assert!(err.to_string().contains("no .m3u8"), "{err}");
    }
}
