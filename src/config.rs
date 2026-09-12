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

    /// Skip the opening animation
    #[arg(long)]
    pub no_intro: bool,

    /// Don't ring the terminal bell when a long run finishes
    #[arg(long)]
    pub no_notify: bool,

    /// Sync every playlist folder already under --dir, using the URL each
    /// one recorded
    #[arg(long, conflicts_with = "url")]
    pub resync: bool,

    /// Print the playlists already under --dir and exit
    #[arg(long, conflicts_with = "url")]
    pub list: bool,

    /// Check the tools, the key and the config, then exit
    #[arg(long, conflicts_with_all = ["url", "list", "resync"])]
    pub check: bool,

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
    pub intro: Option<bool>,
    pub notify: Option<bool>,
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
/* Never guesses. `Config::build` refuses a format that is not in `FORMATS`
   and `set_format` picks out of the same table, so an unknown one here is a
   mistake in this file rather than in anything a user typed. The old fallback
   to "opus" meant such a mistake predicted the wrong filename for every
   track: the folder would read as empty and download again beside itself. */
pub fn extension(format: &str) -> &'static str {
    FORMATS
        .iter()
        .find(|(name, _)| *name == format)
        .map_or_else(
            || unreachable!("no extension for {format}, which FORMATS should have refused"),
            |(_, ext)| *ext,
        )
}

