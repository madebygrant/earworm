/* Asks GitHub whether a newer release exists, at most once a day, and
   remembers the answer in the cache. Nothing here ever blocks the run: the
   probe is bounded, the cache read is a stat and two small reads, and every
   failure is invisible. An update check that nags about itself is worse than
   none. */
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const RELEASES_URL: &str = "https://api.github.com/repos/madebygrant/earworm/releases/latest";
/// Once a day is plenty for a personal tool; a release sits there for hours.
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
/// The shared lookup agent already times out; this bounds the whole probe
/// including the queue a slow resolver can put it in.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// What the cache said, if it had anything to say.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub latest: String,
}

pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// How to update, which is whatever way the binary says it was built.
pub fn hint() -> &'static str {
    if option_env!("EARWORM_GIT_INSTALL").is_some() {
        "cargo install --git https://github.com/madebygrant/earworm"
    } else {
        "git pull && cargo install --path ."
    }
}

/// Older returns Less, same returns Equal. Pieces compare numerically, so
/// `0.10.0` sorts after `0.9.4` where a string compare would not; a missing
/// piece counts as zero and anything after the numbers is ignored, which is
/// what makes `v0.2.0` and `0.2.0-rc1` both parse.
pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    let digits = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['-', '+'])
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (a, b) = (digits(a), digits(b));
    for i in 0..a.len().max(b.len()) {
        match a.get(i).copied().unwrap_or(0).cmp(&b.get(i).copied().unwrap_or(0)) {
            std::cmp::Ordering::Equal => {}
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// The tag_name out of a releases/latest payload, with the `v` stripped, so
/// every render site can add exactly one. GitHub returns the tag as it was
/// typed (`v0.2.0`), and a notice that reads `vv0.2.0` is the whole feature
/// ruining itself.
pub fn tag_of(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = value.get("tag_name")?.as_str()?.trim().trim_start_matches('v').to_string();
    (!tag.is_empty()).then_some(tag)
}

fn cache_path() -> PathBuf {
    /* XDG_CACHE_HOME is already the cache root; HOME is not, and needs the
       `.cache` leg spelled out. Joining `.cache` onto both buries the file
       one level deep in every configured XDG home. */
    let base = std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map_or_else(
            || {
                let home = std::env::var("HOME").unwrap_or_default();
                PathBuf::from(home).join(".cache")
            },
            PathBuf::from,
        );
    base.join("earworm").join("latest")
}

/* The cache holds the tag on line 1 and the unix seconds it was written on
   line 2. A file this small needs no serde and no lock: whoever wrote it
   wrote it in one `fs::write`, which is atomic enough for a hint. */
fn read_cache() -> Option<Notice> {
    let text = std::fs::read_to_string(cache_path()).ok()?;
    let mut lines = text.lines();
    // Trim again on read: a cache written before the strip holds the raw tag.
    let tag = lines.next()?.trim().trim_start_matches('v').to_string();
    let stamp: u64 = lines.next()?.trim().parse().ok()?;
    let age = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .checked_sub(Duration::from_secs(stamp))?;
    (age <= MAX_AGE).then_some(Notice { latest: tag })
}

fn write_cache(tag: &str) {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = cache_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, format!("{tag}\n{stamp}\n"));
}

/// The answer for this session: the cached tag when it is fresh, the network
/// otherwise, and nothing at all when both fail. Never errors, because a
/// check that can fail is a check that can nag.
pub fn notice() -> Option<Notice> {
    if let Some(cached) = read_cache() {
        return Some(cached);
    }
    /* A fresh cache line is worth more than a fast one: the probe happens
       once, and every later start within the day reads the file. */
    let body = probe()?;
    let tag = tag_of(&body)?;
    write_cache(&tag);
    Some(Notice { latest: tag })
}

fn probe() -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let body = crate::lookup::get_text(RELEASES_URL);
        let _ = tx.send(body);
    });
    rx.recv_timeout(PROBE_TIMEOUT).ok()?
}

/// The newer version when one exists, `v`-less like everything `tag_of`
/// returns. Equal, older, switched off and unreachable all answer `None`,
/// which is why no caller has to distinguish them.
pub fn available() -> Option<String> {
    let latest = notice()?.latest;
    (compare(current(), &latest) == std::cmp::Ordering::Less).then_some(latest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /* Pinned against hand-worked answers, not against a second
       implementation: these are the cases the string compare got wrong. */
    #[test]
    fn versions_order_numerically_not_alphabetically() {
        assert_eq!(compare("0.9.4", "0.10.0"), std::cmp::Ordering::Less);
        assert_eq!(compare("0.2.0", "0.1.0"), std::cmp::Ordering::Greater);
        assert_eq!(compare("0.1.0", "0.1"), std::cmp::Ordering::Equal);
        assert_eq!(compare("0.1.0", "0.1.0"), std::cmp::Ordering::Equal);
        // Pre-release and build suffixes are noise here.
        assert_eq!(compare("0.1.0", "0.1.0-rc1"), std::cmp::Ordering::Equal);
        assert_eq!(compare("v0.2.0", "0.2.0"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn the_payload_yields_its_tag() {
        /* A release tag is `v0.2.0` on the wire and `0.2.0` out of here: the
           render sites add the `v` back, and one that arrives with it already
           would print `vv0.2.0`. */
        assert_eq!(
            tag_of(r#"{"tag_name":"v0.2.0","name":"earworm 0.2.0"}"#).unwrap(),
            "0.2.0"
        );
        assert_eq!(tag_of(r#"{"tag_name":"0.2.0"}"#).unwrap(), "0.2.0");
        assert_eq!(tag_of("not json"), None);
        assert_eq!(tag_of(r#"{"name":"v0.2.0"}"#), None);
        // A tag that is only a `v` has nothing left to say.
        assert_eq!(tag_of(r#"{"tag_name":"v"}"#), None);
    }

    #[test]
    fn the_hint_matches_its_build() {
        // Built from a checkout, which is what this repo documents.
        assert!(hint().contains("cargo install --path"));
    }

    /* `release.sh` bumps the crate version before running this suite, so a
       literal here would fail the very release it is meant to gate. The
       version is CARGO_PKG_VERSION's job to be; this only checks it is a shape
       `compare` can read. */
    #[test]
    fn current_is_a_version_compare_can_read() {
        let here = current();
        assert!(!here.is_empty());
        assert!(
            here.split('.').next().is_some_and(|p| p.parse::<u64>().is_ok()),
            "{here} is not numeric-semver shaped"
        );
        assert_eq!(compare(here, here), std::cmp::Ordering::Equal);
    }
}
