use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{ArgMatches, Parser, parser::ValueSource};
use serde::Deserialize;

/// Download a YouTube playlist as tagged audio files.
#[derive(Parser, Debug)]
#[command(name = "earworm", version, about, long_about = None)]
pub struct Cli {
    /// Playlist or video URL. Omit it and earworm asks for one.
    pub url: Option<String>,

    /// Output directory
    #[arg(short, long, default_value = "~/Music")]
    pub dir: String,

    /// Audio format: opus, m4a, mp3, flac, vorbis or alac
    #[arg(short, long, value_name = "NAME")]
    pub format: Option<String>,

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

    /// Keep yt-dlp's filenames instead of renaming from the corrected tags
    #[arg(short = 'R', long)]
    pub no_rename: bool,

    /// Skip filling an empty album tag with the playlist name
    #[arg(short = 'A', long)]
    pub no_album: bool,

    /// Download the whole playlist without stopping to choose tracks
    #[arg(long)]
    pub no_pick: bool,

    /// Sync every playlist folder already under --dir, using the URL each
    /// one recorded
    #[arg(long, conflicts_with = "url")]
    pub resync: bool,

    /// Print the playlists already under --dir and exit
    #[arg(long, conflicts_with = "url")]
    pub list: bool,

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
    pub format: Option<String>,
    pub parse: Option<bool>,
    pub m3u8: Option<bool>,
    pub cover: Option<bool>,
    pub lookup: Option<bool>,
    pub fix: Option<bool>,
    pub rename: Option<bool>,
    pub album: Option<bool>,
    pub pick: Option<bool>,
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

/* yt-dlp's name for the format paired with the extension it actually writes.
   The two disagree for `vorbis` and `alac`, and both halves are needed:
   `run` passes the name to --audio-format while `scan` predicts filenames
   with the extension. wav and aac are deliberately absent: neither carries
   tags or cover art, so earworm would do a third of its job and report it
   as finished. */
pub const FORMATS: [(&str, &str); 6] = [
    ("opus", "opus"),
    ("m4a", "m4a"),
    ("mp3", "mp3"),
    ("flac", "flac"),
    ("vorbis", "ogg"),
    ("alac", "m4a"),
];

pub const DEFAULT_FORMAT: &str = "opus";

/// The extension a format lands on disk with, which is not always its name.
pub fn extension(format: &str) -> &'static str {
    FORMATS
        .iter()
        .find(|(name, _)| *name == format)
        .map_or("opus", |(_, ext)| *ext)
}

/// Writes one key back in place, leaving comments and every other line as
/// they were: the file is hand-edited, and rewriting it from the parsed
/// struct would drop the comments and flatten the formatting.
pub fn save_format(path: &std::path::Path, format: &str) -> Result<()> {
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let line = format!("format = \"{format}\"");
    let mut out = String::new();
    let mut replaced = false;
    for existing in old.lines() {
        let names_format = existing
            .trim_start()
            .strip_prefix("format")
            .is_some_and(|rest| rest.trim_start().starts_with('='));
        out.push_str(if names_format && !replaced {
            replaced = true;
            &line
        } else {
            existing
        });
        out.push('\n');
    }
    if !replaced {
        out.push_str(&line);
        out.push('\n');
    }
    /* A config earworm can no longer read is worse than one that never
       recorded the choice: the next start would fail outright. */
    toml::from_str::<FileConfig>(&out).context("the edited config no longer parses")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))
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
    /// yt-dlp's name for the format, not the extension. See `FORMATS`.
    pub format: String,
    /// Where a setting changed in the tool gets written back. `None` under
    /// --no-config, which asked for the file to be left out of the run and
    /// so cannot be the place a choice is remembered.
    pub config_file: Option<PathBuf>,
    pub parse: bool,
    pub m3u8: bool,
    pub cover: bool,
    pub lookup: bool,
    pub fix: bool,
    pub resync: bool,
    pub list: bool,
    pub rename: bool,
    pub album: bool,
    pub pick: bool,
    pub extra: Vec<String>,
    pub acoustid_key: Option<String>,
}

