//! Where each external tool lives. A desktop app started from Finder gets a PATH without Homebrew's
//! folders, so a bare `Command::new("ffmpeg")` would read as missing on a machine that has it.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::RwLock;

// Set once at startup from the config. Process-wide because ffmpeg is run deep in tagging, far from
// any `Config`, and the answer is the same for the whole session.
static OVERRIDES: RwLock<BTreeMap<String, String>> = RwLock::new(BTreeMap::new());
// What `bin` found for each name, so the tag pass's several ffmpeg runs per track search once.
static FOUND: RwLock<BTreeMap<String, String>> = RwLock::new(BTreeMap::new());

/// Takes the config's `[tools]` table. A front end calls this once, before the worker starts and
/// before anything asks whether a tool is installed: `player::installed` keeps its first answer.
pub fn configure(overrides: &BTreeMap<String, String>) {
    *OVERRIDES.write().unwrap_or_else(|e| e.into_inner()) = overrides.clone();
    FOUND.write().unwrap_or_else(|e| e.into_inner()).clear();
}

/// The path the config's `[tools]` table gives `name`, if it gives one.
pub fn configured(name: &str) -> Option<String> {
    OVERRIDES.read().unwrap_or_else(|e| e.into_inner()).get(name).cloned()
}

/// What to run for `name`: the config's path for it, else the first executable on PATH, else the first
/// in the folders a package manager uses. The bare name when none has it, so the launch fails with
/// `NotFound` and `deps::launch_error` can say how to install it.
pub fn bin(name: &str) -> String {
    if let Some(set) = configured(name) {
        return set;
    }
    if let Some(known) = FOUND.read().unwrap_or_else(|e| e.into_inner()).get(name) {
        return known.clone();
    }
    let found = find(name, std::env::var_os("PATH"), home().as_deref())
        .map_or_else(|| name.to_string(), |p| p.to_string_lossy().into_owned());
    FOUND.write().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), found.clone());
    found
}

/// A command for `name`, run from where `bin` found it and with `path()` as its PATH, so a tool that
/// runs another (yt-dlp running ffmpeg) finds it the way earworm did.
pub fn command(name: &str) -> Command {
    let mut cmd = Command::new(bin(name));
    environ(&mut cmd);
    cmd
}

/// What every child earworm starts gets: `path()` as its PATH and, inside an AppImage, none of the
/// bundle's own folders in its other variables. The AppImage's launcher points the GTK and library
/// variables and PATH into the bundle, and a host tool that inherits them loads the bundle's libraries.
pub fn environ(cmd: &mut Command) -> &mut Command {
    cmd.env("PATH", path());
    if let Some(appdir) = std::env::var_os("APPDIR").filter(|d| !d.is_empty()) {
        let vars = std::env::vars_os().filter(|(key, _)| key != "PATH").chain([("PATH".into(), path())]);
        for (key, value) in outside(vars, Path::new(&appdir)) {
            match value {
                Some(value) => cmd.env(key, value),
                None => cmd.env_remove(key),
            };
        }
    }
    cmd
}

// Each variable that names somewhere inside `appdir`, with those entries taken out, or `None` when
// nothing is left. Kept apart from the environment so it is testable without being in an AppImage.
fn outside(vars: impl Iterator<Item = (OsString, OsString)>, appdir: &Path) -> Vec<(OsString, Option<OsString>)> {
    vars.filter_map(|(key, value)| {
            let entries: Vec<PathBuf> = std::env::split_paths(&value).collect();
            if !entries.iter().any(|e| e.starts_with(appdir)) {
                return None;
            }
            let kept: Vec<PathBuf> = entries.into_iter().filter(|e| !e.starts_with(appdir) && !e.as_os_str().is_empty()).collect();
            Some((key, (!kept.is_empty()).then(|| std::env::join_paths(kept).unwrap_or_default())))
        })
        .collect()
}

/// The PATH every tool earworm starts gets: the one earworm was given, then the package folders, then
/// the folder of each configured tool. Set per child rather than on the process, where changing the
/// environment with other threads running is undefined.
pub fn path() -> OsString {
    let configured: Vec<PathBuf> = OVERRIDES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .filter_map(|p| Path::new(p).parent().map(Path::to_path_buf))
        .collect();
    search_path(std::env::var_os("PATH"), home().as_deref(), configured)
}