/// Writes one key back in place, leaving comments and every other line as
/// they were: the file is hand-edited, and rewriting it from the parsed
/// struct would drop the comments and flatten the formatting.
pub fn save_format(path: &std::path::Path, format: &str) -> Result<()> {
    /* Only an absent file is an empty one. Any other read error — a
       permission denial, a byte that is not UTF-8, a directory sitting on the
       path — used to come back as "the config was empty", and the whole file
       was then replaced with a single line. The one thing worse than failing
       to record the setting is losing everything the file already held. */
    let old = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(err).with_context(|| format!("reading {}", path.display()));
        }
    };
    /* A file that was already broken is not something this edit did, and
       reporting it as one sends the reader looking at the wrong line. */
    if !old.trim().is_empty() {
        toml::from_str::<FileConfig>(&old)
            .with_context(|| format!("{} does not parse", path.display()))?;
    }
    let line = format!("format = \"{format}\"");
    let mut out = String::new();
    let mut replaced = false;
    for existing in old.lines() {
        out.push_str(if names_format(existing) && !replaced {
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
    write_atomically(path, &out)
}

/* TOML allows a key to be quoted, and `"format"` read as some other key
   appended a second one. That is a duplicate key, so the re-parse refused it,
   and every save after that failed the same way: the setting could never be
   written again. */
fn names_format(line: &str) -> bool {
    let rest = line.trim_start();
    rest.strip_prefix("format")
        .or_else(|| rest.strip_prefix("\"format\""))
        .or_else(|| rest.strip_prefix("'format'"))
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

/* Written beside the config and renamed over it, so a reader sees either the
   whole old file or the whole new one. `fs::write` truncates first, and a
   crash or a full disk between the truncate and the write leaves a config the
   next start refuses to read. */
fn write_atomically(path: &std::path::Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    /* The resolved path, because a config symlinked out of a dotfiles repo
       would otherwise be replaced by a plain file: the edit would land
       outside the repo and every later change there would stop arriving. */
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = target.parent().unwrap_or_else(|| std::path::Path::new("."));
    let name = target.file_name().unwrap_or_default().to_string_lossy().to_string();
    // Beside the target, since rename only works within one filesystem.
    let temp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&temp, text).with_context(|| format!("writing {}", temp.display()))?;
    /* An AcoustID key sits in this file in plain text and the README says to
       chmod 600 it. A fresh temp file is readable by everyone, so the mode
       travels with the content or an atomic save quietly opens the key up. */
    if let Ok(meta) = std::fs::metadata(&target) {
        let _ = std::fs::set_permissions(&temp, meta.permissions());
    }
    if let Err(err) = std::fs::rename(&temp, &target) {
        let _ = std::fs::remove_file(&temp);
        return Err(err).with_context(|| format!("writing {}", target.display()));
    }
    Ok(())
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
    pub check: bool,
    pub rename: bool,
    pub album: bool,
    pub pick: bool,
    /// The opening animation. Right the first twenty times, and a daily user
    /// looks for the switch in the config rather than in the flags.
    pub intro: bool,
    /// Ring the bell when a run that took a while settles.
    pub notify: bool,
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
            check: cli.check,
            rename: off("no_rename", file.rename),
            album: off("no_album", file.album),
            pick: off("no_pick", file.pick),
            intro: off("no_intro", file.intro),
            notify: off("no_notify", file.notify),
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

    /* Two seconds is right the first twenty times, and a daily user looks
       for the switch in the file rather than in the flags. Both go through
       the same negation rule as every other setting. */
    #[test]
    fn the_intro_and_the_bell_are_on_until_something_turns_them_off() {
        assert!(build("", &[]).intro);
        assert!(build("", &[]).notify);
        assert!(!build("intro = false\n", &[]).intro);
        assert!(!build("notify = false\n", &[]).notify);
        assert!(!build("", &["--no-intro"]).intro);
        assert!(!build("", &["--no-notify"]).notify);
        // The command line can only switch a feature off, never back on.
        assert!(!build("intro = true\n", &["--no-intro"]).intro);
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

    /* `extension` no longer guesses, so the table is what has to hold: every
       name it offers must resolve, and `Config::build` must refuse everything
       else. Between them there is no input that reaches the unreachable. */
    #[test]
    fn every_format_offered_resolves_and_nothing_else_is_accepted() {
        for (name, ext) in FORMATS {
            assert_eq!(extension(name), ext, "{name}");
            assert!(!ext.is_empty(), "{name} has no extension");
            assert_eq!(
                build(&format!("format = \"{name}\"\n"), &[]).format,
                name,
                "{name} is in the table and was still refused"
            );
        }
        for unknown in ["", "wav", "aac", "OPUS", "opus "] {
            let mut file = temp("reject");
            writeln!(file.handle, "format = \"{unknown}\"").unwrap();
            let matches = Cli::command().get_matches_from(["earworm", "--config", &file.path]);
            let built = Config::build(Cli::from_arg_matches(&matches).unwrap(), &matches);
            assert!(
                built.is_err(),
                "{unknown:?} was accepted, and the scan would predict a filename for it"
            );
        }
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

    /* A config earworm cannot read is not an empty one. Read as empty, the
       save replaced a whole hand-written file with a single line: every other
       setting and every comment gone, and the tool reporting success. */
    #[test]
    fn a_config_that_cannot_be_read_is_never_treated_as_an_empty_one() {
        let file = temp("unreadable");
        // Valid TOML apart from one byte, so only the read can object.
        let mut bytes = b"dir = \"/tmp/music\"  # \n".to_vec();
        bytes[21] = 0xff;
        std::fs::write(&file.path, &bytes).unwrap();

        let err = save_format(std::path::Path::new(&file.path), "flac")
            .expect_err("a config it could not read was saved over")
            .to_string();
        assert!(err.contains("reading"), "{err}");
        assert_eq!(
            std::fs::read(&file.path).unwrap(),
            bytes,
            "the file was rewritten from what it could not read"
        );
    }

    /// A file that is not there yet is the ordinary first save, and still the
    /// one case that writes a config of one line.
    #[test]
    fn saving_the_format_creates_a_config_that_is_not_there_yet() {
        let dir = std::env::temp_dir().join(format!("earworm-newcfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("earworm").join("config.toml");
        save_format(&path, "flac").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "format = \"flac\"\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /* TOML lets a key be quoted. Read as some other key, the save appended a
       second `format`, which is a duplicate the re-parse then refused: the
       setting could never be written again, and `f` failed for good. */
    #[test]
    fn a_quoted_format_key_is_the_format_key() {
        let file = temp("quoted");
        std::fs::write(&file.path, "\"format\" = \"opus\"\ndir = \"/tmp/music\"\n").unwrap();
        save_format(std::path::Path::new(&file.path), "flac").unwrap();

        let back = std::fs::read_to_string(&file.path).unwrap();
        assert_eq!(back.matches("format").count(), 1, "a second key was added: {back}");
        assert_eq!(build_at(&file.path).format, "flac");

        // A key that merely starts with the word is a different key, and a
        // commented-out one is not a key at all.
        assert!(names_format("format = \"opus\""));
        assert!(names_format("  \"format\"  = 'mp3'"));
        assert!(names_format("'format'= 1"));
        assert!(!names_format("formats = 1"));
        assert!(!names_format("formatting = 1"));
        assert!(!names_format("# format = opus"));
    }

    /* A config that was already broken is not something the edit did, and
       saying so sends the reader looking at the wrong line. */
    #[test]
    fn a_config_that_never_parsed_is_reported_as_its_own_problem() {
        let file = temp("broken");
        std::fs::write(&file.path, "dir = \"/tmp\"\nnonsense\n").unwrap();
        let err = save_format(std::path::Path::new(&file.path), "flac")
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not parse"), "{err}");
        assert!(!err.contains("edited"), "blamed the edit for a file it never read: {err}");
    }

    /* The one case the guard exists for: a line that looks like the key but
       sits inside another value, where replacing it leaves TOML that will not
       load. The next start would fail outright, which is worse than never
       recording the choice. */
    #[test]
    fn an_edit_that_would_break_the_config_is_not_written() {
        let file = temp("guard");
        let before = "dir = \"\"\"/tmp\nformat = x\"\"\"\n";
        std::fs::write(&file.path, before).unwrap();
        let err = save_format(std::path::Path::new(&file.path), "flac")
            .unwrap_err()
            .to_string();
        assert!(err.contains("no longer parses"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&file.path).unwrap(),
            before,
            "a config that would not load was written anyway"
        );
    }

    /* A config is often a symlink into a dotfiles repo. Renaming a temp file
       onto the link replaces the link with a plain file: the edit lands
       outside the repo and every later change there stops arriving. */
    #[test]
    #[cfg(unix)]
    fn saving_through_a_symlink_writes_the_file_it_points_at() {
        let dir = std::env::temp_dir().join(format!("earworm-symlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("dotfiles")).unwrap();
        let real = dir.join("dotfiles").join("config.toml");
        let link = dir.join("config.toml");
        std::fs::write(&real, "# kept\nformat = \"opus\"\n").unwrap();
        // 0600, as the README tells anyone with a key in the file to set.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        save_format(&link, "flac").unwrap();

        assert!(
            std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(),
            "the link was replaced by a plain file"
        );
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "# kept\nformat = \"flac\"\n");
        /* An atomic write starts from a fresh file, which is readable by
           everyone: the mode has to travel with the content or saving a
           setting quietly opens up a file holding an API key. */
        let mode = std::fs::metadata(&real).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the save widened who can read the config");

        // Nothing left beside it: a temp file is not a config, but it is one
        // more thing in a directory people keep by hand.
        let strays: Vec<String> = std::fs::read_dir(dir.join("dotfiles"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|name| name != "config.toml")
            .collect();
        assert!(strays.is_empty(), "{strays:?}");
        let _ = std::fs::remove_dir_all(&dir);
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
