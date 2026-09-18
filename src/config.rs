use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::theme::{Palette, Themes};
use clap::{ArgMatches, Parser, parser::ValueSource};
use unicode_normalization::UnicodeNormalization;
use serde::Deserialize;

/* `about` with no value reads Cargo.toml's `description`, so that is what
   `--help` actually prints and editing the line below changes nothing there.
   Kept saying the same thing, since two answers to "what is this" is how they
   drift. */
/// Playlist in, library out. Turns a YouTube playlist into audio files with
/// verified tags, cover art and an .m3u8.
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

    /// Colour theme: warm, light, cool or neon
    #[arg(long, value_name = "NAME")]
    pub theme: Option<String>,

    /// Leave tracks already on disk in the format they were downloaded in,
    /// whatever `convert` says in the config
    #[arg(long)]
    pub no_convert: bool,

    /// Keep YouTube's own artist/track, skip title parsing
    #[arg(short = 'P', long)]
    pub no_parse: bool,

    /// Skip the .m3u8 playlist written beside the tracks
    #[arg(short = 'M', long)]
    pub no_m3u8: bool,

    /// Skip cover art: nothing fetched, nothing embedded in the tracks
    #[arg(short = 'C', long)]
    pub no_cover: bool,

    /// Skip AcoustID/Deezer identification, keep parsed tags
    #[arg(short = 'L', long)]
    pub no_lookup: bool,

    /// Skip the Apple Music (iTunes Search) fallback when Deezer finds nothing
    #[arg(long)]
    pub no_apple: bool,

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

    /// Don't ask GitHub whether a newer release exists
    #[arg(long)]
    pub no_update_check: bool,

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
    pub convert: Option<bool>,
    pub parse: Option<bool>,
    pub m3u8: Option<bool>,
    pub cover: Option<bool>,
    pub lookup: Option<bool>,
    pub apple: Option<bool>,
    pub fix: Option<bool>,
    pub rename: Option<bool>,
    pub album: Option<bool>,
    pub pick: Option<bool>,
    pub intro: Option<bool>,
    pub notify: Option<bool>,
    pub update_check: Option<bool>,
    pub extra: Option<Vec<String>>,
    pub acoustid_key: Option<String>,
    pub theme: Option<String>,
    /// Field name to `#rrggbb`, painted on top of whichever theme is named. A
    /// map rather than a struct so the error can name the field that was
    /// wrong, which a serde field error cannot.
    pub colors: Option<BTreeMap<String, String>>,
    /// Palettes of the user's own, joining the built-ins under their own
    /// names. Ordered by the map, so the `^t` walk is alphabetical after the
    /// four that ship and does not move when the file is reordered.
    pub themes: Option<BTreeMap<String, FileTheme>>,
}

/// One `[themes.<name>]` table.
#[derive(Deserialize, Default, Debug)]
pub struct FileTheme {
    /* The built-in to start from, so a theme that changes three colours says
       three. Built-ins only, and not another `[themes.*]`: a base that could
       name a peer is a base that can name a cycle, and resolving that is more
       machinery than starting from `cool` is worth. */
    pub base: Option<String>,
    /* Everything else, validated by name rather than by serde so the error
       can say which field was wrong and what the alternatives are. Flattened,
       so the table reads as a palette rather than as a palette in a box. */
    #[serde(flatten)]
    pub fields: BTreeMap<String, String>,
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

/* yt-dlp's name for the format, the extension it actually writes, and the
   ffmpeg encoders that produce it. The first two disagree for `vorbis` and
   `alac`, and both are needed: `run` passes the name to --audio-format while
   `scan` predicts filenames with the extension. wav and aac are deliberately
   absent: neither carries tags or cover art, so earworm would do a third of
   its job and report it as finished.

   The encoders are a list because the name is not the same on every ffmpeg
   build: `libvorbis` and `libopus` are the usual external ones and some
   builds carry only the native encoder under the bare format name. Whoever
   runs them tries the list in order. One table rather than two, so a format
   added here without a working encoder fails in the fixtures that read this
   same column rather than in somebody's folder. */
pub const FORMATS: [(&str, &str, &[&str]); 6] = [
    ("opus", "opus", &["libopus", "opus"]),
    ("m4a", "m4a", &["aac", "libfdk_aac"]),
    ("mp3", "mp3", &["libmp3lame", "mp3"]),
    ("flac", "flac", &["flac"]),
    ("vorbis", "ogg", &["libvorbis", "vorbis"]),
    ("alac", "m4a", &["alac"]),
];

pub const DEFAULT_FORMAT: &str = "opus";

/// The palette earworm has always drawn in, and the first stop on the `^t`
/// walk. A name rather than `Palette::default()` because the theme is now
/// tracked by name: the walk goes on from wherever the config started it.
pub const DEFAULT_THEME: &str = "warm";

/// The extension a format lands on disk with, which is not always its name.
/* Never guesses. `Config::build` refuses a format that is not in `FORMATS`
   and `set_format` picks out of the same table, so an unknown one here is a
   mistake in this file rather than in anything a user typed. The old fallback
   to "opus" meant such a mistake predicted the wrong filename for every
   track: the folder would read as empty and download again beside itself. */
pub fn extension(format: &str) -> &'static str {
    FORMATS
        .iter()
        .find(|(name, _, _)| *name == format)
        .map_or_else(
            || unreachable!("no extension for {format}, which FORMATS should have refused"),
            |(_, ext, _)| *ext,
        )
}