fn search_path(path: Option<OsString>, home: Option<&Path>, configured: Vec<PathBuf>) -> OsString {
    let mut dirs: Vec<PathBuf> = path.iter().flat_map(std::env::split_paths).collect();
    for dir in fallbacks(home).into_iter().chain(configured) {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

// Searched after PATH, where Homebrew (on a Mac and on Linux) and pipx put things.
fn fallbacks(home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin"), PathBuf::from("/home/linuxbrew/.linuxbrew/bin")];
    dirs.extend(home.map(|h| h.join(".local/bin")));
    dirs
}

// Kept apart from the environment, so the search is testable without changing PATH or HOME, which in
// a threaded test runner is a process-wide change.
fn find(name: &str, path: Option<OsString>, home: Option<&Path>) -> Option<PathBuf> {
    let on_path = path.iter().flat_map(std::env::split_paths).collect::<Vec<_>>();
    on_path.into_iter().chain(fallbacks(home)).map(|dir| dir.join(name)).find(|p| executable(p))
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("earworm-tools-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    fn tool(at: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(at, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // PATH first, so a tool somebody put there on purpose wins over a package manager's copy, then the
    // folders a Finder-launched app's PATH leaves out.
    #[cfg(unix)]
    #[test]
    fn path_wins_and_the_package_folders_catch_what_it_misses() {
        let dir = scratch("order");
        let (mine, home) = (dir.join("mine"), dir.join("home"));
        tool(&mine.join("earworm-test-tool"));
        tool(&home.join(".local/bin/earworm-test-tool"));
        tool(&home.join(".local/bin/earworm-test-other"));

        let path = Some(OsString::from(mine.as_os_str()));
        assert_eq!(find("earworm-test-tool", path.clone(), Some(&home)), Some(mine.join("earworm-test-tool")), "PATH lost to a fallback");
        assert_eq!(find("earworm-test-other", path.clone(), Some(&home)), Some(home.join(".local/bin/earworm-test-other")), "the fallback was not searched");
        assert_eq!(find("earworm-test-other", None, Some(&home)), Some(home.join(".local/bin/earworm-test-other")), "no PATH at all stopped the search");
        assert_eq!(find("no-such-tool", path, Some(&home)), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // A tool that runs another gets the folders earworm searched, so yt-dlp finds the ffmpeg earworm
    // found: under Finder, PATH alone has neither. Nothing is listed twice, and PATH keeps its order.
    #[test]
    fn a_launched_tool_sees_the_folders_earworm_searched() {
        let home = PathBuf::from("/home/me");
        let configured = vec![PathBuf::from("/opt/ffmpeg-7"), PathBuf::from("/usr/local/bin")];
        let joined = search_path(Some(OsString::from("/usr/bin:/bin:/opt/homebrew/bin")), Some(&home), configured);
        let dirs: Vec<PathBuf> = std::env::split_paths(&joined).collect();
        assert_eq!(
            dirs,
            ["/usr/bin", "/bin", "/opt/homebrew/bin", "/usr/local/bin", "/home/linuxbrew/.linuxbrew/bin", "/home/me/.local/bin", "/opt/ffmpeg-7"]
                .map(PathBuf::from),
        );
    }

    // Inside an AppImage the launcher's variables point into the bundle, and a host tool that inherits
    // them loads the bundle's libraries. Only those entries go; the host's own stay.
    #[test]
    fn a_tool_started_from_an_appimage_keeps_none_of_its_folders() {
        let var = |k: &str, v: &str| (OsString::from(k), OsString::from(v));
        let vars = vec![
            var("XDG_DATA_DIRS", "/tmp/.mount_x/usr/share:/usr/share"),
            var("GDK_PIXBUF_MODULE_FILE", "/tmp/.mount_x/usr/lib/loaders.cache"),
            var("HOME", "/home/me"),
            var("PATH", "/tmp/.mount_x/usr/bin:/usr/bin"),
        ];
        let mut changed = outside(vars.into_iter(), Path::new("/tmp/.mount_x"));
        changed.sort();
        let some = |k: &str, v: &str| (OsString::from(k), Some(OsString::from(v)));
        assert_eq!(
            changed,
            vec![(OsString::from("GDK_PIXBUF_MODULE_FILE"), None), some("PATH", "/usr/bin"), some("XDG_DATA_DIRS", "/usr/share")],
        );
    }

    // A file of the right name that cannot run is not the tool: launching it would fail with a
    // permission error and the real one further down would never be tried.
    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_run_is_passed_over() {
        let dir = scratch("exec");
        let (first, second) = (dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(&first).unwrap();
        std::fs::write(first.join("earworm-test-tool"), "not a program").unwrap();
        tool(&second.join("earworm-test-tool"));
        let path = std::env::join_paths([&first, &second]).ok();
        assert_eq!(find("earworm-test-tool", path, None), Some(second.join("earworm-test-tool")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
