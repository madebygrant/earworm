use std::path::PathBuf;

use clap::Parser;

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

    /// Extra arguments passed straight to yt-dlp
    #[arg(last = true)]
    pub extra: Vec<String>,
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

impl From<Cli> for Config {
    fn from(cli: Cli) -> Self {
        Config {
            url: cli.url.unwrap_or_default(),
            dir: expand(&cli.dir),
            parse: !cli.no_parse,
            m3u8: !cli.no_m3u8,
            cover: !cli.no_cover,
            lookup: !cli.no_lookup,
            fix: !cli.no_fix,
            extra: cli.extra,
            acoustid_key: std::env::var("ACOUSTID_API_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
        }
    }
}

impl Config {
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
