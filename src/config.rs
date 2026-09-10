use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{ArgMatches, Parser, parser::ValueSource};
use serde::Deserialize;

/// Download a YouTube playlist as tagged opus files.
#[derive(Parser, Debug)]
#[command(name = "earworm", version, about, long_about = None)]
pub struct Cli {
    /// Playlist or video URL. Omit it and earworm asks for one.
    pub url: Option<String>,

    /// Output directory
    #[arg(short, long, default_value = "~/Music")]
    pub dir: String,

    /// Keep YouTube's own artist/track, skip title parsing
    #[arg(short = 'P', long)]
    pub no_parse: bool,

    /// Skip the .m3u8 playlist written beside the tracks
    #[arg(short = 'M', long)]
    pub no_m3u8: bool,

    /// Skip the cover.jpg written beside the tracks
    #[arg(short = 'C', long)]
    pub no_cover: bool,

    /// Skip AcoustID/Deezer identification, keep parsed tags
    #[arg(short = 'L', long)]
    pub no_lookup: bool,

    /// Never prompt; unresolved tracks keep their parsed tags
    #[arg(short = 'F', long)]
    pub no_fix: bool,

    /// Read this config file instead of the one in ~/.config/earworm
    #[arg(long, value_name = "PATH")]
    pub config: Option<String>,

    /// Ignore the config file entirely
    #[arg(long, conflicts_with = "config")]
    pub no_config: bool,

    /// Extra arguments passed straight to yt-dlp
    #[arg(last = true)]
    pub extra: Vec<String>,
}

/// Every field optional, so an absent key means "no opinion" and falls through
/// to the built-in default rather than overwriting it with `false`.
#[derive(Deserialize, Default, Debug)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub dir: Option<String>,
    pub parse: Option<bool>,
    pub m3u8: Option<bool>,
    pub cover: Option<bool>,
    pub lookup: Option<bool>,
    pub fix: Option<bool>,
    pub extra: Option<Vec<String>>,
    pub acoustid_key: Option<String>,
}

impl FileConfig {
    /// `required` when the path came from --config: naming a file earworm
    /// then ignores is worse than no config at all, so only the default
    /// location is allowed to be absent.
    pub fn load(path: &std::path::Path, required: bool) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => {
                return Ok(Self::default());
            }
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        /* A typo'd key silently ignored would look like earworm disregarding
           the setting, so deny_unknown_fields turns it into a startup error. */
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map_or_else(|| expand("~/.config"), PathBuf::from);
    base.join("earworm").join("config.toml")
}

pub struct Config {
    pub url: String,
    pub dir: PathBuf,
    pub parse: bool,
    pub m3u8: bool,
    pub cover: bool,
    pub lookup: bool,
    pub fix: bool,
    pub extra: Vec<String>,
    pub acoustid_key: Option<String>,
}

impl Config {
    /// Needs the raw matches as well as the parsed `Cli`: a `bool` flag reads
    /// as `false` whether it was left off or never mentioned, and only the
    /// value source separates "the user asked for this" from "unset".
    pub fn build(cli: Cli, matches: &ArgMatches) -> Result<Self> {
        let file = if cli.no_config {
            FileConfig::default()
        } else {
            let named = cli.config.as_ref();
            let path = named.map_or_else(config_path, |p| expand(p));
            FileConfig::load(&path, named.is_some())?
        };

        let given = |id: &str| matches.value_source(id) == Some(ValueSource::CommandLine);
        // Every flag is a negation, so the command line can only switch a
        // feature off. Absent, the file decides; absent there, the default.
        let off = |id: &str, from_file: Option<bool>| {
            if given(id) { false } else { from_file.unwrap_or(true) }
        };

        let dir = if given("dir") {
            cli.dir
        } else {
            file.dir.unwrap_or(cli.dir)
        };

        /* File arguments first so a command-line one wins: yt-dlp takes the
           last occurrence of a repeated option. */
        let mut extra = file.extra.unwrap_or_default();
        extra.extend(cli.extra);

        Ok(Config {
            url: cli.url.unwrap_or_default(),
            dir: expand(&dir),
            parse: off("no_parse", file.parse),
            m3u8: off("no_m3u8", file.m3u8),
            cover: off("no_cover", file.cover),
            lookup: off("no_lookup", file.lookup),
            fix: off("no_fix", file.fix),
            extra,
            // The environment wins, so a key can be swapped for one run.
            acoustid_key: std::env::var("ACOUSTID_API_KEY")
                .ok()
                .filter(|k| !k.is_empty())
                .or(file.acoustid_key)
                .filter(|k| !k.is_empty()),
        })
    }

