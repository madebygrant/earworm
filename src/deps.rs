use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::lookup::run_bounded;

/// Every version flag here prints and exits at once.
const VERSION: Duration = Duration::from_secs(3);

pub struct Dep {
    pub bin: &'static str,
    /// The Homebrew formula, which is not always the binary's own name.
    pub formula: &'static str,
    pub what: &'static str,
    /// How the tool prints its version. Not all of them take `--version`.
    flag: &'static str,
    /// Whether earworm can do its job without it.
    pub required: bool,
}

/* One table, so a missing binary can say how to get it: `fpcalc` is the one
   name here that is not its own package. */
pub const DEPS: [Dep; 4] = [
    Dep {
        bin: "yt-dlp",
        formula: "yt-dlp",
        what: "downloading",
        flag: "--version",
        required: true,
    },
    Dep {
        bin: "ffmpeg",
        formula: "ffmpeg",
        what: "audio conversion and cover art",
        flag: "-version",
        required: true,
    },
    Dep {
        bin: "fpcalc",
        formula: "chromaprint",
        what: "acoustic fingerprinting",
        flag: "-version",
        required: false,
    },
    Dep {
        bin: "cliamp",
        formula: "cliamp",
        what: "playback",
        flag: "--version",
        required: false,
    },
];

fn dep(bin: &str) -> Option<&'static Dep> {
    DEPS.iter().find(|d| d.bin == bin)
}

/// Only a NotFound is a missing install: a permission error told to run
/// `brew install` sends the reader after the wrong problem.
pub fn launch_error(bin: &str, err: &std::io::Error) -> String {
    if err.kind() != ErrorKind::NotFound {
        return format!("could not run {bin}: {err}");
    }
    match dep(bin) {
        Some(d) => format!("{bin} not found on PATH · brew install {}", d.formula),
        None => format!("{bin} not found on PATH"),
    }
}

pub struct Found {
    pub dep: &'static Dep,
    /// `None` when the tool is not on PATH or would not say.
    pub version: Option<String>,
}

/// ffmpeg opens with its whole build configuration, and three of the four
/// restate their own name before the number that was asked for.
fn version_of(bin: &str, text: &str) -> Option<String> {
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    let mut words = line.split_whitespace().peekable();
    for skip in [bin, "version"] {
        if words.peek().is_some_and(|w| w.eq_ignore_ascii_case(skip)) {
            words.next();
        }
    }
    // One word: what follows it is a copyright line or a build string.
    words.next().map(str::to_string)
}

pub fn probe_all() -> Vec<Found> {
    DEPS.iter()
        .map(|dep| {
            let out = run_bounded(
                Command::new(dep.bin)
                    .arg(dep.flag)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null()),
                VERSION,
            );
            let version = out
                .filter(|o| o.status.success())
                .and_then(|o| version_of(dep.bin, &String::from_utf8_lossy(&o.stdout)));
            Found { dep, version }
        })
        .collect()
}

/// Gathered by the caller, so `report` touches neither PATH nor the disk.
pub struct Facts<'a> {
    pub tools: &'a [Found],
    /// The config earworm would read, and whether it is there. `None` under
    /// --no-config, which asked for no file at all.
    pub config: Option<&'a Path>,
    pub config_exists: bool,
    pub key: bool,
    pub apple: bool,
    pub format: &'a str,
    pub extension: &'a str,
    /// The palette that would be drawn, and anything about it that measures
    /// badly. A `[colors]` table makes the name "custom".
    pub theme: &'a str,
    pub theme_warnings: &'a [String],
    /* How this terminal would draw a cover, as the protocol handshake
       answered it. Gathered by the caller like everything else here, because
       the query needs a real terminal and `--check` is meant to work without
       one. */
    pub graphics: &'a str,
    /// Whether tracks already on disk follow `format` at the next sync.
    pub convert: bool,
    /* Files under --dir that are in some other format, and the folders they
       are spread over. Gathered by the caller, like everything else here, so
       the report stays testable as the text it is. `None` when the directory
       is not there to walk. */
    pub off_format: Option<(usize, usize)>,
    pub dir: &'a Path,
    /// `None` when the directory is not there yet.
    pub playlists: Option<usize>,
    /// Folders `D` has hidden, which are on disk and on no screen. Named
    /// rather than counted, since the question this answers is which one.
    pub hidden: &'a [String],
    /// The newer release the probe found, gathered by the caller so this
    /// stays as testable as the rest of the report. `None` is also a switched
    /// off check, a stale read and "already current" all at once, which is
    /// why there is no false row: silence is the no-news answer.
    pub update: Option<String>,
}

