//! Links waiting for the worker. Shared with the front ends rather than sent as a `Cmd`, because the worker
//! reads no commands during a run, which is exactly when somebody pastes the next batch.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use crate::cheats::Unlocked;
use crate::protocol::Msg;
use crate::worker::{settle_url, split_url};

// Beside the playlist folders, like `.artists`: a dotfile, so the library walk never lists it.
const FILE: &str = ".queue";

fn file(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// The links left when earworm last closed, the one it was downloading first.
pub fn saved(dir: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(file(dir)).unwrap_or_default();
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

/// Drops the file, for a start whose answer was not to resume.
pub fn forget(dir: &Path) {
    let _ = std::fs::remove_file(file(dir));
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub text: String,
    pub why: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Added {
    pub added: usize,
    /// Repeated within the paste, already waiting, or the link running now.
    pub repeats: usize,
    pub rejected: Vec<Rejected>,
}

impl Added {
    /// What a front end flashes after a paste. One wording, so the two front ends cannot say it differently.
    pub fn said(&self) -> String {
        let mut said = match (self.added, self.repeats) {
            (0, 0) if self.rejected.is_empty() => "no links in that paste".to_string(),
            (0, 0) => "nothing queued".to_string(),
            (0, _) => "already queued".to_string(),
            (n, 0) => format!("queued {n}  ·  Q shows them"),
            (n, r) => format!("queued {n}  ·  {r} already there  ·  Q shows them"),
        };
        if !self.rejected.is_empty() {
            said = format!("{said}  ·  {} refused  ·  l has which", self.rejected.len());
        }
        said
    }
}

/// Every link in pasted text, in order. Links only: a file path would mean reading the disk, and this
/// runs on the UI thread. Words and list bullets around the links are passed over rather than refused.
pub fn links(text: &str, unlocked: &Unlocked) -> (Vec<String>, Vec<Rejected>) {
    let mut links = Vec::new();
    let mut rejected = Vec::new();
    for token in text.split_whitespace().filter_map(candidate) {
        match settle_url(token, unlocked) {
            Some(Ok(url)) => links.push(url),
            Some(Err(why)) => rejected.push(Rejected { text: token.into(), why: why.into() }),
            None => rejected.push(Rejected { text: token.into(), why: "not a YouTube link".into() }),
        }
    }
    (links, rejected)
}

// A link as it sits in prose or markdown: `[name](url)`, `(url)`, `url,`.
fn candidate(token: &str) -> Option<&str> {
    let token = token.rsplit_once("](").map_or(token, |(_, url)| url);
    let token = token.trim_matches(|c| "()[]<>\"'`,;.!*".contains(c));
    (token.contains("://") || token.contains("youtu")).then_some(token)
}

/// What makes two links the same download: the host without `www.` or `m.`, the route, and only the
/// `v` and `list` parameters, so `youtu.be/x`, `watch?v=x&si=...` and `WWW.YouTube.com` all match.
pub(crate) fn key(url: &str) -> String {
    let Some((host, path)) = split_url(url) else { return url.to_string() };
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let host = host.trim_start_matches("www.").trim_start_matches("m.");
    if host == "youtu.be" {
        return format!("youtube.com/watch?v={}", route.trim_end_matches('/'));
    }
    let mut params: Vec<&str> = query.split('&').filter(|p| p.starts_with("v=") || p.starts_with("list=")).collect();
    params.sort_unstable();
    format!("{host}/{}?{}", route.trim_end_matches('/'), params.join("&"))
}

#[derive(Default)]
struct State {
    links: VecDeque<String>,
    running: Option<String>,
    rev: u64,
    /// Saved links set aside this session, by an Esc on the offer or a lock, carried into every save.
    kept: Vec<String>,
    /// This session loaded or changed the queue. Until it has, a save leaves the file alone: it is the last
    /// session's, or another front end's open on the same folder.
    touched: bool,
}

/// Each change returns the `Msg::Queue` its caller sends. The queue holds no sender of its own: a front
/// end holding one would keep the worker's channel open after the worker died, and the TUI reads a
/// closed channel as the crash.
#[derive(Default)]
pub struct Queue {
    state: Mutex<State>,
    // The worker's save and a front end's exit save can meet, and `write_atomically` has one temp file per process.
    saving: Mutex<()>,
}

impl Queue {
    pub fn add(&self, text: &str, unlocked: &Unlocked) -> (Added, Option<Msg>) {
        self.add_behind(None, text, unlocked)
    }

    /// `add`, for links pasted behind `lead`, which the URL prompt is about to run: it counts as a repeat, and
    /// leaves the queue if it was waiting there, or the queue would download it again.
    pub fn add_behind(&self, lead: Option<&str>, text: &str, unlocked: &Unlocked) -> (Added, Option<Msg>) {
        let (found, rejected) = links(text, unlocked);
        let mut added = Added { rejected, ..Added::default() };
        let msg = self.change(|state| {
            // The prompt runs it now, so a quit before it finishes saves it first, as a queued link would be.
            if let Some(lead) = lead {
                state.running = Some(lead.to_string());
                state.touched = true;
            }
            let lead = lead.map(key);
            let before = state.links.len();
            state.links.retain(|l| Some(key(l)) != lead);
            let mut held: HashSet<String> = state.links.iter().chain(&state.running).map(|l| key(l)).chain(lead.clone()).collect();
            let mut lead_seen = false;
            for link in found {
                // The lead itself, once: it is the prompt's answer, not a repeat.
                if !lead_seen && lead.as_deref() == Some(key(&link).as_str()) {
                    lead_seen = true;
                    continue;
                }
                if held.insert(key(&link)) {
                    state.links.push_back(link);
                    added.added += 1;
                } else {
                    added.repeats += 1;
                }
            }
            added.added > 0 || state.links.len() != before + added.added
        });
        (added, msg)
    }

    /// The next link, which counts as a repeat until `done` says its run is over.
    pub fn pop(&self) -> Option<(String, Msg)> {
        let mut next = None;
        let msg = self.change(|state| {
            next = state.links.pop_front();
            state.running.clone_from(&next);
            next.is_some()
        })?;
        next.map(|link| (link, msg))
    }

    pub fn done(&self) {
        self.lock().running = None;
    }

    /// Sets saved links aside without queueing them, so the next save keeps them for the start after.
    pub fn keep(&self, links: &[String]) {
        let mut state = self.lock();
        state.kept.extend(links.iter().cloned());
        state.touched = true;
    }

    // By the link, not its row: the worker can pop the head between a front end drawing the list and the
    // key that removes from it, and a stale row number would take the next link instead.
    pub fn remove(&self, link: &str) -> Option<Msg> {
        self.change(|state| {
            let at = state.links.iter().position(|l| l == link);
            at.and_then(|at| state.links.remove(at)).is_some()
        })
    }

    pub fn clear(&self) -> Option<Msg> {
        self.change(|state| {
            let had = !state.links.is_empty();
            state.links.clear();
            had
        })
    }

    pub fn links(&self) -> Vec<String> {
        self.lock().links.iter().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().links.is_empty()
    }

    /// Writes what is left for the next start, the link running now first, since quitting stops it short.
    /// Nothing left removes the file. Called by the worker as it takes links and by a front end on its way
    /// out, never on every change: those come from a UI thread, and the worker owns the file writes.
    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        let _one = self.saving.lock().unwrap_or_else(|e| e.into_inner());
        let left: Vec<String> = {
            let state = self.lock();
            if !state.touched {
                return Ok(());
            }
            state.running.iter().chain(&state.links).chain(&state.kept).cloned().collect()
        };
        if left.is_empty() {
            forget(dir);
            return Ok(());
        }
        crate::config::write_atomically(&file(dir), &(left.join("\n") + "\n"))
    }

    fn change(&self, edit: impl FnOnce(&mut State) -> bool) -> Option<Msg> {
        let mut state = self.lock();
        if !edit(&mut state) {
            return None;
        }
        state.touched = true;
        state.rev += 1;
        Some(Msg::Queue { rev: state.rev, links: state.links.iter().cloned().collect() })
    }

    // A thread that panicked holding it left a list of strings, which is still a list of strings.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cheats::{Feature, LOCKED};

    const A: &str = "https://www.youtube.com/playlist?list=PLa";
    const B: &str = "https://www.youtube.com/watch?v=bbbbbbbbbbb";

    fn said(msg: Option<Msg>) -> (u64, Vec<String>) {
        match msg {
            Some(Msg::Queue { rev, links }) => (rev, links),
            _ => panic!("no Msg::Queue"),
        }
    }

    #[test]
    fn a_paste_keeps_its_links_in_order_and_says_what_it_refused() {
        let text = format!("{A}\n  {B}\nhttps://example.com/x\nyoutube.com\nhttps://music.youtube.com/playlist?list=OLAK5uy_x\n");
        let (found, rejected) = links(&text, &Unlocked::default());
        assert_eq!(found, [A, B]);
        assert_eq!(
            rejected,
            [
                Rejected { text: "https://example.com/x".into(), why: "not a YouTube link".into() },
                Rejected { text: "youtube.com".into(), why: "not a YouTube link".into() },
                Rejected { text: "https://music.youtube.com/playlist?list=OLAK5uy_x".into(), why: LOCKED.into() },
            ]
        );
    }

    #[test]
    fn links_in_prose_and_markdown_lose_what_surrounds_them() {
        let text = format!("1. Albums: {A}, and [the video]({B}) (also https://youtu.be/ccccccccccc).");
        let (found, rejected) = links(&text, &Unlocked::default());
        assert_eq!(found, [A, B, "https://youtu.be/ccccccccccc"]);
        assert!(rejected.is_empty(), "words were refused as links: {rejected:?}");
    }

    // `settle_url` is what the URL prompt asks, so the console unlocks both at once.
    #[test]
    fn an_unlocked_session_takes_music_links() {
        let unlocked = Unlocked::default();
        unlocked.grant(Feature::Music);
        let (found, rejected) = links("https://music.youtube.com/playlist?list=OLAK5uy_x", &unlocked);
        assert_eq!(found.len(), 1);
        assert!(rejected.is_empty());
    }

    #[test]
    fn one_download_spelt_several_ways_is_queued_once() {
        let queue = Queue::default();
        let spellings = [
            A,
            "www.youtube.com/playlist?list=PLa",
            "http://WWW.YouTube.com/playlist/?list=PLa&si=tracking",
            B,
            "https://youtu.be/bbbbbbbbbbb",
            "https://m.youtube.com/watch?si=x&v=bbbbbbbbbbb",
        ];
        let (added, _) = queue.add(&spellings.join("\n"), &Unlocked::default());
        assert_eq!((added.added, added.repeats), (2, 4));
        assert_eq!(queue.links(), [A, B]);
    }

    #[test]
    fn every_change_says_so_with_a_higher_rev_and_nothing_else_does() {
        let queue = Queue::default();
        let unlocked = Unlocked::default();

        let (added, msg) = queue.add(A, &unlocked);
        assert_eq!(added, Added { added: 1, ..Added::default() });
        assert_eq!(said(msg), (1, vec![A.to_string()]));
        assert!(queue.add(A, &unlocked).1.is_none(), "a paste of nothing new said something");
        assert_eq!(said(queue.add(B, &unlocked).1), (2, vec![A.into(), B.into()]));

        let (next, msg) = queue.pop().unwrap();
        assert_eq!(next, A);
        assert_eq!(said(Some(msg)), (3, vec![B.to_string()]));
        assert!(queue.remove(A).is_none(), "the running link was removed from the waiting list");
        assert_eq!(said(queue.remove(B)), (4, vec![]));
        assert!(queue.clear().is_none());
        assert!(queue.pop().is_none());
    }

    #[test]
    fn the_running_link_is_a_repeat_until_it_is_done() {
        let queue = Queue::default();
        let unlocked = Unlocked::default();
        queue.add(A, &unlocked);
        queue.pop();
        assert_eq!(queue.add(A, &unlocked).0.repeats, 1, "the link being downloaded was queued again");
        queue.done();
        assert_eq!(queue.add(A, &unlocked).0.added, 1);
    }

    // The link the URL prompt runs is not queued again, spelt another way or already waiting.
    #[test]
    fn links_pasted_behind_the_prompts_lead_skip_it() {
        let queue = Queue::default();
        let unlocked = Unlocked::default();
        queue.add(&format!("{A} {B}"), &unlocked);
        let (added, msg) = queue.add_behind(Some(B), &format!("{B} https://youtu.be/bbbbbbbbbbb https://youtu.be/ccccccccccc"), &unlocked);
        assert_eq!((added.added, added.repeats), (1, 1), "the lead itself counted as a repeat");
        assert_eq!(queue.links(), [A, "https://youtu.be/ccccccccccc"]);
        assert_eq!(said(msg).1, queue.links());
    }

    // The URL prompt runs its link outside the queue, so quitting during it must still save it, first.
    #[test]
    fn the_prompts_lead_counts_as_running_until_done() {
        let dir = std::env::temp_dir().join(format!("earworm-queue-lead-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let queue = Queue::default();
        queue.add_behind(Some(A), &format!("{A} {B}"), &Unlocked::default());
        queue.save(&dir).unwrap();
        assert_eq!(saved(&dir), [A, B]);
        queue.done();
        queue.save(&dir).unwrap();
        assert_eq!(saved(&dir), [B]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // A session that never loaded or changed the queue must not wipe the file the last one left.
    #[test]
    fn an_untouched_queue_leaves_the_file_and_kept_links_ride_along() {
        let dir = std::env::temp_dir().join(format!("earworm-queue-keep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(file(&dir), format!("{A}\n")).unwrap();
        Queue::default().save(&dir).unwrap();
        assert_eq!(saved(&dir), [A], "an untouched session deleted the saved queue");

        let queue = Queue::default();
        queue.keep(&[A.to_string()]);
        queue.add(B, &Unlocked::default());
        queue.pop();
        queue.done();
        queue.save(&dir).unwrap();
        assert_eq!(saved(&dir), [A], "the kept link was lost when the session's own queue ran dry");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_is_left_is_saved_with_the_running_link_first_and_nothing_left_removes_the_file() {
        let dir = std::env::temp_dir().join(format!("earworm-queue-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let queue = Queue::default();
        queue.add(&format!("{A} {B}"), &Unlocked::default());
        queue.pop();
        queue.save(&dir).unwrap();
        assert_eq!(saved(&dir), [A, B], "the link cut short by quitting was not kept");
        queue.done();
        queue.clear();
        queue.save(&dir).unwrap();
        assert!(!file(&dir).exists());
        assert!(saved(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // The worker popping the head between a front end drawing [A, B] and the user removing A must not cost B.
    #[test]
    fn a_remove_after_the_head_was_taken_leaves_the_rest() {
        let queue = Queue::default();
        queue.add(&format!("{A} {B}"), &Unlocked::default());
        queue.pop();
        assert!(queue.remove(A).is_none());
        assert_eq!(queue.links(), [B]);
    }
}