impl Config {
    /// Needs the raw matches as well as the parsed `Cli`: a `bool` flag reads
    /// as `false` whether it was left off or never mentioned, and only the
    /// value source separates "the user asked for this" from "unset".
    pub fn build(cli: Cli, matches: &ArgMatches) -> Result<Self> {
        let mut config_file = None;
        let file = if cli.no_config {
            FileConfig::default()
        } else {
            let named = cli.config.as_ref();
            let path = named.map_or_else(config_path, |p| expand(p));
            let loaded = FileConfig::load(&path, named.is_some())?;
            config_file = Some(path);
            loaded
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

        let format = cli
            .format
            .or(file.format)
            .unwrap_or_else(|| DEFAULT_FORMAT.to_string());
        /* Checked here rather than left to yt-dlp: an unknown name gets as
           far as the download before failing, by which time the scan has
           already predicted filenames with an extension nothing will write. */
        if !FORMATS.iter().any(|(name, _)| *name == format) {
            let known: Vec<&str> = FORMATS.iter().map(|(name, _)| *name).collect();
            anyhow::bail!("unknown format \"{format}\", expected one of {}", known.join(", "));
        }

        Ok(Config {
            url: cli.url.unwrap_or_default(),
            dir: expand(&dir),
            format,
            config_file,
            parse: off("no_parse", file.parse),
            m3u8: off("no_m3u8", file.m3u8),
            cover: off("no_cover", file.cover),
            lookup: off("no_lookup", file.lookup),
            fix: off("no_fix", file.fix),
            // Actions, not settings, so they have no config key.
            resync: cli.resync,
            list: cli.list,
            rename: off("no_rename", file.rename),
            album: off("no_album", file.album),
            pick: off("no_pick", file.pick),
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
            "format {} · parse {} · lookup {} · cover {} · m3u8 {} · prompts {} · rename {}",
            self.format,
            on(self.parse),
            on(self.lookup),
            on(self.cover),
            on(self.m3u8),
            on(self.fix),
            on(self.rename)
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

    /// Reads a config file back through the real parser, which is the only
    /// thing that proves a written setting survives a restart.
    fn build_at(path: &str) -> Config {
        run(vec!["earworm".to_string(), "--config".into(), path.to_string()])
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
    fn the_format_comes_from_the_file_and_the_flag_beats_it() {
        assert_eq!(build("", &[]).format, DEFAULT_FORMAT);
        assert_eq!(build("format = \"flac\"\n", &[]).format, "flac");
        assert_eq!(build("format = \"flac\"\n", &["--format", "mp3"]).format, "mp3");
    }

    /* Left to yt-dlp it fails after the scan has already predicted filenames
       with an extension nothing is going to write. */
    #[test]
    fn an_unknown_format_is_refused_at_startup() {
        let mut file = temp("format");
        writeln!(file.handle, "format = \"wav\"").unwrap();
        let matches = Cli::command()
            .get_matches_from(["earworm", "--config", &file.path]);
        let built = Config::build(Cli::from_arg_matches(&matches).unwrap(), &matches);
        let err = match built {
            Err(err) => err.to_string(),
            Ok(cfg) => panic!("wav was accepted as {}", cfg.format),
        };
        assert!(err.contains("unknown format"), "{err}");
    }

    /// Two formats land on an extension that is not their name, and the scan
    /// predicts filenames from the extension alone.
    #[test]
    fn the_extension_is_not_always_the_format_name() {
        assert_eq!(extension("vorbis"), "ogg");
        assert_eq!(extension("alac"), "m4a");
        assert_eq!(extension("opus"), "opus");
    }

    /* The file is hand-edited, so saving a setting has to leave every other
       line where it was. */
    #[test]
    fn saving_the_format_replaces_the_key_and_keeps_the_rest() {
        let file = temp("save");
        std::fs::write(
            &file.path,
            "# keep me\nformat = \"opus\"\ndir = \"/tmp/music\"\n",
        )
        .unwrap();
        save_format(std::path::Path::new(&file.path), "flac").unwrap();

        let back = std::fs::read_to_string(&file.path).unwrap();
        assert_eq!(back, "# keep me\nformat = \"flac\"\ndir = \"/tmp/music\"\n");
        assert_eq!(build_at(&file.path).format, "flac");
    }

    #[test]
    fn saving_the_format_adds_the_key_when_it_is_missing() {
        let file = temp("add");
        std::fs::write(&file.path, "dir = \"/tmp/music\"\n").unwrap();
        save_format(std::path::Path::new(&file.path), "m4a").unwrap();
        assert_eq!(build_at(&file.path).format, "m4a");
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

    /* The gate is the default now, so its flag is a negation like every other
       one: the command line can switch it off, and only the file can hold it
       on against a habit of typing --no-pick. */
    #[test]
    fn the_pick_gate_is_on_unless_something_says_otherwise() {
        assert!(build("", &[]).pick, "a bare run lost the gate");
        assert!(!build("", &["--no-pick"]).pick, "the flag did not switch it off");
        assert!(!build("pick = false\n", &[]).pick, "the file could not switch it off");
        assert!(
            !build("pick = true\n", &["--no-pick"]).pick,
            "the command line lost to the file"
        );
    }
}