/// Whether `format` stores the audio without throwing anything away. Not a
/// claim about the recording: every one of these ultimately comes from
/// YouTube, which serves Opus or AAC.
pub fn lossless(format: &str) -> bool {
    matches!(format, "flac" | "alac")
}

/// The ffmpeg encoders that write `format`, in the order to try them. See
/// `FORMATS` for why it is a list.
pub fn encoders(format: &str) -> &'static [&'static str] {
    FORMATS
        .iter()
        .find(|(name, _, _)| *name == format)
        .map_or_else(
            || unreachable!("no encoder for {format}, which FORMATS should have refused"),
            |(_, _, codecs)| *codecs,
        )
}

pub fn save_format(path: &std::path::Path, format: &str) -> Result<()> {
    save_key(path, "format", &format!("\"{format}\""))
}

/* The four that ship plus every `[themes.<name>]`, resolved once. A name that
   is already taken is refused rather than shadowing: a `[themes.warm]` that
   silently won would make the built-in unreachable with nothing saying so,
   and a config that redefines `warm` is far more likely to be a mistake than
   an intent. */
fn registry(defined: Option<&BTreeMap<String, FileTheme>>) -> Result<Themes> {
    let mut themes = Themes::default();
    let Some(defined) = defined else {
        return Ok(themes);
    };
    for (name, spec) in defined {
        writable(name)?;
        if themes.has(name) {
            anyhow::bail!("theme \"{name}\" is already one earworm ships · give yours another name");
        }
        let palette = define(name, spec)
            .with_context(|| format!("in theme \"{name}\""))?;
        themes.add(name.clone(), palette);
    }
    Ok(themes)
}

/* A name has to survive being written back: `^t` puts it in the config as
   `theme = "<name>"`, and TOML has no escape inside that which `save_key`
   would produce. Refused here rather than at the save, because a theme that
   can be selected and then never remembered is a worse thing to discover than
   a config that would not start. Caught either way — `save_key` re-parses
   before it writes, so the file was never at risk — but the message there is
   "the edited config no longer parses", which sends the reader to inspect a
   config that is perfectly fine.

   Spaces are allowed: they round-trip, and the rule is the real constraint
   rather than a tidier one. */
fn writable(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("a theme needs a name · [themes.\"\"] has none");
    }
    if name.contains(['"', '\\']) || name.chars().any(char::is_control) {
        anyhow::bail!(
            "theme name {name:?} cannot be written back · no quotes, backslashes or control characters"
        );
    }
    Ok(())
}

/* One `[themes.<name>]` table. With a `base` it is a diff on a built-in; with
   none it has to say everything, because a half-written palette silently
   finishing itself from `warm` is the "quietly does not apply" problem with
   more places to hide. */
fn define(name: &str, spec: &FileTheme) -> Result<Palette> {
    let mut palette = match &spec.base {
        Some(base) => Palette::built_in(base).ok_or_else(|| {
            anyhow::anyhow!("unknown base \"{base}\", expected one of {}", built_in_names())
        })?,
        None => Palette::default(),
    };
    paint(&mut palette, &spec.fields)?;
    if spec.base.is_none() {
        /* Asked of the palette rather than of the table, so a field added to
           `Palette` is one a baseless theme is immediately told to supply
           rather than one it quietly inherits from warm. */
        let missing: Vec<&str> = palette
            .slots()
            .iter()
            .map(|(field, _)| *field)
            .chain(crate::theme::GROUNDS)
            .filter(|field| !spec.fields.contains_key(*field))
            .collect();
        if !missing.is_empty() {
            anyhow::bail!(
                "no base, so every colour is needed · {name} is missing {}",
                missing.join(", ")
            );
        }
    }
    Ok(palette)
}

/// `#rrggbb` values onto a palette, by field name. Shared by a theme
/// definition and the `[colors]` table, so both take the same names and
/// refuse the same way.
fn paint(palette: &mut Palette, fields: &BTreeMap<String, String>) -> Result<()> {
    for (field, value) in fields {
        let rgb = hex(value).with_context(|| format!("colour {field} = \"{value}\""))?;
        if !palette.set_field(field, rgb) {
            anyhow::bail!(
                "unknown colour \"{field}\", expected one of {}",
                Palette::field_names()
            );
        }
    }
    Ok(())
}