/// The report, and whether everything earworm cannot work without is present.
/// The bool is the exit status, so `--check` is usable from a script.
pub fn report(facts: &Facts) -> (String, bool) {
    let mut out = format!("earworm {}\n\ntools\n", env!("CARGO_PKG_VERSION"));
    let mut ready = true;
    for found in facts.tools {
        let dep = found.dep;
        let note = match &found.version {
            Some(version) => version.clone(),
            None => {
                // An optional tool says what you lose, or a bare cross
                // reads as breakage rather than a feature switched off.
                if dep.required {
                    ready = false;
                    format!("not found · brew install {}", dep.formula)
                } else {
                    format!(
                        "not found · brew install {} · no {}",
                        dep.formula, dep.what
                    )
                }
            }
        };
        out.push_str(&row(found.version.is_some(), dep.bin, &note));
    }

    out.push_str("\nsettings\n");
    match facts.config {
        Some(path) if facts.config_exists => {
            out.push_str(&row(true, "config", &path.display().to_string()));
        }
        Some(path) => {
            // Not an error: the defaults are a whole configuration.
            out.push_str(&row(
                true,
                "config",
                &format!("none yet · would read {}", path.display()),
            ));
        }
        None => out.push_str(&row(true, "config", "ignored · --no-config")),
    }
    out.push_str(&row(
        facts.key,
        "acoustid",
        if facts.key {
            "key set"
        } else if facts.apple {
            "no key · set ACOUSTID_API_KEY · lookups fall back to Deezer and Apple"
        } else {
            "no key · set ACOUSTID_API_KEY · lookups fall back to Deezer"
        },
    ));
    out.push_str(&row(
        true,
        "format",
        &format!("{} · writes .{}", facts.format, facts.extension),
    ));
    /* The one row that answers "why is there no cover art". Everything about
       it is invisible otherwise: whether the terminal answered the handshake,
       and which of the four ways of drawing a picture it settled on. Never a
       cross, because no protocol is a terminal's choice and not a fault. */
    out.push_str(&row(true, "graphics", facts.graphics));
    /* Named here because `^t` writes it back to the config, so "which theme
       am I on" is a question with an answer that changes. A colour that
       measures badly is said here too: the TUI says it once at startup and
       then never again, which is the right amount there and no use at all to
       somebody working out why a theme looks wrong. */
    out.push_str(&row(
        facts.theme_warnings.is_empty(),
        "theme",
        &if facts.theme_warnings.is_empty() {
            facts.theme.to_string()
        } else {
            format!("{} · {}", facts.theme, facts.theme_warnings.join(" · "))
        },
    ));
    /* The answer to "did my format change take", which is otherwise one
       folder at a time. Silent at zero: a row saying nothing is wrong is a
       row the reader has to check every time. */
    out.push_str(&row(
        true,
        "convert",
        &match (facts.convert, facts.off_format) {
            (false, Some((n, folders))) if n > 0 => format!(
                "off · {n} tracks in {folders} playlists are not {}",
                facts.format
            ),
            (false, _) => "off · tracks already on disk keep their format".into(),
            (true, Some((0, _)) | None) => {
                format!("on · everything under --dir is already {}", facts.format)
            }
            (true, Some((n, folders))) => format!(
                "on · {n} tracks in {folders} playlists become {} at the next sync",
                facts.format
            ),
        },
    ));
    let dir = facts.dir.display();
    out.push_str(&match facts.playlists {
        Some(1) => row(true, "directory", &format!("{dir} · 1 playlist")),
        Some(n) => row(true, "directory", &format!("{dir} · {n} playlists")),
        None => row(true, "directory", &format!("{dir} · not created yet")),
    });
    /* Its own row rather than a clause on the directory's, since it is the
       answer to "why is that folder not here", and a row the reader can find
       is the whole point of putting it in `--check`. Absent when nothing is
       hidden: a row saying none would be a question nobody asked. */
    if !facts.hidden.is_empty() {
        out.push_str(&row(
            true,
            "hidden",
            &format!(
                "{}  ·  on disk, on no screen, and passed over by --resync",
                facts.hidden.join(", ")
            ),
        ));
    }
    if let Some(latest) = &facts.update {
        /* Both sides carry exactly one `v`: `update::available` hands over the
           tag without one, so the prefix is added here and nowhere else. */
        out.push_str(&row(
            true,
            "update",
            &format!(
                "v{} → v{latest} · {}",
                crate::update::current(),
                crate::update::hint()
            ),
        ));
    }
    (out, ready)
}