    /// Shown in the help overlay, so a run's settings are visible without
    /// remembering which flags were passed.
    pub fn describe(&self) -> String {
        let on = |flag: bool| if flag { "on" } else { "off" };
        format!(
            "parse {} · lookup {} · cover {} · m3u8 {} · prompts {}",
            on(self.parse),
            on(self.lookup),
            on(self.cover),
            on(self.m3u8),
            on(self.fix)
        )
    }
}

pub fn expand(path: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    if path == "~" {
        return PathBuf::from(home);
    }
    match path.strip_prefix("~/") {
        Some(rest) => PathBuf::from(home).join(rest),
        None => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, FromArgMatches};
    use std::io::Write;

    /// Runs the real argument parser over a throwaway config file, so the
    /// tests exercise the same value_source path the binary does.
    fn build(toml: &str, args: &[&str]) -> Config {
        let mut file = temp("build");
        write!(file.handle, "{toml}").unwrap();
        let mut argv = vec!["earworm".to_string(), "--config".into(), file.path.clone()];
        argv.extend(args.iter().map(|a| a.to_string()));
        run(argv)
    }

    fn run(argv: Vec<String>) -> Config {
        let matches = Cli::command().get_matches_from(argv);
        Config::build(Cli::from_arg_matches(&matches).unwrap(), &matches).unwrap()
    }

    struct Temp {
        handle: std::fs::File,
        path: String,
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /* Unique per call, not just per process: the harness runs tests in
       parallel, and two sharing a path would delete each other's file. */
    fn temp(tag: &str) -> Temp {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "earworm-test-{tag}-{}-{n}.toml",
            std::process::id()
        ));
        Temp {
            handle: std::fs::File::create(&path).unwrap(),
            path: path.display().to_string(),
        }
    }

    #[test]
    fn file_supplies_defaults() {
        let cfg = build("dir = \"/tmp/music\"\nlookup = false\n", &[]);
        assert_eq!(cfg.dir, PathBuf::from("/tmp/music"));
        assert!(!cfg.lookup);
        assert!(cfg.parse, "unmentioned keys keep the built-in default");
    }

    #[test]
    fn command_line_beats_file() {
        let cfg = build("dir = \"/tmp/music\"\n", &["--dir", "/tmp/other"]);
        assert_eq!(cfg.dir, PathBuf::from("/tmp/other"));
    }

    #[test]
    fn extra_args_concatenate_with_the_file_first() {
        let cfg = build("extra = [\"--sleep-requests\", \"1\"]\n", &["--", "--quiet"]);
        assert_eq!(cfg.extra, ["--sleep-requests", "1", "--quiet"]);
    }

    #[test]
    fn no_config_falls_back_to_the_built_in_defaults() {
        let cfg = run(vec!["earworm".to_string(), "--no-config".into()]);
        assert_eq!(cfg.dir, expand("~/Music"));
        assert!(cfg.cover);
    }

    #[test]
    fn no_config_and_config_together_are_rejected() {
        let err = Cli::command()
            .try_get_matches_from(["earworm", "--no-config", "--config", "/tmp/x.toml"])
            .unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn unknown_key_is_an_error() {
        let mut file = temp("unknown");
        writeln!(file.handle, "dirr = \"/tmp\"").unwrap();
        let err = FileConfig::load(std::path::Path::new(&file.path), false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("parsing"), "{err}");
    }

    #[test]
    fn a_named_config_must_exist() {
        let missing = std::env::temp_dir().join("earworm-test-absent.toml");
        let _ = std::fs::remove_file(&missing);
        assert!(FileConfig::load(&missing, false).is_ok(), "default path may be absent");
        let err = FileConfig::load(&missing, true).unwrap_err().to_string();
        assert!(err.contains("reading"), "{err}");
    }
}