fn built_in_names() -> String {
    crate::theme::BUILT_INS
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

/* A name nobody defined is a startup error naming the ones that exist, not a
   silent fallback: a theme that quietly does not apply reads as a theme that
   does not work. Returns the base's name as well as the palette, because a
   `[colors]` table makes the two different things and `^t` walks by name. */
fn theme(
    themes: &Themes,
    name: Option<&str>,
    colors: Option<&BTreeMap<String, String>>,
) -> Result<(String, Palette, Vec<String>)> {
    let name = name.unwrap_or(DEFAULT_THEME).to_string();
    let mut palette = themes.named(&name).ok_or_else(|| {
        anyhow::anyhow!("unknown theme \"{name}\", expected one of {}", themes.names())
    })?;
    /* Applied on top of whatever was named, so an override is a diff rather
       than a whole palette: nobody should have to restate twelve colours to
       change one. A key or a value that cannot work stops the start, for the
       same reason an unknown format does. */
    if let Some(colors) = colors {
        paint(&mut palette, colors).context("in [colors]")?;
    }
    /* Measured whatever it is, not only when a `[colors]` table repainted it:
       a `[themes.*]` palette is just as unmeasurable by the suite, and the
       first version of this only looked at the override path, so a theme
       somebody defined could be illegible with nothing said. The four that
       ship cost nothing here — their ratios are a test, so they come back
       with nothing to report. */
    let warnings = palette.unreadable();
    Ok((name, palette, warnings))
}

/// `#rrggbb`, the only spelling worth supporting: it is what every palette,
/// picker and stylesheet in the world hands you.
fn hex(value: &str) -> Result<(u8, u8, u8)> {
    /* The `#` is required rather than merely tolerated: the error says
       `#rrggbb` and so does the README, and a second accepted spelling
       nobody documents is one more thing that works on one machine and not
       on the next. */
    let Some(digits) = value.strip_prefix('#') else {
        anyhow::bail!("not a #rrggbb colour");
    };
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("not a #rrggbb colour");
    }
    let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16);
    Ok((byte(0)?, byte(2)?, byte(4)?))
}

/// The palette `^t` landed on. The base theme only: a `[colors]` table stays
/// in the file and goes on repainting whatever the walk lands on.
pub fn save_theme(path: &std::path::Path, theme: &str) -> Result<()> {
    save_key(path, "theme", &format!("\"{theme}\""))
}

pub fn save_convert(path: &std::path::Path, on: bool) -> Result<()> {
    save_key(path, "convert", if on { "true" } else { "false" })
}

