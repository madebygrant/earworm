use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

const ACOUSTID_URL: &str = "https://api.acoustid.org/v2/lookup";
const DEEZER_URL: &str = "https://api.deezer.com/search";
const ITUNES_URL: &str = "https://itunes.apple.com/search";
const COVERART_URL: &str = "https://coverartarchive.org/release-group";
const UA: &str = "earworm/0.1";
const LRCLIB_URL: &str = "https://lrclib.net/api";

// Below this the fingerprint is a guess, not an identification.
const ACOUSTID_MIN_SCORE: f64 = 0.8;

// AcoustID asks for <=3 requests/second.
const ACOUSTID_DELAY: Duration = Duration::from_millis(340);
const DEEZER_DELAY: Duration = Duration::from_millis(200);
const IMAGE_DELAY: Duration = Duration::from_millis(250);
const ITUNES_DELAY: Duration = Duration::from_millis(200);
const LRCLIB_DELAY: Duration = Duration::from_millis(250);

/* What these services ask for is a gap between requests, and a request that
   took longer than the gap has already served it. Sleeping afterwards paid it
   twice: on a slow connection most of the tagging pass was a sleep on top of
   a wait that had already happened. Held as the earliest moment the next
   request may leave, so time on the wire counts toward it. */
struct Limiter {
    next: Mutex<Option<Instant>>,
    gap: Duration,
}

impl Limiter {
    const fn new(gap: Duration) -> Self {
        Self {
            next: Mutex::new(None),
            gap,
        }
    }

    /// Called before the request, not after: a limiter that paces on the way
    /// out cannot know how long the call it is pacing took.
    fn wait(&self) {
        let now = Instant::now();
        // Claimed under the lock and slept outside it, so the slot this call
        // took is spoken for while it waits and callers cannot share one.
        let at = {
            let mut next = self.next.lock().unwrap();
            let at = next.map_or(now, |at| at.max(now));
            *next = Some(at + self.gap);
            at
        };
        if let Some(pause) = at.checked_duration_since(now) {
            sleep(pause);
        }
    }
}

static ACOUSTID_RATE: Limiter = Limiter::new(ACOUSTID_DELAY);
static DEEZER_RATE: Limiter = Limiter::new(DEEZER_DELAY);
/* Artwork comes off a CDN rather than an API, so this is politeness and not a
   published limit. A download takes longer than the gap on any connection
   that can carry one, so in practice it costs a restore nothing. */
static IMAGE_RATE: Limiter = Limiter::new(IMAGE_DELAY);
static ITUNES_RATE: Limiter = Limiter::new(ITUNES_DELAY);
static LRCLIB_RATE: Limiter = Limiter::new(LRCLIB_DELAY);

// LRCLIB itself matches within two seconds, and a longer cut is a different recording.
const LRCLIB_SLACK: f64 = 2.0;
// Shorter than the shared agent's, since a track is asked about up to twice and a library is hundreds of them.
const LRCLIB_TIMEOUT: Duration = Duration::from_secs(8);
// Consecutive failed requests before lyrics stop being asked for, and for how long.
const LRCLIB_GIVE_UP: u32 = 3;
const LRCLIB_PAUSE: Duration = Duration::from_secs(300);
static LRCLIB_BREAKER: Breaker = Breaker::new();

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
    /// The recording's identity, which two different videos of one song share.
    pub isrc: Option<String>,
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

/// Text through the shared agent, for callers that parse it themselves.
pub fn get_text(url: &str) -> Option<String> {
    let mut resp = agent().get(url).call().ok()?;
    resp.body_mut().read_to_string().ok()
}

/* Every caller of this is downloading artwork, which is why the pacing sits
   here rather than on one of them. It used to sit on none: the metadata
   queries were paced and the image that followed was not, which cost nothing
   while every caller fetched one image per keypress. `tag_tracks` fetches one
   per track and `restore_art` fetches one per track on demand, so a folder of
   forty arrived as forty back-to-back requests, and the Cover Art Archive URL
   redirects to archive.org, which is the host in this set that minds most. */
