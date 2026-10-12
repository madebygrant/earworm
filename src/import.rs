use std::collections::HashSet;
use std::path::Path;

use crate::config::expand;
use crate::worker::{HOSTS, MUSIC_HOST, split_url};

// YouTube's `watch_videos` queue ignores every id past this one, without saying so. Refused rather
// than cut, because a re-import of a longer file would offer what fell off the end to `D`.
pub const QUEUE_MAX: usize = 50;
const QUEUE: &str = "https://www.youtube.com/watch_videos?video_ids=";
// A playlist file is a few kilobytes; anything far bigger was typed by mistake.
const MAX_BYTES: u64 = 1 << 20;
// As `playlists::MAX_NAME`: room for a suffix and an extension inside a 255-byte filename.
const MAX_NAME: usize = 200;

/// The tracks a markdown file links to, in the order they first appear.
#[derive(Debug, PartialEq, Eq)]
pub struct Found {
    pub name: Option<String>,
    pub ids: Vec<String>,
    /// Whether any link was a `music.youtube.com` one, which the cheat lock covers.
    pub music: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Fault {
    /// The answer names no file, so it is a mistyped link rather than a bad import.
    NotAFile,
    /// The answer reads as a path, and nothing is there.
    Missing,
    Folder,
    TooBig,
    TooMany(usize),
    Unreadable,
    NoLinks,
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Fault::NotAFile => f.write_str("Not a YouTube link"),
            Fault::Missing => f.write_str("No file at that path"),
            Fault::Folder => f.write_str("That's a folder, not a file"),
            Fault::TooBig => f.write_str("That file is too big to be a playlist"),
            Fault::TooMany(n) => write!(f, "That file has {n} links  ·  a YouTube queue holds {QUEUE_MAX}"),
            Fault::Unreadable => f.write_str("That file can't be read"),
            Fault::NoLinks => f.write_str("No YouTube links in that file"),
        }
    }
}

impl Found {
    /// The queue link yt-dlp lists as one playlist.
    pub fn url(&self) -> String {
        format!("{QUEUE}{}", self.ids.join(","))
    }
}

/// The ids an import's recorded URL holds, or `None` when the URL is not an import's.
pub fn queue_ids(url: &str) -> Option<Vec<String>> {
    url.starts_with(QUEUE).then(|| parse(url).ids)
}

pub fn read(answer: &str) -> Result<Found, Fault> {
    let typed = unquote(answer);
    let path = expand(&typed);
    let pathlike = typed.starts_with(['/', '~', '.']) || typed.ends_with(".md");
    let meta = std::fs::metadata(&path).map_err(|_| if pathlike { Fault::Missing } else { Fault::NotAFile })?;
    if meta.is_dir() {
        return Err(Fault::Folder);
    }
    if !meta.is_file() {
        return Err(Fault::NotAFile);
    }
    if meta.len() > MAX_BYTES {
        return Err(Fault::TooBig);
    }
    let text = std::fs::read_to_string(&path).map_err(|_| Fault::Unreadable)?;
    let mut found = parse(&text);
    if found.ids.is_empty() {
        return Err(Fault::NoLinks);
    }
    if found.ids.len() > QUEUE_MAX {
        return Err(Fault::TooMany(found.ids.len()));
    }
    if found.name.is_none() {
        found.name = stem(&path);
    }
    Ok(found)
}

pub fn parse(text: &str) -> Found {
    let mut fenced = false;
    let name = text
        .lines()
        .map(str::trim)
        .find_map(|line| {
            fenced ^= line.starts_with("```") || line.starts_with("~~~");
            line.strip_prefix("# ").filter(|_| !fenced)
        })
        .and_then(|heading| folder_name(heading.trim_end_matches('#')));
    let mut found = Found { name, ids: Vec::new(), music: false };
    let mut seen = HashSet::new();
    // Not a comma: `video_ids=` separates its ids with them.
    for token in text.split(|c: char| c.is_whitespace() || "|()<>[]\"'`*".contains(c)) {
        let Some((host, path)) = link(token) else { continue };
        let path = path.split('#').next().unwrap_or("");
        let (route, query) = path.split_once('?').unwrap_or((path, ""));
        let param = |key: &str| query.split('&').find_map(|p| p.strip_prefix(key));
        let ids: Vec<&str> = match route.trim_end_matches('/') {
            "watch" => param("v=").into_iter().collect(),
            "watch_videos" => param("video_ids=").map(|v| v.split(',').collect()).unwrap_or_default(),
            id if host == "youtu.be" => vec![id],
            other => match other.split_once('/') {
                Some(("shorts" | "live", id)) => vec![id],
                _ => Vec::new(),
            },
        };
        let mut any = false;
        for id in ids.into_iter().filter(|id| video_id(id)) {
            any = true;
            if seen.insert(id) {
                found.ids.push(id.to_string());
            }
        }
        found.music |= any && host == MUSIC_HOST;
    }
    found
}

fn link(token: &str) -> Option<(String, &str)> {
    let token = token.trim_end_matches(['.', ',', ';', ':', '!', '?']);
    if !token.contains("://") {
        return None;
    }
    let (host, path) = split_url(token)?;
    HOSTS.contains(&host.as_str()).then_some((host, path))
}

fn video_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

