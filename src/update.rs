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
    hint_for(option_env!("EARWORM_GIT_INSTALL").is_some())
}

/* Kept apart from the build flag, like `depth_from` and `opener` are from
   theirs, so both answers are testable: resolved through `option_env!` only
   one of the two exists in any given binary, and a test of `hint()` asserts
   whichever way the machine running it happened to be built. */
fn hint_for(from_git: bool) -> &'static str {
    if from_git {
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
    read_cache_at(&cache_path())
}

/* Takes the path, so the decision is testable as a decision rather than as
   whatever `XDG_CACHE_HOME` happens to be on the machine running the suite:
   setting it for a test is a process-wide change in a threaded runner, which
   is the same trap the colour depth documents. */
fn read_cache_at(path: &std::path::Path) -> Option<Notice> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    // Trim again on read: a cache written before the strip holds the raw tag.
    let tag = lines.next()?.trim().trim_start_matches('v').to_string();
    let stamp: u64 = lines.next()?.trim().parse().ok()?;
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).ok()?;
    /* A stamp ahead of the clock is a clock that has moved, not a cache from
       the future, so it counts as just written. Subtracting and giving up
       made the file unreadable instead, and the cache is the only throttle
       there is: every start would go back to GitHub for as long as the clock
       stayed behind. */
    let age = now.saturating_sub(Duration::from_secs(stamp));
    (age <= MAX_AGE).then_some(Notice { latest: tag })
}

fn write_cache(tag: &str) {
    write_cache_at(&cache_path(), tag);
}

fn write_cache_at(path: &std::path::Path, tag: &str) {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    /* The same replace the config uses. A torn cache would only cost one
       extra probe, but two answers to "how does earworm replace a file" is
       how the weaker one ends up somewhere that cannot afford it. */
    let _ = crate::config::write_atomically(path, &format!("{tag}\n{stamp}\n"));
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

    /* Both branches, since only one of them exists in any given binary: a
       test of `hint()` alone passes by agreeing with however the machine
       running it was built, and says nothing about the other. */
    #[test]
    fn each_build_is_told_how_to_update_itself() {
        assert!(hint_for(false).contains("cargo install --path"));
        assert!(hint_for(true).contains("--git"));
        // Whichever this binary is, it has something to say.
        assert!(hint().contains("cargo install"));
    }

    /* The cache is the only throttle there is. A clock that moved backwards
       made the file unreadable rather than stale, so every start went back to
       GitHub for as long as the clock stayed behind. */
    #[test]
    fn a_stamp_from_the_future_is_a_moved_clock_and_not_a_stale_cache() {
        let dir = std::env::temp_dir().join(format!("earworm-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("latest");
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let stamped = |at: u64| std::fs::write(&path, format!("9.9.9\n{at}\n")).unwrap();

        // Written through the same replace the config uses, directories and
        // all, which is the other half of what changed here.
        write_cache_at(&path, "9.9.9");
        assert_eq!(read_cache_at(&path).map(|n| n.latest), Some("9.9.9".into()));

        // An hour ahead of this machine, which is a clock and not a cache.
        stamped(now + 3600);
        assert_eq!(
            read_cache_at(&path).map(|n| n.latest),
            Some("9.9.9".into()),
            "a moved clock turned the throttle off"
        );

        // Genuinely old is still old, or the throttle never lets go.
        stamped(now - 2 * 24 * 60 * 60);
        assert_eq!(read_cache_at(&path), None);

        // And a file that says nothing readable is no cache at all.
        std::fs::write(&path, "9.9.9\nnot a stamp\n").unwrap();
        assert_eq!(read_cache_at(&path), None);
        let _ = std::fs::remove_dir_all(&dir);
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