fn get_bytes(url: &str) -> Option<Vec<u8>> {
    IMAGE_RATE.wait();
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

static CHILD: AtomicU32 = AtomicU32::new(0);

/* The same promise `ytdlp::stop` makes, for the other half of the
   subprocesses. A transcode is minutes long and writes into the user's
   playlist folder, so quitting without this leaves an ffmpeg running after
   the terminal is restored, still writing a file nothing will ever clean up.
   One registration inside `run_bounded` rather than one per caller: the
   answer to "how does earworm stop a child it is waiting on" has to be in
   the same place as the waiting. */
pub fn stop() {
    STOPPING.store(true, Ordering::SeqCst);
    let pid = CHILD.swap(0, Ordering::SeqCst);
    if pid != 0 {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}

/* Latched, and the reason is `transcode`: killing its ffmpeg makes the call
   come back a failure, and a failure is what makes it try the next encoder in
   the list. Without this the shutdown killed one child and the very next line
   spawned another, orphaned this time because nothing is left to kill it. It
   also closes the gap between the cancel check at the top of a loop and the
   spawn a few lines later.

   Process-wide and never cleared, which is the trap the colour depth
   documents: nothing in the suite may call `stop`, or every bounded
   subprocess after it in that binary returns `None`. */
static STOPPING: AtomicBool = AtomicBool::new(false);

/// std has no timeout on `output()`, and a wedged fpcalc would stall the run on
/// one unreadable file.
/// The caller sets the stdio, since which streams are worth capturing is the
/// caller's business and a piped stream nobody drains can fill and block.
pub fn run_bounded(cmd: &mut Command, limit: Duration) -> Option<std::process::Output> {
    // Nothing new starts once the shutdown has begun. See `STOPPING`.
    if STOPPING.load(Ordering::SeqCst) {
        return None;
    }
    let mut child = cmd.spawn().ok()?;
    CHILD.store(child.id(), Ordering::SeqCst);
    let deadline = Instant::now() + limit;
    loop {
        /* Killed from the shutdown path rather than by this loop, so the exit
           does not wait out a transcode. `try_wait` then reports the child
           gone and the caller sees the same failure a timeout gives it. */
        match child.try_wait() {
            Ok(Some(_)) => {
                CHILD.store(0, Ordering::SeqCst);
                return child.wait_with_output().ok();
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                CHILD.store(0, Ordering::SeqCst);
                return None;
            }
            Ok(None) => sleep(Duration::from_millis(50)),
            Err(_) => {
                CHILD.store(0, Ordering::SeqCst);
                return None;
            }
        }
    }
}

fn fingerprint(path: &Path) -> Option<(i64, String)> {
    let out = run_bounded(
        Command::new("fpcalc")
            .arg("-json")
            .arg(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
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
    ACOUSTID_RATE.wait();
    let data = get_json(&url)?;
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
                isrc: None,
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
    DEEZER_RATE.wait();
    let data = get_json(&format!("{DEEZER_URL}?q={}&limit=1", encode(query)));
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
        isrc: hit
            .get("isrc")
            .and_then(Value::as_str)
            .filter(|isrc| !isrc.is_empty())
            .map(str::to_string),
        source: "deezer",
    })
}

/// iTunes Search API hit turned into a `Match`. Kept apart from the fetch so
/// the mapping is testable as text in, `Match` out.
fn itunes_match(hit: &Value) -> Option<Match> {
    Some(Match {
        title: hit.get("trackName")?.as_str()?.to_string(),
        artist: hit
            .get("artistName")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        album: hit
            .get("collectionName")
            .and_then(Value::as_str)
            .map(str::to_string),
        cover_url: hit
            .get("artworkUrl100")
            .and_then(Value::as_str)
            .map(upscale_artwork),
        mbid: None,
        score: None,
        isrc: None,
        source: "apple",
    })
}

/// iTunes serves 100x100 thumbnails; the same path at 600x600 exists and is
/// what gets embedded. A URL without the size marker is used as-is.
fn upscale_artwork(url: &str) -> String {
    url.replace("100x100", "600x600")
}

/// Apple Music catalogue via the free iTunes Search API, no key needed.
/// Fallback for when Deezer has no plausible hit.
pub fn itunes(artist: &str, title: &str) -> Option<Match> {
    let query = format!("{artist} {title}");
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    ITUNES_RATE.wait();
    let data = get_json(&format!(
        "{ITUNES_URL}?term={}&media=music&entity=song&limit=1",
        encode(query)
    ));
    let hit = data?.get("results")?.as_array()?.first()?.clone();
    itunes_match(&hit)
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

/// Deezer albums matching the track, plus Apple ones when the fallback is on,
/// plus the Cover Art Archive release group when the identification supplied
/// one.
pub fn artwork(artist: &str, title: &str, mbid: Option<&str>, apple: bool) -> Vec<Artwork> {
    let mut out = Vec::new();
    if let Some(mbid) = mbid {
        out.push(Artwork {
            label: "Cover Art Archive  release group  500x500".into(),
            url: format!("{COVERART_URL}/{mbid}/front-500"),
        });
    }

    let query = format!("{artist} {title}");
    let query = query.trim();
    if !query.is_empty() {
        DEEZER_RATE.wait();
        if let Some(data) = get_json(&format!("{DEEZER_URL}?q={}&limit=8", encode(query))) {
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
        if apple {
            ITUNES_RATE.wait();
        }
        if apple
            && let Some(data) = get_json(&format!(
                "{ITUNES_URL}?term={}&media=music&entity=song&limit=8",
                encode(query)
            ))
        {
            for hit in data
                .get("results")
                .and_then(Value::as_array)
                .unwrap_or(&vec![])
            {
                let (Some(name), Some(url)) = (
                    hit.get("collectionName").and_then(Value::as_str),
                    hit.get("artworkUrl100").and_then(Value::as_str),
                ) else {
                    continue;
                };
                push_unique(
                    &mut out,
                    format!("Apple  {name}  600x600"),
                    upscale_artwork(url),
                );
            }
        }
    }
    out
}

pub fn cover_bytes(m: &Match, apple: bool) -> Option<Vec<u8>> {
    if let Some(mbid) = &m.mbid
        && let Some(data) = get_bytes(&format!("{COVERART_URL}/{mbid}/front-500"))
    {
        return Some(data);
    }
    if let Some(url) = &m.cover_url {
        return get_bytes(url);
    }
    // AcoustID carries no artwork, so fall back to a text search for the
    // image alone: Deezer first, then Apple when the fallback is on.
    if let Some(alt) = deezer(&m.artist, &m.title)
        && let Some(url) = alt.cover_url
    {
        return get_bytes(&url);
    }
    if apple {
        let alt = itunes(&m.artist, &m.title)?;
        return get_bytes(&alt.cover_url?);
    }
    None
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

/// Deezer and Apple both return a top hit for any query, so the gate has to be
/// real. A loose title match is only safe when the artist we searched with
/// also corresponds; searching with a channel name instead demands an exact
/// title.
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

/// What asking for a track's lyrics came to. `Missing` and `Unavailable` look
/// alike to the tags, which stay empty, but not to the summary: one is a track
/// LRCLIB lacks and the other is a service that did not answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    Found(String),
    Missing,
    /// Not asked: no length or no name to ask with.
    Skipped,
    Unavailable,
}

// How one request ended. A 404 is an answer, where a timeout or a refused connection is not.
enum Reply {
    Found(Value),
    Missing,
    Down,
}

fn lrclib_json(url: &str) -> Reply {
    let call = agent().get(url).config().timeout_global(Some(LRCLIB_TIMEOUT)).build().call();
    match call {
        Ok(mut resp) => resp.body_mut().read_json::<Value>().map_or(Reply::Down, Reply::Found),
        Err(ureq::Error::StatusCode(404)) => Reply::Missing,
        Err(_) => Reply::Down,
    }
}

/* LRCLIB has gone down for hours before, and every track asks up to twice with
   a timeout each, so a library would sit on it for the length of the sync.
   After a few failures in a row it stops asking for a while, and a success at
   any point starts the count again. */
struct Breaker {
    // Consecutive failures, and when the pause ends once there have been enough.
    state: Mutex<(u32, Option<Instant>)>,
}

impl Breaker {
    const fn new() -> Self {
        Self { state: Mutex::new((0, None)) }
    }

    fn paused(&self, now: Instant) -> bool {
        let mut state = self.state.lock().unwrap();
        match state.1 {
            Some(until) if now < until => true,
            Some(_) => {
                *state = (0, None);
                false
            }
            None => false,
        }
    }

    fn record(&self, answered: bool, now: Instant) {
        let mut state = self.state.lock().unwrap();
        state.0 = if answered { 0 } else { state.0 + 1 };
        if state.0 >= LRCLIB_GIVE_UP {
            state.1 = Some(now + LRCLIB_PAUSE);
        }
    }
}

/// Plain lyrics for a track. LRCLIB's `get` wants an album and a duration, and
/// most playlist tracks have no album, so a miss or a missing album falls
/// through to `search`, which is held to the same duration and to the same
/// artist and title.
pub fn lyrics(artist: &str, title: &str, album: &str, duration: u64) -> Lookup {
    lyrics_via(&LRCLIB_BREAKER, artist, title, album, duration, |url| {
        LRCLIB_RATE.wait();
        lrclib_json(url)
    })
}

// Takes the request so a test can stand in for the network and count what was asked.
fn lyrics_via(
    breaker: &Breaker,
    artist: &str,
    title: &str,
    album: &str,
    duration: u64,
    fetch: impl Fn(&str) -> Reply,
) -> Lookup {
    // Without a length there is no telling the studio cut from a live one.
    if duration == 0 || artist.is_empty() || title.is_empty() {
        return Lookup::Skipped;
    }
    if breaker.paused(Instant::now()) {
        return Lookup::Unavailable;
    }
    if !album.is_empty() {
        let url = format!(
            "{LRCLIB_URL}/get?track_name={}&artist_name={}&album_name={}&duration={duration}",
            encode(title),
            encode(artist),
            encode(album)
        );
        match fetch(&url) {
            Reply::Down => {
                breaker.record(false, Instant::now());
                return Lookup::Unavailable;
            }
            Reply::Found(hit) => {
                breaker.record(true, Instant::now());
                // An exact match that says instrumental is the answer: another record's words are not.
                if instrumental(&hit) {
                    return Lookup::Missing;
                }
                if let Some(text) = lyrics_of(&hit) {
                    return Lookup::Found(text);
                }
            }
            Reply::Missing => breaker.record(true, Instant::now()),
        }
    }
    let url = format!(
        "{LRCLIB_URL}/search?track_name={}&artist_name={}",
        encode(title),
        encode(artist)
    );
    match fetch(&url) {
        Reply::Down => {
            breaker.record(false, Instant::now());
            Lookup::Unavailable
        }
        Reply::Missing => {
            breaker.record(true, Instant::now());
            Lookup::Missing
        }
        Reply::Found(results) => {
            breaker.record(true, Instant::now());
            results
                .as_array()
                .and_then(|results| pick(results, artist, title, duration))
                .and_then(lyrics_of)
                .map_or(Lookup::Missing, Lookup::Found)
        }
    }
}

/* The first result with words, within the slack of the track's length and by
   the same artist and title. Length alone lets a cover or tribute of the same
   song through, and lyrics are never overwritten once written. `similar` is
   the containment test the lookups use, so a collaboration still matches. */
fn pick<'a>(results: &'a [Value], artist: &str, title: &str, duration: u64) -> Option<&'a Value> {
    let said = |hit: &Value, key: &str| hit.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    results.iter().find(|hit| {
        hit.get("duration")
            .and_then(Value::as_f64)
            .is_some_and(|d| (d - duration as f64).abs() <= LRCLIB_SLACK)
            && similar(&said(hit, "artistName"), artist)
            && similar(&said(hit, "trackName"), title)
            && lyrics_of(hit).is_some()
    })
}

fn instrumental(hit: &Value) -> bool {
    hit.get("instrumental").and_then(Value::as_bool) == Some(true)
}

fn lyrics_of(hit: &Value) -> Option<String> {
    if instrumental(hit) {
        return None;
    }
    let text = |key: &str| hit.get(key).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
    // Synced text in a plain tag would show its timestamps in a player that does not read them.
    text("plainLyrics")
        .map(str::to_string)
        .or_else(|| text("syncedLyrics").map(strip_timestamps))
        .filter(|t| !t.is_empty())
}

// `[01:23.45] words` becomes `words`; a line with no timestamp is kept as it is.
fn strip_timestamps(synced: &str) -> String {
    let line = |line: &str| {
        let mut rest = line.trim_start();
        while let Some(inner) = rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
            let (stamp, after) = inner;
            if !stamp.chars().all(|c| c.is_ascii_digit() || matches!(c, ':' | '.')) {
                break;
            }
            rest = after.trim_start();
        }
        rest.to_string()
    };
    synced.lines().map(line).collect::<Vec<_>>().join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /* The gap these services ask for is between requests, so a request that
       took longer than the gap has already served it. Sleeping afterwards
       paid it twice, which on a slow connection was most of a tagging pass. */
    #[test]
    fn a_request_that_outlasts_the_gap_does_not_wait_again() {
        let limiter = Limiter::new(Duration::from_millis(60));

        // Nothing has gone out yet, so the first call owes nothing.
        let at = Instant::now();
        limiter.wait();
        assert!(at.elapsed() < Duration::from_millis(40), "{:?}", at.elapsed());

        // Straight afterwards the whole gap is still owed.
        let at = Instant::now();
        limiter.wait();
        assert!(at.elapsed() >= Duration::from_millis(55), "{:?}", at.elapsed());

        // And a call that took longer than the gap has already paid it.
        sleep(Duration::from_millis(80));
        let at = Instant::now();
        limiter.wait();
        assert!(at.elapsed() < Duration::from_millis(40), "{:?}", at.elapsed());
    }

    /* The rate is the service's, not one thread's. Holding it across callers
       is what would make a pool over these lookups safe: the slot is claimed
       under the lock, so two threads cannot be handed the same one. */
    #[test]
    fn the_rate_is_shared_by_everything_that_asks_for_it() {
        static SHARED: Limiter = Limiter::new(Duration::from_millis(60));
        let at = Instant::now();
        std::thread::scope(|scope| {
            for _ in 0..3 {
                scope.spawn(|| SHARED.wait());
            }
        });
        // Three requests at one per 60ms is 120ms of gap, whoever asks.
        assert!(at.elapsed() >= Duration::from_millis(110), "{:?}", at.elapsed());
    }

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

    #[test]
    fn itunes_hit_maps_to_a_match_with_upscaled_artwork() {
        let hit: Value = serde_json::from_str(
            r#"{
                "trackName": "Roygbiv",
                "artistName": "Boards of Canada",
                "collectionName": "Music Has the Right to Children",
                "artworkUrl100": "https://example.com/cover/100x100bb.jpg"
            }"#,
        )
        .unwrap();
        let m = itunes_match(&hit).unwrap();
        assert_eq!(m.title, "Roygbiv");
        assert_eq!(m.artist, "Boards of Canada");
        assert_eq!(m.album.as_deref(), Some("Music Has the Right to Children"));
        assert_eq!(
            m.cover_url.as_deref(),
            Some("https://example.com/cover/600x600bb.jpg")
        );
        assert_eq!(m.source, "apple");
    }

    #[test]
    fn itunes_hit_without_a_track_name_is_no_match() {
        let hit: Value = serde_json::from_str(r#"{"artistName": "Nobody"}"#).unwrap();
        assert!(itunes_match(&hit).is_none());
    }

    #[test]
    fn artwork_url_without_a_size_marker_passes_through() {
        assert_eq!(
            upscale_artwork("https://example.com/cover.jpg"),
            "https://example.com/cover.jpg"
        );
    }

    #[test]
    fn apple_hits_pass_through_the_same_plausibility_gate() {
        let good = Match {
            title: "Roygbiv".into(),
            artist: "Boards of Canada".into(),
            album: None,
            mbid: None,
            cover_url: None,
            score: None,
            isrc: None,
            source: "apple",
        };
        assert!(plausible(&good, "Boards of Canada", "Roygbiv"));
        let bad = Match {
            title: "Something Entirely Different".into(),
            artist: "Boards of Canada".into(),
            album: None,
            mbid: None,
            cover_url: None,
            score: None,
            isrc: None,
            source: "apple",
        };
        assert!(!plausible(&bad, "Boards of Canada", "Roygbiv"));
    }

    #[test]
    fn synced_lyrics_lose_their_timestamps_and_nothing_else() {
        let synced = "[00:12.50] Fahren fahren fahren\n[00:15.00] auf der Autobahn\n[00:16.00]\n[01:02.3][01:30.1] twice\n[ar: Kraftwerk]";
        assert_eq!(
            strip_timestamps(synced),
            "Fahren fahren fahren\nauf der Autobahn\n\ntwice\n[ar: Kraftwerk]"
        );
    }

    #[test]
    fn lyrics_prefer_plain_text_and_skip_an_instrumental() {
        let both = serde_json::json!({"plainLyrics": "plain", "syncedLyrics": "[00:01.00] synced"});
        assert_eq!(lyrics_of(&both).as_deref(), Some("plain"));
        let synced = serde_json::json!({"plainLyrics": null, "syncedLyrics": "[00:01.00] synced"});
        assert_eq!(lyrics_of(&synced).as_deref(), Some("synced"));
        let instrumental = serde_json::json!({"instrumental": true, "plainLyrics": "words"});
        assert_eq!(lyrics_of(&instrumental), None);
        assert_eq!(lyrics_of(&serde_json::json!({"plainLyrics": "  "})), None);
    }

    // The length is what keeps a live cut or a radio edit's words off the studio track.
    #[test]
    fn a_search_result_has_to_match_the_length_and_have_words() {
        let hit = |duration: f64, words: Option<&str>| {
            serde_json::json!({"artistName": "Kraftwerk", "trackName": "Autobahn", "duration": duration, "plainLyrics": words})
        };
        let results = [hit(330.0, Some("live")), hit(214.0, None), hit(212.4, Some("studio"))];
        let found = pick(&results, "Kraftwerk", "Autobahn", 213).unwrap();
        assert_eq!(lyrics_of(found).as_deref(), Some("studio"));
        assert!(pick(&results, "Kraftwerk", "Autobahn", 100).is_none());
        assert!(pick(&[], "Kraftwerk", "Autobahn", 213).is_none());
    }

    // A cover of the same song at the same length would otherwise get its words written for good.
    #[test]
    fn a_search_result_has_to_be_by_the_same_artist_and_title() {
        let by = |artist: &str, title: &str| {
            serde_json::json!({"artistName": artist, "trackName": title, "duration": 213.0, "plainLyrics": "words"})
        };
        let results = [by("The Tribute Band", "Autobahn"), by("Kraftwerk", "Radioactivity")];
        assert!(pick(&results, "Kraftwerk", "Autobahn", 213).is_none());
        // A collaboration credit and a longer title still match.
        let credited = [by("Kraftwerk & Friends", "Autobahn (Remastered)")];
        assert!(pick(&credited, "Kraftwerk", "Autobahn", 213).is_some());
    }

    #[test]
    fn no_lookup_is_made_without_a_length_or_a_name() {
        let breaker = Breaker::new();
        let never = |_: &str| -> Reply { panic!("asked the network") };
        assert_eq!(lyrics_via(&breaker, "Kraftwerk", "Autobahn", "Autobahn", 0, never), Lookup::Skipped);
        assert_eq!(lyrics_via(&breaker, "", "Autobahn", "", 213, never), Lookup::Skipped);
        assert_eq!(lyrics_via(&breaker, "Kraftwerk", "", "", 213, never), Lookup::Skipped);
    }

    // An exact match that says instrumental is final: searching on could attach another record's words.
    #[test]
    fn an_instrumental_from_the_exact_lookup_is_not_searched_past() {
        let calls = std::cell::Cell::new(0);
        let fetch = |url: &str| {
            calls.set(calls.get() + 1);
            assert!(url.contains("/get?"), "went on to {url}");
            Reply::Found(serde_json::json!({"instrumental": true, "plainLyrics": null}))
        };
        let got = lyrics_via(&Breaker::new(), "Kraftwerk", "Autobahn", "Autobahn", 213, fetch);
        assert_eq!((got, calls.get()), (Lookup::Missing, 1));
    }

    #[test]
    fn a_miss_on_the_exact_lookup_falls_through_to_search() {
        let urls = std::cell::RefCell::new(Vec::new());
        let fetch = |url: &str| {
            urls.borrow_mut().push(url.split('?').next().unwrap().to_string());
            if url.contains("/get?") {
                Reply::Missing
            } else {
                Reply::Found(serde_json::json!([
                    {"artistName": "Kraftwerk", "trackName": "Autobahn", "duration": 213.0, "plainLyrics": "words"}
                ]))
            }
        };
        let got = lyrics_via(&Breaker::new(), "Kraftwerk", "Autobahn", "Autobahn", 213, fetch);
        assert_eq!(got, Lookup::Found("words".into()));
        assert_eq!(urls.borrow().len(), 2);
        // No album, no exact lookup.
        urls.borrow_mut().clear();
        lyrics_via(&Breaker::new(), "Kraftwerk", "Autobahn", "", 213, fetch);
        assert_eq!(urls.borrow().len(), 1);
        assert!(urls.borrow()[0].ends_with("/search"));
    }

    // Three failures in a row stop the asking, so a down service costs a few timeouts and not a whole sync.
    #[test]
    fn a_service_that_keeps_failing_is_left_alone_for_a_while() {
        let breaker = Breaker::new();
        let calls = std::cell::Cell::new(0);
        let down = |_: &str| {
            calls.set(calls.get() + 1);
            Reply::Down
        };
        let ask = || lyrics_via(&breaker, "Kraftwerk", "Autobahn", "", 213, down);
        for _ in 0..LRCLIB_GIVE_UP {
            assert_eq!(ask(), Lookup::Unavailable);
        }
        assert_eq!(calls.get(), LRCLIB_GIVE_UP as usize);
        // Paused: answered with no request at all.
        assert_eq!(ask(), Lookup::Unavailable);
        assert_eq!(calls.get(), LRCLIB_GIVE_UP as usize, "asked while paused");
        // And asked again once the pause is over.
        let later = Instant::now() + LRCLIB_PAUSE + Duration::from_secs(1);
        assert!(!breaker.paused(later));
    }

    // A 404 is an answer, so a library full of tracks LRCLIB lacks must not trip it.
    #[test]
    fn misses_and_successes_do_not_count_as_failures() {
        let breaker = Breaker::new();
        let now = Instant::now();
        breaker.record(false, now);
        breaker.record(false, now);
        breaker.record(true, now);
        breaker.record(false, now);
        breaker.record(false, now);
        assert!(!breaker.paused(now), "a success did not reset the count");
        let missing = |_: &str| Reply::Missing;
        for _ in 0..(LRCLIB_GIVE_UP * 3) {
            assert_eq!(lyrics_via(&breaker, "A", "B", "", 213, missing), Lookup::Missing);
        }
        assert!(!breaker.paused(Instant::now()));
    }
}