// The folder is named after this, and yt-dlp's own title for a queue is "Untitled List".
// A slash would move it and a leading dot would hide it, as `rename_shelf` refuses.
fn folder_name(raw: &str) -> Option<String> {
    let name = raw.replace(['/', '\\'], "-");
    let mut name = name.trim().trim_start_matches('.').trim();
    if name.len() > MAX_NAME {
        let cut = (0..=MAX_NAME).rev().find(|&i| name.is_char_boundary(i)).unwrap_or(0);
        name = name[..cut].trim_end();
    }
    (!name.is_empty()).then(|| name.to_string())
}

// `mix.ytmusic.md` is named `mix`, but `v1.2 mix.md` keeps its dot.
fn stem(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    let stem = match stem.rsplit_once('.') {
        Some((head, ext)) if !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphabetic()) => head,
        _ => &stem,
    };
    folder_name(stem)
}

// A path dragged into a terminal arrives quoted or with its spaces escaped, depending on the terminal.
fn unquote(answer: &str) -> String {
    let answer = answer.trim();
    for quote in ['\'', '"'] {
        if let Some(inner) = answer.strip_prefix(quote).and_then(|a| a.strip_suffix(quote)) {
            return inner.to_string();
        }
    }
    let mut plain = String::with_capacity(answer.len());
    let mut chars = answer.chars();
    while let Some(c) = chars.next() {
        plain.push(if c == '\\' { chars.next().unwrap_or(c) } else { c });
    }
    plain
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPORT: &str = "# powerful dystopian

## Play as a queue (no login)

- https://www.youtube.com/watch_videos?video_ids=UEZ-apKKDKQ,HE7mpRV0fdE

## Tracks

| # | Song | Artist | Link |
|---|---|---|---|
| 1 | Remembrance | Tommee Profitt, Fleurie | https://music.youtube.com/watch?v=UEZ-apKKDKQ |
| 2 | Chosen (feat. Svrcina) | Generdyn, SVRCINA | https://music.youtube.com/watch?v=HE7mpRV0fdE |
| 3 | Is This the End | Joseph William Morgan | https://music.youtube.com/watch?v=_ZfNTeM-cKs |
";

    #[test]
    fn an_export_yields_its_heading_and_every_id_once_in_order() {
        let found = parse(EXPORT);
        assert_eq!(found.name.as_deref(), Some("powerful dystopian"));
        assert_eq!(found.ids, ["UEZ-apKKDKQ", "HE7mpRV0fdE", "_ZfNTeM-cKs"]);
        assert!(found.music);
        assert_eq!(
            found.url(),
            "https://www.youtube.com/watch_videos?video_ids=UEZ-apKKDKQ,HE7mpRV0fdE,_ZfNTeM-cKs"
        );
    }

    #[test]
    fn links_to_other_hosts_and_lookalike_paths_are_ignored() {
        let text = "[a](https://youtu.be/dQw4w9WgXcQ#t=30) **https://youtu.be/UEZ-apKKDKQ**, https://evil.com/youtube.com/watch?v=aaaaaaaaaaa \
                    https://www.youtube.com/watch?v=short https://www.youtube.com/playlist?list=PL1";
        let found = parse(text);
        assert_eq!(found.ids, ["dQw4w9WgXcQ", "UEZ-apKKDKQ"]);
        assert!(!found.music, "only a music link sets the lock");
    }

    #[test]
    fn shorts_and_live_links_count() {
        let found = parse("https://www.youtube.com/shorts/dQw4w9WgXcQ https://youtube.com/live/UEZ-apKKDKQ?si=x");
        assert_eq!(found.ids, ["dQw4w9WgXcQ", "UEZ-apKKDKQ"]);
    }

    #[test]
    fn the_heading_is_the_first_one_outside_a_code_block() {
        let text = "```\n# not this\n```\n# Night drive\n";
        assert_eq!(parse(text).name.as_deref(), Some("Night drive"));
    }

    #[test]
    fn a_heading_cannot_move_or_hide_the_folder() {
        assert_eq!(parse("# ../a/b ##").name.as_deref(), Some("-a-b"));
        assert_eq!(parse("#   \n").name, None);
        let long = format!("# {}", "é".repeat(150));
        assert_eq!(parse(&long).name.map(|n| n.len()), Some(MAX_NAME), "a heading past the filename limit");
    }

    #[test]
    fn a_dragged_path_is_read_quoted_or_escaped() {
        let dir = std::env::temp_dir().join(format!("earworm-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("powerful dystopian (v1.2).ytmusic.md");
        std::fs::write(&file, "https://youtu.be/dQw4w9WgXcQ").unwrap();
        let plain = file.display().to_string();
        let escaped = plain.replace(' ', "\\ ").replace('(', "\\(").replace(')', "\\)");
        for typed in [format!("'{plain}' "), escaped, plain.clone()] {
            let found = read(&typed).unwrap_or_else(|f| panic!("{typed}: {f}"));
            assert_eq!(found.name.as_deref(), Some("powerful dystopian (v1.2)"), "the stem names it without a heading");
        }
        assert_eq!(read(&dir.display().to_string()), Err(Fault::Folder));
        assert_eq!(read("~/no/such/mix.md"), Err(Fault::Missing));
        assert_eq!(read("hello"), Err(Fault::NotAFile));
        std::fs::write(&file, "no links").unwrap();
        assert_eq!(read(&plain), Err(Fault::NoLinks));
        let many: String = (0..QUEUE_MAX + 1).map(|n| format!("https://youtu.be/{n:0>11}\n")).collect();
        std::fs::write(&file, many).unwrap();
        assert_eq!(read(&plain), Err(Fault::TooMany(QUEUE_MAX + 1)), "a file YouTube would cut short");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