/// Writes one key back in place, leaving comments and every other line as
/// they were: the file is hand-edited, and rewriting it from the parsed
/// struct would drop the comments and flatten the formatting.
///
/// `value` is TOML as it should appear after the `=`, quotes included, so a
/// caller writing a string is the one that quotes it.
fn save_key(path: &std::path::Path, key: &str, value: &str) -> Result<()> {
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
    let line = format!("{key} = {value}");
    let mut out = String::new();
    let mut replaced = false;
    /* A key earworm writes is a top-level one, and in TOML those have to come
       before the first table or they belong to it. Appended at the end,
       `format = "flac"` landed inside `[colors]` and parsed as a colour named
       "format": the save reported success, recorded nothing, and the next
       start refused the file. The re-parse below does not catch it, because a
       string in a `BTreeMap<String, String>` is a perfectly valid colour
       table as far as serde is concerned. */
    let mut depth = 0usize;
    for existing in old.lines() {
        if names_key(existing, key) && !replaced {
            replaced = true;
            out.push_str(&line);
            out.push('\n');
            continue;
        }
        if !replaced && depth == 0 && opens_a_table(existing) {
            replaced = true;
            out.push_str(&line);
            out.push('\n');
        }
        /* Brackets outside a table header, so a multi-line `extra = [` is not
           mistaken for one on its `]`. A table header is balanced and leaves
           this where it was. */
        depth = depth
            .saturating_add(existing.matches('[').count())
            .saturating_sub(existing.matches(']').count());
        out.push_str(existing);
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
/* A `[table]` header, which is where the top-level keys have to stop. Judged
   on the whole trimmed line rather than the first character, so a value that
   happens to begin with a bracket is not one. The caller only asks while no
   multi-line value is open. */
fn opens_a_table(line: &str) -> bool {
    let line = line.trim();
    line.starts_with('[') && line.ends_with(']')
}

fn names_key(line: &str, key: &str) -> bool {
    let rest = line.trim_start();
    rest.strip_prefix(key)
        .or_else(|| rest.strip_prefix(&format!("\"{key}\"")))
        .or_else(|| rest.strip_prefix(&format!("'{key}'")))
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

/* The same filename reaches earworm in two forms. macOS stores these names
   composed and hands them over decomposed to anything copying the folder off
   the machine, and APFS ignores the difference on lookup, so the two are the
   same file here and different strings everywhere a string is compared. One
   pair, because a second copy of this is one that goes stale. */
pub fn composed(name: &str) -> String {
    name.nfc().collect()
}

/* Canonical, never compatibility: `nfkd` would rewrite a ligature or a
   full-width digit in a filename, which is a different file. */
pub fn decomposed(name: &str) -> String {
    name.nfd().collect()
}

/* Written beside the target and renamed over it, so a reader sees either the
   whole old file or the whole new one. `fs::write` truncates first, and a
   crash or a full disk between the truncate and the write leaves a file the
   next start refuses to read. Shared with the update cache: one answer to
   "how does earworm replace a file it owns", or the weaker one gets copied
   into the next place that cannot afford it. */
pub fn write_atomically(path: &std::path::Path, text: &str) -> Result<()> {
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
    /* Whether tracks already on disk follow `format` when their playlist is
       next synced. Off unless the file turns it on, which is the opposite of
       every other setting here and deliberate: `format` means "format for new
       downloads", and a plain format change that rewrote the library would
       turn one pick into an unattended rewrite of every folder under --dir.
       Only a sync acts on it; opening a folder from the library converts
       nothing, because that path never touches the network or the encoder. */
    pub convert: bool,
    /* The folder this run must write into, for a playlist somebody has
       renamed. `None` is every other run, where yt-dlp names the folder from
       the playlist's title. Not a setting and not in the file: it belongs to
       the playlist being synced, and `Msg::Restart` territory rather than
       config, which is why every path that starts a run sets or clears it. */
    pub folder: Option<String>,
    /// Where a setting changed in the tool gets written back. `None` under
    /// --no-config, which asked for the file to be left out of the run and
    /// so cannot be the place a choice is remembered.
    pub config_file: Option<PathBuf>,
    pub parse: bool,
    pub m3u8: bool,
    pub cover: bool,
    pub lookup: bool,
    pub apple: bool,
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
    /// Ask GitHub about a newer release, at most once a day.
    pub update_check: bool,
    pub extra: Vec<String>,
    pub acoustid_key: Option<String>,
    /// The palette every draw reads. Resolved here so a bad name or a bad
    /// colour stops the start rather than reaching a frame.
    /* Deliberately absent from `describe()`, unlike every other setting on
       this struct. `^t` changes the theme from inside the tool and nothing
       re-sends the settings string, so a copy of the name in there is one
       that goes stale the first time the key is pressed. The help overlay
       reads it off the UI's own state. */
    pub theme: Palette,
    /* What it is called, tracked beside the colours rather than derived from
       them: a `[colors]` table makes the palette match no entry in the
       registry, and the walk still has to know where it is. */
    pub theme_name: String,
    /// Every palette this run can reach, the built-ins plus `[themes.*]`.
    pub themes: Themes,
    /// Colours the config repainted that measure badly on their own ground.
    /// Said once and never again: it is the user's screen.
    pub theme_warnings: Vec<String>,
    /// Whether a `[colors]` table is in the file, which `^t` has to mention:
    /// the walk moves the base palette and the table repaints whatever it
    /// lands on, so the name in the flash is not the whole story.
    pub theme_overridden: bool,
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
        /* Same shape as `off` and the same rule about the command line only
           ever switching something off, but defaulting the other way. The one
           setting that rewrites files already on disk does not get to be on
           because nobody said otherwise. */
        let opt_in = |id: &str, from_file: Option<bool>| {
            if given(id) { false } else { from_file.unwrap_or(false) }
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
        if !FORMATS.iter().any(|(name, _, _)| *name == format) {
            let known: Vec<&str> = FORMATS.iter().map(|(name, _, _)| *name).collect();
            anyhow::bail!("unknown format \"{format}\", expected one of {}", known.join(", "));
        }

        let themes = registry(file.themes.as_ref())?;
        let (theme_name, palette, theme_warnings) = theme(
            &themes,
            cli.theme.as_deref().or(file.theme.as_deref()),
            file.colors.as_ref(),
        )?;
        let theme_overridden = file.colors.as_ref().is_some_and(|c| !c.is_empty());

        Ok(Config {
            url: cli.url.unwrap_or_default(),
            dir: expand(&dir),
            folder: None,
            format,
            convert: opt_in("no_convert", file.convert),
            config_file,
            parse: off("no_parse", file.parse),
            m3u8: off("no_m3u8", file.m3u8),
            cover: off("no_cover", file.cover),
            lookup: off("no_lookup", file.lookup),
            apple: off("no_apple", file.apple),
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
            update_check: off("no_update_check", file.update_check),
            extra,
            theme: palette,
            theme_name,
            themes,
            theme_warnings,
            theme_overridden,
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
            "format {} · convert {} · parse {} · lookup {} · apple {} · cover {} · m3u8 {} · prompts {} · rename {}",
            self.format,
            on(self.convert),
            on(self.parse),
            on(self.lookup),
            on(self.apple),
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

    /* The one setting here that is off unless the file says otherwise, and
       the reason is that it rewrites files already on disk: a `format` change
       alone must never do that. The flag still only switches it off, which is
       the rule every other setting follows. */
    #[test]
    fn convert_is_off_unless_the_file_turns_it_on() {
        assert!(!build("", &[]).convert, "on with nothing asking for it");
        assert!(!build("convert = false\n", &[]).convert);
        assert!(build("convert = true\n", &[]).convert);
        // The command line can switch it off and never on.
        assert!(!build("convert = true\n", &["--no-convert"]).convert);
        // And it is independent of the format, which is the whole point.
        let cfg = build("format = \"flac\"\n", &[]);
        assert_eq!(cfg.format, "flac");
        assert!(!cfg.convert, "picking a format turned conversion on");
    }

    /// Written back the same way the format is, so the next start reads it.
    #[test]
    fn saving_convert_keeps_the_rest_of_the_file() {
        let mut file = temp("saveconvert");
        write!(file.handle, "# mine\nformat = \"flac\"\n").unwrap();
        save_convert(std::path::Path::new(&file.path), true).unwrap();

        let back = build_at(&file.path);
        assert!(back.convert);
        assert_eq!(back.format, "flac");
        let text = std::fs::read_to_string(&file.path).unwrap();
        assert!(text.contains("# mine"), "the file was rewritten: {text:?}");

        // And off again, in place rather than appended a second time.
        save_convert(std::path::Path::new(&file.path), false).unwrap();
        let text = std::fs::read_to_string(&file.path).unwrap();
        assert_eq!(text.matches("convert").count(), 1, "{text:?}");
        assert!(!build_at(&file.path).convert);
    }

    /// Every format has to name an encoder, or converting to it fails at the
    /// point somebody is waiting on it rather than here.
    #[test]
    fn every_format_names_an_encoder() {
        for (name, _, codecs) in FORMATS {
            assert!(!codecs.is_empty(), "{name} has no encoder");
            assert_eq!(encoders(name), codecs, "{name}");
        }
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

    /// The same throwaway file, but for the cases that must not build at all.
    fn refuse(toml: &str, args: &[&str]) -> String {
        let mut file = temp("refuse");
        write!(file.handle, "{toml}").unwrap();
        let mut argv = vec!["earworm".to_string(), "--config".into(), file.path.clone()];
        argv.extend(args.iter().map(|a| a.to_string()));
        let matches = Cli::command().get_matches_from(argv);
        match Config::build(Cli::from_arg_matches(&matches).unwrap(), &matches) {
            Ok(_) => panic!("it was accepted"),
            Err(err) => format!("{err:#}"),
        }
    }

    /* A theme nobody ships is a startup error naming the ones that exist,
       exactly like an unknown format: a theme that quietly does not apply
       reads as a theme that does not work. */
    #[test]
    fn an_unknown_theme_stops_the_start_and_names_the_ones_that_exist() {
        let err = refuse("theme = \"midnight\"\n", &[]);
        assert!(err.contains("unknown theme \"midnight\""), "{err}");
        for shipped in ["warm", "light", "cool", "neon"] {
            assert!(err.contains(shipped), "{shipped} went unlisted: {err}");
        }
        // And the same name off the command line, which is the other route in.
        let flag = refuse("", &["--theme", "midnight"]);
        assert!(flag.contains("unknown theme"), "{flag}");
    }

    #[test]
    fn the_command_line_theme_beats_the_file() {
        let cfg = build("theme = \"neon\"\n", &["--theme", "cool"]);
        assert_eq!(cfg.theme, crate::theme::COOL);
        // And with nothing said anywhere, the palette earworm has always had.
        assert_eq!(build("", &[]).theme, crate::theme::WARM);
    }

    /* An override is a diff on top of a named base, not a whole palette:
       changing one colour must not mean restating ten. */
    #[test]
    fn colors_repaint_one_slot_of_the_named_theme() {
        let cfg = build("theme = \"neon\"\n[colors]\ncursor = \"#00ff88\"\n", &[]);
        assert_eq!(cfg.theme.cursor, ratatui::style::Color::Rgb(0, 255, 136));
        // Everything it did not name is still neon's.
        assert_eq!(cfg.theme.text, crate::theme::NEON.text);
        assert_eq!(cfg.theme.accent, crate::theme::NEON.accent);
        /* The name is the base the overrides were painted on, not "custom":
           that is what `^t` walks on from and what gets written back. */
        assert_eq!(cfg.theme_name, "neon");
        assert!(cfg.theme_overridden, "the flash would not mention the table");
    }

    /* A key or a value that cannot work stops the start, for the same reason
       an unknown theme does. Both messages name what it should have been:
       a colour that silently does not apply is indistinguishable from a
       theme system that does not work. */
    #[test]
    fn a_colour_that_cannot_work_stops_the_start_and_says_what_it_wanted() {
        for bad in ["00ff88", "#00ff8", "#00ff8g", "red", ""] {
            let err = refuse(&format!("[colors]\ncursor = \"{bad}\"\n"), &[]);
            assert!(err.contains("#rrggbb"), "{bad:?} gave {err}");
            assert!(err.contains("cursor"), "{bad:?} did not name the slot: {err}");
        }
        let slot = refuse("[colors]\nbackdrop = \"#00ff88\"\n", &[]);
        assert!(slot.contains("unknown colour \"backdrop\""), "{slot}");
        assert!(slot.contains("[colors]"), "it did not say which table: {slot}");
        // Every field that does work is offered in its place, gradient included.
        for name in ["text", "accent", "cursor", "unsure", "ink", "near", "far"] {
            assert!(slot.contains(name), "{name} went unlisted: {slot}");
        }
    }

    /// Twelve fields is a lot to type in a test that is about something else.
    fn whole_palette(name: &str) -> String {
        let mut out = format!("[themes.{name}]\n");
        for field in ["text", "accent", "cursor", "warn", "error", "muted", "rule",
                      "surface", "unsure", "ink", "near", "far"] {
            out.push_str(&format!("{field} = \"#101010\"\n"));
        }
        out
    }

    /* The point of the feature: a palette of your own, under its own name, in
       the same list as the four that ship. With a base it is a diff, so
       changing two colours means writing two. */
    #[test]
    fn a_defined_theme_can_be_named_and_is_a_diff_on_its_base() {
        let cfg = build(
            "theme = \"midnight\"\n[themes.midnight]\nbase = \"cool\"\naccent = \"#ff5cd5\"\nnear = \"#1a0a2c\"\n",
            &[],
        );
        assert_eq!(cfg.theme_name, "midnight");
        assert_eq!(cfg.theme.accent, ratatui::style::Color::Rgb(255, 92, 213));
        // The gradient is nameable like any other colour.
        assert_eq!(cfg.theme.near, (26, 10, 44));
        // Everything it did not name is still cool's, including the far end.
        assert_eq!(cfg.theme.cursor, crate::theme::COOL.cursor);
        assert_eq!(cfg.theme.far, crate::theme::COOL.far);
        // And it is reachable by name from the command line too.
        assert_eq!(
            build("[themes.midnight]\nbase = \"cool\"\n", &["--theme", "midnight"]).theme_name,
            "midnight"
        );
    }

    /* A defined theme joins the walk after the four that ship, so `^t` can
       reach it: a theme only `--theme` could select would be half a feature. */
    #[test]
    fn a_defined_theme_joins_the_walk_after_the_shipped_ones() {
        let cfg = build(
            "[themes.midnight]\nbase = \"cool\"\n[themes.aurora]\nbase = \"neon\"\n",
            &[],
        );
        // Alphabetical after the built-ins, which is what a BTreeMap yields.
        assert_eq!(cfg.themes.after("neon").0, "aurora");
        assert_eq!(cfg.themes.after("aurora").0, "midnight");
        assert_eq!(cfg.themes.after("midnight").0, "warm", "the walk did not wrap");
    }

    /* No base means the table has to say everything. A half-written palette
       quietly finishing itself from warm is the "quietly does not apply"
       problem with more places for it to hide. */
    #[test]
    fn a_theme_with_no_base_must_give_every_colour() {
        let whole = build(&format!("theme = \"flat\"\n{}", whole_palette("flat")), &[]);
        assert_eq!(whole.theme.text, ratatui::style::Color::Rgb(16, 16, 16));
        assert_eq!(whole.theme.near, (16, 16, 16));

        let err = refuse("[themes.flat]\ntext = \"#101010\"\n", &[]);
        assert!(err.contains("no base"), "{err}");
        // Named, so the fix is the message rather than a hunt through docs.
        for missing in ["accent", "cursor", "surface", "ink", "near", "far"] {
            assert!(err.contains(missing), "{missing} went unnamed: {err}");
        }
        assert!(!err.contains("text,"), "it asked again for the one given: {err}");
    }

    /* Shadowing a built-in would make it unreachable with nothing saying so,
       and a config that redefines `warm` is far likelier to be a mistake. */
    #[test]
    fn a_defined_theme_may_not_take_a_shipped_name() {
        let err = refuse("[themes.warm]\nbase = \"cool\"\n", &[]);
        assert!(err.contains("already one earworm ships"), "{err}");
        assert!(err.contains("warm"), "{err}");
    }

    /* The base is a built-in and only a built-in: one that could name a peer
       could name a cycle. The error says which names do work. */
    #[test]
    fn an_unknown_base_is_refused_and_names_the_ones_that_work() {
        let err = refuse("[themes.mine]\nbase = \"midnight\"\n", &[]);
        assert!(err.contains("unknown base \"midnight\""), "{err}");
        assert!(err.contains("mine"), "it did not say whose base: {err}");
        for shipped in ["warm", "light", "cool", "neon"] {
            assert!(err.contains(shipped), "{shipped} went unlisted: {err}");
        }
        // And a peer is not a base, however real its name is.
        let peer = refuse(
            "[themes.one]\nbase = \"cool\"\n[themes.two]\nbase = \"one\"\n",
            &[],
        );
        assert!(peer.contains("unknown base \"one\""), "{peer}");
    }

    /* `[colors]` sits on top of whatever `theme` names, a defined one
       included: the two features compose rather than competing. */
    #[test]
    fn colors_repaint_a_defined_theme_too() {
        let cfg = build(
            "theme = \"midnight\"\n[themes.midnight]\nbase = \"cool\"\naccent = \"#ff5cd5\"\n[colors]\naccent = \"#00ff88\"\n",
            &[],
        );
        assert_eq!(cfg.theme.accent, ratatui::style::Color::Rgb(0, 255, 136));
        // The name is still the theme's: that is what `^t` walks on from.
        assert_eq!(cfg.theme_name, "midnight");
        assert!(cfg.theme_overridden);
    }

    /* A defined theme cannot be measured by the suite, so it is measured at
       startup instead and the reader is told. Never refused: it is their
       screen, and the four that ship are the ones earworm vouches for. */
    #[test]
    fn a_defined_theme_that_measures_badly_starts_and_says_so() {
        let cfg = build(
            "theme = \"murk\"\n[themes.murk]\nbase = \"cool\"\ntext = \"#1b2028\"\n",
            &[],
        );
        assert_eq!(cfg.theme_name, "murk");
        assert!(
            cfg.theme_warnings.iter().any(|w| w.contains("text")),
            "it started without a word: {:?}",
            cfg.theme_warnings
        );
    }

    /* A key earworm writes is a top-level one, and TOML puts those before the
       first table. Appended at the end it landed inside `[colors]`, parsed as
       a colour named "format", and the re-parse waved it through because a
       string in a colour table is a valid colour table. The save said it
       worked, recorded nothing, and the next start refused the file. */
    #[test]
    fn a_saved_key_lands_above_the_tables_and_not_inside_one() {
        let file = temp("abovetables");
        std::fs::write(
            &file.path,
            "# mine\ndir = \"/tmp/m\"\n\n[colors]\ncursor = \"#00ff88\"\n",
        )
        .unwrap();
        save_format(std::path::Path::new(&file.path), "flac").unwrap();

        let text = std::fs::read_to_string(&file.path).unwrap();
        let at = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("no {needle}: {text}"));
        assert!(at("format =") < at("[colors]"), "it went into the table: {text}");
        assert!(at("# mine") < at("format ="), "it went above the comment: {text}");

        // The only thing that proves it: read it back through the real parser.
        let cfg = build_at(&file.path);
        assert_eq!(cfg.format, "flac");
        assert_eq!(cfg.theme.cursor, ratatui::style::Color::Rgb(0, 255, 136));
        assert_eq!(cfg.dir, PathBuf::from("/tmp/m"), "another setting was lost");
    }

    /* The same for a file whose first table is a theme, which is the shape
       that made this findable: the old behaviour wrote `theme = "midnight"`
       into `[themes.midnight]` as a colour called "theme". */
    #[test]
    fn a_saved_theme_lands_above_the_theme_tables() {
        let file = temp("abovethemes");
        std::fs::write(
            &file.path,
            "format = \"flac\"\n[themes.midnight]\nbase = \"cool\"\n",
        )
        .unwrap();
        save_theme(std::path::Path::new(&file.path), "midnight").unwrap();
        assert_eq!(build_at(&file.path).theme_name, "midnight");
    }

    /* A line inside a value is not a table header however much it looks like
       one, which is the whole reason the scan counts brackets rather than
       reading the first character. The plain case first: a multi-line array
       must come through untouched and the key must still land above the real
       table. */
    #[test]
    fn a_multi_line_value_is_not_mistaken_for_a_table() {
        let file = temp("multiline");
        std::fs::write(
            &file.path,
            "extra = [\n  \"--sleep-requests\",\n  \"1\",\n]\n\n[colors]\ncursor = \"#00ff88\"\n",
        )
        .unwrap();
        save_format(std::path::Path::new(&file.path), "flac").unwrap();
        let cfg = build_at(&file.path);
        assert_eq!(cfg.format, "flac");
        assert_eq!(cfg.extra, vec!["--sleep-requests".to_string(), "1".to_string()]);

        /* And the case the counting is actually for: a multi-line string
           holding a line that reads exactly like a header. Contrived, but it
           is valid TOML, and inserting a key into the middle of somebody's
           string value is the one outcome here that loses their data rather
           than just misplacing earworm's. */
        let tricky = temp("multilinestr");
        std::fs::write(
            &tricky.path,
            "extra = [\"\"\"\n[colors]\n\"\"\"]\n\n[colors]\ncursor = \"#00ff88\"\n",
        )
        .unwrap();
        save_format(std::path::Path::new(&tricky.path), "flac").unwrap();
        let cfg = build_at(&tricky.path);
        assert_eq!(cfg.format, "flac");
        assert_eq!(cfg.extra, vec!["[colors]\n".to_string()], "the string was written into");
        assert_eq!(cfg.theme.cursor, ratatui::style::Color::Rgb(0, 255, 136));
    }

    /// A file with no tables at all still appends, which is what it always did.
    #[test]
    fn a_file_without_tables_gains_the_key_at_the_end() {
        let file = temp("notables");
        std::fs::write(&file.path, "dir = \"/tmp/m\"\n").unwrap();
        save_format(std::path::Path::new(&file.path), "flac").unwrap();
        let text = std::fs::read_to_string(&file.path).unwrap();
        assert!(text.trim_end().ends_with("format = \"flac\""), "{text}");
        assert_eq!(build_at(&file.path).format, "flac");
    }

    /* `^t` writes the name into the config as `theme = "<name>"`, so a name
       that cannot be spelled there is one that can be selected and then never
       remembered. Caught at the definition, where the message can say what is
       wrong with the name, rather than at the save, where `save_key`'s
       re-parse catches it and blames a config that is perfectly fine. */
    #[test]
    fn a_theme_name_that_cannot_be_written_back_is_refused() {
        for bad in ["say\\\"hi", "back\\\\slash", ""] {
            let err = refuse(&format!("[themes.'{bad}']\nbase = \"cool\"\n"), &[]);
            assert!(
                err.contains("cannot be written back") || err.contains("needs a name"),
                "{bad:?} was accepted with: {err}"
            );
        }
        // A space round-trips, so the rule is the real constraint and no more.
        let cfg = build("[themes.'my theme']\nbase = \"cool\"\n", &["--theme", "my theme"]);
        assert_eq!(cfg.theme_name, "my theme");
    }

    /* The whole point of `^t` writing a name down is the next launch opening
       on it, and for a theme the config defined that means the written name
       has to resolve against `[themes.*]` on the way back in. Through the real
       parser, which is the only thing that proves it. */
    #[test]
    fn a_walked_to_theme_of_your_own_survives_the_round_trip() {
        let file = temp("definedround");
        std::fs::write(
            &file.path,
            "format = \"flac\"\n[themes.midnight]\nbase = \"cool\"\naccent = \"#ff5cd5\"\n",
        )
        .unwrap();
        save_theme(std::path::Path::new(&file.path), "midnight").unwrap();

        let cfg = build_at(&file.path);
        assert_eq!(cfg.theme_name, "midnight");
        assert_eq!(cfg.theme.accent, ratatui::style::Color::Rgb(255, 92, 213));
        // Still a diff on cool, so the table came back whole and not just its name.
        assert_eq!(cfg.theme.cursor, crate::theme::COOL.cursor);
        assert_eq!(cfg.format, "flac", "another setting was lost");
        // And it is still in the walk it was written out of.
        assert_eq!(cfg.themes.after("neon").0, "midnight");
    }

    /* A colour that merely measures badly starts anyway and says so: it is
       the user's screen and their eyes, and a warning that blocks is one
       people learn to route around. */
    #[test]
    fn an_unreadable_override_warns_rather_than_refusing() {
        // Near-black text on the warm theme's near-black ground.
        let cfg = build("[colors]\ntext = \"#1a1a1a\"\n", &[]);
        assert_eq!(cfg.theme.text, ratatui::style::Color::Rgb(26, 26, 26));
        assert!(
            cfg.theme_warnings.iter().any(|w| w.contains("text")),
            "it started without a word: {:?}",
            cfg.theme_warnings
        );
        // And a palette nobody touched warns about nothing.
        assert!(build("theme = \"cool\"\n", &[]).theme_warnings.is_empty());
    }

    /* `^t` writes one key back into a file somebody wrote by hand, which is
       the same promise `format` makes: the comments and every other key come
       through, and the file still parses afterwards. */
    #[test]
    fn saving_a_theme_keeps_the_rest_of_the_file() {
        let file = temp("savetheme");
        std::fs::write(
            &file.path,
            "# mine\nformat = \"flac\"\ntheme = \"warm\"\ndir = \"/tmp/m\"\n",
        )
        .unwrap();
        save_theme(std::path::Path::new(&file.path), "neon").unwrap();

        let text = std::fs::read_to_string(&file.path).unwrap();
        assert!(text.contains("# mine"), "the comment went: {text}");
        assert!(text.contains("theme = \"neon\""), "{text}");
        assert_eq!(text.matches("theme =").count(), 1, "a second key: {text}");
        // Read back through the real parser, which is what proves it survives.
        let cfg = build_at(&file.path);
        assert_eq!(cfg.theme, crate::theme::NEON);
        assert_eq!(cfg.format, "flac", "another setting was lost");
    }

    /// A file with no `theme` line at all gains one rather than being ignored.
    #[test]
    fn saving_a_theme_into_a_file_that_never_had_one_adds_it() {
        let file = temp("addtheme");
        std::fs::write(&file.path, "format = \"opus\"\n").unwrap();
        save_theme(std::path::Path::new(&file.path), "light").unwrap();
        assert_eq!(build_at(&file.path).theme, crate::theme::LIGHT);
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
        for (name, ext, _) in FORMATS {
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
        assert!(names_key("format = \"opus\"", "format"));
        assert!(names_key("  \"format\"  = 'mp3'", "format"));
        assert!(names_key("'format'= 1", "format"));
        assert!(!names_key("formats = 1", "format"));
        assert!(!names_key("formatting = 1", "format"));
        assert!(!names_key("# format = opus", "format"));
        // The same rule has to hold for the second key that goes through it.
        assert!(names_key("convert = true", "convert"));
        assert!(!names_key("converted = true", "convert"));
        assert!(!names_key("format = \"opus\"", "convert"));
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

    /* The Apple fallback is on by default like the gate: the command line can
    only switch it off, and the file is the only way to hold the choice. */
    #[test]
    fn the_apple_fallback_is_on_unless_something_says_otherwise() {
        assert!(build("", &[]).apple, "a bare run lost the fallback");
        assert!(!build("", &["--no-apple"]).apple, "the flag did not switch it off");
        assert!(!build("apple = false\n", &[]).apple, "the file could not switch it off");
        assert!(
            !build("apple = true\n", &["--no-apple"]).apple,
            "the command line lost to the file"
        );
    }

    /* The update check phones home, so it is the one default worth a way to
    refuse: same negation shape as everything else — the command line can only
    switch it off, and the file is the only way to hold the choice. */
    #[test]
    fn the_update_check_is_on_unless_something_says_otherwise() {
        assert!(build("", &[]).update_check, "a bare run lost the check");
        assert!(
            !build("", &["--no-update-check"]).update_check,
            "the flag did not switch it off"
        );
        assert!(
            !build("update_check = false\n", &[]).update_check,
            "the file could not switch it off"
        );
        assert!(
            !build("update_check = true\n", &["--no-update-check"]).update_check,
            "the command line lost to the file"
        );
    }
}