/// A glyph rather than a colour, so it survives a pipe and a screen reader.
fn row(ok: bool, name: &str, note: &str) -> String {
    let mark = if ok { '✓' } else { '✗' };
    format!("  {mark} {name:<10} {note}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn found(bin: &str, version: Option<&str>) -> Found {
        Found {
            dep: dep(bin).unwrap(),
            version: version.map(str::to_string),
        }
    }

    /* Hiding a folder takes it off every screen and out of `--resync`, and
       says so once, in a flash, at the moment it happens. `--check` is where
       somebody working out why a folder is not syncing goes, so it is where
       the answer has to be. */
    #[test]
    fn check_names_the_folders_that_are_hidden_and_stays_quiet_when_none_are() {
        let tools: Vec<Found> = Vec::new();
        let dir = PathBuf::from("/tmp/music");

        let quiet = report(&facts(&tools, &dir)).0;
        assert!(!quiet.contains("hidden"), "a row about nothing: {quiet}");

        let names = ["deep-focus-chillstep".to_string(), "sleep".to_string()];
        let mut with = facts(&tools, &dir);
        with.hidden = &names;
        let text = report(&with).0;
        assert!(text.contains("deep-focus-chillstep, sleep"), "{text}");
        assert!(text.contains("--resync"), "no word on what it costs: {text}");
    }

    fn facts<'a>(tools: &'a [Found], dir: &'a Path) -> Facts<'a> {
        Facts {
            tools,
            config: None,
            config_exists: false,
            key: true,
            format: "opus",
            convert: false,
            off_format: Some((0, 0)),
            extension: "opus",
            theme: "warm",
            theme_warnings: &[],
            graphics: "halfblocks",
            dir,
            playlists: Some(7),
            hidden: &[],
            apple: true,
            update: None,
        }
    }

    /// One screen saying what is missing and what to type to get it.
    #[test]
    fn a_missing_tool_is_reported_with_the_way_to_install_it() {
        let tools = [
            found("yt-dlp", Some("2024.08.06")),
            found("ffmpeg", None),
            found("fpcalc", None),
            found("cliamp", Some("cliamp 2.1.0")),
        ];
        let dir = PathBuf::from("/tmp/music");
        let (text, ready) = report(&facts(&tools, &dir));

        assert!(!ready, "a missing ffmpeg is not a working install");
        assert!(text.contains("✓ yt-dlp     2024.08.06"), "{text}");
        assert!(text.contains("✗ ffmpeg     not found · brew install ffmpeg"), "{text}");
        // fpcalc is not its own package, which is the whole reason for the table.
        assert!(text.contains("brew install chromaprint"), "{text}");
        // An optional tool says what you lose, or a cross reads as breakage.
        assert!(text.contains("no acoustic fingerprinting"), "{text}");
        assert!(text.contains("7 playlists"), "{text}");
    }

    /// Nothing missing that matters, so a script can act on the status.
    #[test]
    fn a_complete_install_reports_ready() {
        let tools = [
            found("yt-dlp", Some("2024.08.06")),
            found("ffmpeg", Some("ffmpeg version 7.0.2")),
            found("fpcalc", Some("fpcalc version 1.5.1")),
            found("cliamp", None),
        ];
        let dir = PathBuf::from("/tmp/music");
        let (text, ready) = report(&facts(&tools, &dir));
        assert!(ready, "an optional tool decided the exit status: {text}");
        assert_eq!(text.matches('✗').count(), 1, "{text}");
        assert!(text.contains("✗ cliamp"), "{text}");
    }

    /// A config that is not there yet is the ordinary first run.
    #[test]
    fn an_absent_config_says_where_one_would_go() {
        let tools = [
            found("yt-dlp", Some("1")),
            found("ffmpeg", Some("1")),
            found("fpcalc", Some("1")),
            found("cliamp", Some("1")),
        ];
        let dir = PathBuf::from("/tmp/music");
        let path = PathBuf::from("/home/me/.config/earworm/config.toml");
        let mut facts = facts(&tools, &dir);
        facts.config = Some(&path);
        facts.key = false;
        facts.playlists = None;

        let (text, ready) = report(&facts);
        assert!(ready, "a missing config is not a broken install: {text}");
        assert!(text.contains("none yet · would read /home/me/.config"), "{text}");
        assert!(text.contains("ACOUSTID_API_KEY"), "{text}");
        assert!(text.contains("not created yet"), "{text}");
    }

    /// Without a key the report names the fallbacks, so `--check` says what
    /// a lookup without fingerprinting actually searches.
    #[test]
    fn the_fallback_row_names_apple_only_when_it_is_on() {
        let tools = [
            found("yt-dlp", Some("1")),
            found("ffmpeg", Some("1")),
            found("fpcalc", Some("1")),
            found("cliamp", Some("1")),
        ];
        let dir = PathBuf::from("/tmp/music");
        let mut with = facts(&tools, &dir);
        with.key = false;
        with.apple = true;
        let (text, _) = report(&with);
        assert!(text.contains("Deezer and Apple"), "{text}");

        let mut without = facts(&tools, &dir);
        without.key = false;
        without.apple = false;
        let (text, _) = report(&without);
        assert!(text.contains("fall back to Deezer"), "{text}");
        assert!(!text.contains("Apple"), "{text}");
    }

    /// A newer release gets one row, with the numbers and the way to act on
    /// them; nothing newer says nothing, which is why there is no false row.
    #[test]
    fn an_update_row_carries_its_hint_and_no_news_carries_none() {
        let tools = [
            found("yt-dlp", Some("1")),
            found("ffmpeg", Some("1")),
            found("fpcalc", Some("1")),
            found("cliamp", Some("1")),
        ];
        let dir = PathBuf::from("/tmp/music");
        let mut facts = facts(&tools, &dir);
        facts.update = Some("99.0.0".into());
        let (text, _) = report(&facts);
        assert!(
            text.contains(&format!("v{} → v99.0.0", crate::update::current())),
            "{text}"
        );
        assert!(text.contains("cargo install"), "{text}");

        facts.update = None;
        let (text, _) = report(&facts);
        assert!(!text.contains("update"), "{text}");
    }

    /// ffmpeg opens with its entire build configuration, and most of them
    /// repeat their own name before the number the column is for.
    #[test]
    fn a_version_banner_is_cut_down_to_a_version() {
        let banner = "ffmpeg version 7.0.2 Copyright (c) 2000-2024 the FFmpeg developers\n\
                      built with Apple clang version 15.0.0\n";
        assert_eq!(version_of("ffmpeg", banner).unwrap(), "7.0.2");
        assert_eq!(version_of("cliamp", "cliamp version v2.1.0\n").unwrap(), "v2.1.0");
        assert_eq!(version_of("yt-dlp", "2024.08.06\n").unwrap(), "2024.08.06");
        assert_eq!(version_of("ffmpeg", "\n\n"), None);
    }

    /// Telling someone to install a binary they already have, because it is
    /// not executable, sends them after the wrong problem.
    #[test]
    fn only_a_missing_binary_is_told_how_to_install_it() {
        let gone = std::io::Error::new(ErrorKind::NotFound, "no such file");
        assert_eq!(
            launch_error("fpcalc", &gone),
            "fpcalc not found on PATH · brew install chromaprint"
        );

        let denied = std::io::Error::new(ErrorKind::PermissionDenied, "denied");
        let said = launch_error("yt-dlp", &denied);
        assert!(said.starts_with("could not run yt-dlp"), "{said}");
        assert!(!said.contains("brew"), "{said}");
    }
}
