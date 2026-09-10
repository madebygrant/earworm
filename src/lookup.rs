use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

const ACOUSTID_URL: &str = "https://api.acoustid.org/v2/lookup";
const DEEZER_URL: &str = "https://api.deezer.com/search";
const COVERART_URL: &str = "https://coverartarchive.org/release-group";
const UA: &str = "earworm/0.1";

// Below this the fingerprint is a guess, not an identification.
const ACOUSTID_MIN_SCORE: f64 = 0.8;

// AcoustID asks for <=3 requests/second.
const ACOUSTID_DELAY: Duration = Duration::from_millis(340);
const DEEZER_DELAY: Duration = Duration::from_millis(200);

// ureq has no timeout by default, and a stalled lookup would hang the worker
// with no way to skip the track.
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const FPCALC_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug)]
pub struct Match {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub mbid: Option<String>,
    pub cover_url: Option<String>,
    pub score: Option<f64>,
    pub source: &'static str,
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT))
            .user_agent(UA)
            .build()
            .into()
    })
}

fn get_json(url: &str) -> Option<Value> {
    let mut resp = agent().get(url).call().ok()?;
    resp.body_mut().read_json::<Value>().ok()
}

fn get_bytes(url: &str) -> Option<Vec<u8>> {
    let mut resp = agent().get(url).call().ok()?;
    resp.body_mut()
        .with_config()
        .limit(20 * 1024 * 1024)
        .read_to_vec()
        .ok()
}

fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// std has no timeout on `output()`, and a wedged fpcalc would stall the run on
/// one unreadable file.
fn run_bounded(cmd: &mut Command, limit: Duration) -> Option<std::process::Output> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => sleep(Duration::from_millis(50)),
            Err(_) => return None,
        }
    }
}

fn fingerprint(path: &Path) -> Option<(i64, String)> {
    let out = run_bounded(
        Command::new("fpcalc").arg("-json").arg(path),
        FPCALC_TIMEOUT,
    )?;
    if !out.status.success() {
        return None;
    }
    let data: Value = serde_json::from_slice(&out.stdout).ok()?;
    Some((
        data.get("duration")?.as_f64()? as i64,
        data.get("fingerprint")?.as_str()?.to_string(),
    ))
}

pub fn acoustid(path: &Path, key: &str) -> Option<Match> {
    let (duration, code) = fingerprint(path)?;
    let url = format!(
        "{ACOUSTID_URL}?client={}&duration={duration}&fingerprint={}&meta=recordings+releasegroups",
        encode(key),
        encode(&code)
    );
    let data = get_json(&url);
    sleep(ACOUSTID_DELAY);
    let data = data?;
    if data.get("status").and_then(Value::as_str) != Some("ok") {
        return None;
    }

    /* Ranking is not guaranteed, and a low-confidence fingerprint is worse than
    no answer because it bypasses every check a Deezer hit must pass. */
    let mut results: Vec<&Value> = data.get("results")?.as_array()?.iter().collect();
    results.sort_by(|a, b| score_of(b).total_cmp(&score_of(a)));

    for result in results {
        let score = score_of(result);
        if score < ACOUSTID_MIN_SCORE {
            break;
        }
        for rec in result
            .get("recordings")
            .and_then(Value::as_array)
            .unwrap_or(&vec![])
        {
            let title = rec.get("title").and_then(Value::as_str);
            let artist = rec
                .get("artists")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|a| a.get("name"))
                .and_then(Value::as_str);
            let (Some(title), Some(artist)) = (title, artist) else {
                continue;
            };
            let group = rec
                .get("releasegroups")
                .and_then(Value::as_array)
                .and_then(|g| g.first());
            return Some(Match {
                title: title.to_string(),
                artist: artist.to_string(),
                album: group
                    .and_then(|g| g.get("title"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                mbid: group
                    .and_then(|g| g.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                cover_url: None,
                score: Some(score),
                source: "acoustid",
            });
        }
    }
    None
}

fn score_of(result: &Value) -> f64 {
    result.get("score").and_then(Value::as_f64).unwrap_or(0.0)
}

pub fn deezer(artist: &str, title: &str) -> Option<Match> {
    let query = format!("{artist} {title}");
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    let data = get_json(&format!("{DEEZER_URL}?q={}&limit=1", encode(query)));
    sleep(DEEZER_DELAY);
    let hit = data?.get("data")?.as_array()?.first()?.clone();
    Some(Match {
        title: hit.get("title")?.as_str()?.to_string(),
        artist: hit
            .get("artist")
            .and_then(|a| a.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        album: hit
            .get("album")
            .and_then(|a| a.get("title"))
            .and_then(Value::as_str)
            .map(str::to_string),
        cover_url: hit
            .get("album")
            .and_then(|a| a.get("cover_xl"))
            .and_then(Value::as_str)
            .map(str::to_string),
        mbid: None,
        score: None,
        source: "deezer",
    })
}

/// A cover the user could pick. Nothing is fetched to build the list; only the
/// chosen one is downloaded.
pub struct Artwork {
    pub label: String,
    pub url: String,
}

pub fn fetch(url: &str) -> Option<Vec<u8>> {
    get_bytes(url)
}

/// Width and height from a JPEG's frame header, for labelling art already on
/// disk. Returns (0, 0) for anything it cannot parse.
pub fn jpeg_size(data: &[u8]) -> (u16, u16) {
    let mut i = 2;
    while i + 9 < data.len() {
        if data[i] != 0xFF {
            break;
        }
        let marker = data[i + 1];
        let seglen = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            return (
                u16::from_be_bytes([data[i + 7], data[i + 8]]),
                u16::from_be_bytes([data[i + 5], data[i + 6]]),
            );
        }
        i += 2 + seglen;
    }
    (0, 0)
}

/// Different releases of one album share a title but not a URL, and two
/// identical-looking rows are worse than one fewer choice.
fn push_unique(out: &mut Vec<Artwork>, label: String, url: String) {
    if out.iter().any(|a| a.url == url || a.label == label) {
        return;
    }
    out.push(Artwork { label, url });
}

/// Deezer albums matching the track, plus the Cover Art Archive release group
/// when the identification supplied one.
pub fn artwork(artist: &str, title: &str, mbid: Option<&str>) -> Vec<Artwork> {
    let mut out = Vec::new();
    if let Some(mbid) = mbid {
        out.push(Artwork {
            label: "Cover Art Archive  release group  500x500".into(),
            url: format!("{COVERART_URL}/{mbid}/front-500"),
        });
    }

    let query = format!("{artist} {title}");
    let query = query.trim();
    if !query.is_empty()
        && let Some(data) = get_json(&format!("{DEEZER_URL}?q={}&limit=8", encode(query)))
    {
        sleep(DEEZER_DELAY);
        for hit in data.get("data").and_then(Value::as_array).unwrap_or(&vec![]) {
            let album = hit.get("album");
            let (Some(name), Some(url)) = (
                album.and_then(|a| a.get("title")).and_then(Value::as_str),
                album.and_then(|a| a.get("cover_xl")).and_then(Value::as_str),
            ) else {
                continue;
            };
            push_unique(
                &mut out,
                format!("Deezer  {name}  1000x1000"),
                url.to_string(),
            );
        }
    }
    out
}

pub fn cover_bytes(m: &Match) -> Option<Vec<u8>> {
    if let Some(mbid) = &m.mbid
        && let Some(data) = get_bytes(&format!("{COVERART_URL}/{mbid}/front-500"))
    {
        return Some(data);
    }
    if let Some(url) = &m.cover_url {
        return get_bytes(url);
    }
    // AcoustID carries no artwork, so fall back to Deezer for the image alone.
    let alt = deezer(&m.artist, &m.title)?;
    get_bytes(&alt.cover_url?)
}

pub fn normalise(text: &str) -> String {
    text.to_lowercase()
        .nfkd()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

pub fn similar(a: &str, b: &str) -> bool {
    let (a, b) = (normalise(a), normalise(b));
    !a.is_empty() && !b.is_empty() && (a == b || a.contains(&b) || b.contains(&a))
}

/// Deezer returns a top hit for any query, so the gate has to be real. A loose
/// title match is only safe when the artist we searched with also corresponds;
/// searching with a channel name instead demands an exact title.
pub fn plausible(m: &Match, artist: &str, title: &str) -> bool {
    if m.source == "acoustid" {
        return true; // gated on score at lookup time instead
    }
    let (got, want) = (normalise(&m.title), normalise(title));
    if got.is_empty() || want.is_empty() {
        return false;
    }
    if similar(&m.artist, artist) {
        got == want || want.contains(&got) || got.contains(&want)
    } else {
        got == want
    }
}

const SYMBOL_SEPARATORS: [char; 3] = [',', '&', '/'];
const WORD_SEPARATORS: [&str; 3] = ["feat", "ft", "with"];

/// Word separators need boundaries on both sides, or "Wilco" and "Software"
/// would be split down the middle.
fn word_separator(chars: &[char], i: usize) -> Option<usize> {
    if i > 0 && chars[i - 1].is_alphanumeric() {
        return None;
    }
    for word in WORD_SEPARATORS {
        let n = word.len();
        if i + n > chars.len() {
            continue;
        }
        if !chars[i..i + n]
            .iter()
            .zip(word.chars())
            .all(|(c, w)| c.to_ascii_lowercase() == w)
        {
            continue;
        }
        let len = n + usize::from(chars.get(i + n) == Some(&'.'));
        if chars.get(i + len).is_none_or(|c| !c.is_alphanumeric()) {
            return Some(len);
        }
    }
    None
}

/// Indexes chars, not bytes: `to_lowercase` does not preserve byte length, so
/// scanning a lowercased copy with offsets from the original panics on input
/// as ordinary as a Kelvin sign or a Turkish dotted capital.
fn split_artists(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < chars.len() {
        if SYMBOL_SEPARATORS.contains(&chars[i]) {
            parts.push(std::mem::take(&mut current));
            i += 1;
        } else if let Some(len) = word_separator(&chars, i) {
            parts.push(std::mem::take(&mut current));
            i += len;
        } else {
            current.push(chars[i]);
            i += 1;
        }
    }
    parts.push(current);

    let mut seen = Vec::new();
    parts
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| {
            let key = normalise(p);
            if key.is_empty() || seen.contains(&key) {
                false
            } else {
                seen.push(key);
                true
            }
        })
        .collect()
}

/// True when a match drops a distinct collaborator the tags already had. One
/// name repeated is a dedupe, not a loss, so casing and diacritic fixes on a
/// single-artist credit still go through.
pub fn downgrades(new: &str, old: &str) -> bool {
    let parts = split_artists(old);
    if parts.len() <= 1 {
        return false;
    }
    let got = normalise(new);
    parts.iter().any(|p| !got.contains(&normalise(p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_without_panicking_on_length_changing_lowercase() {
        // U+212A lowercases shorter, U+0130 lowercases longer; indexing a
        // lowercased copy by the original's offsets used to panic on both.
        assert_eq!(split_artists("\u{212A}urt & Ann"), ["\u{212A}urt", "Ann"]);
        assert_eq!(split_artists("\u{130}stanbul & X"), ["\u{130}stanbul", "X"]);
    }

    #[test]
    fn splits_on_separators_with_or_without_spaces() {
        assert_eq!(split_artists("A,B"), ["A", "B"]);
        assert_eq!(
            split_artists("Gavin Greenaway,The Lyndhurst Orchestra"),
            ["Gavin Greenaway", "The Lyndhurst Orchestra"]
        );
        assert_eq!(
            split_artists("Hans Zimmer & Lisa Gerrard"),
            ["Hans Zimmer", "Lisa Gerrard"]
        );
        assert_eq!(split_artists("A feat. B"), ["A", "B"]);
        assert_eq!(split_artists("A FT C"), ["A", "C"]);
    }

    #[test]
    fn keeps_separator_words_inside_names() {
        assert_eq!(split_artists("Wilco"), ["Wilco"]);
        assert_eq!(split_artists("Within Temptation"), ["Within Temptation"]);
        assert_eq!(split_artists("Daft Punk"), ["Daft Punk"]);
    }

    #[test]
    fn artwork_rows_are_unique_by_label_and_by_url() {
        let mut out = Vec::new();
        push_unique(&mut out, "Gladiator".into(), "http://a".into());
        push_unique(&mut out, "Gladiator".into(), "http://b".into()); // reissue
        push_unique(&mut out, "Other".into(), "http://a".into()); // same image
        push_unique(&mut out, "Other".into(), "http://c".into());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].label, "Gladiator");
        assert_eq!(out[1].label, "Other");
    }

    #[test]
    fn downgrade_guard_catches_dropped_collaborators() {
        assert!(downgrades("Lisa Gerrard", "Hans Zimmer & Lisa Gerrard"));
        assert!(!downgrades(
            "Hans Zimmer & Lisa Gerrard",
            "Hans Zimmer & Lisa Gerrard"
        ));
        assert!(!downgrades("Bjork", "Björk"));
        assert!(!downgrades("Sigur Ros", "Sigur Rós"));
    }
}
