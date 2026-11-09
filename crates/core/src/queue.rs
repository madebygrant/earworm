//! Links waiting for the worker. Shared with the front ends rather than sent as a `Cmd`, because the worker
//! reads no commands during a run, which is exactly when somebody pastes the next batch.

use std::collections::{HashSet, VecDeque};
use std::sync::{Mutex, MutexGuard};

use crate::cheats::Unlocked;
use crate::protocol::Msg;
use crate::worker::{settle_url, split_url};

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
fn key(url: &str) -> String {
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
}

/// Each change returns the `Msg::Queue` its caller sends. The queue holds no sender of its own: a front
/// end holding one would keep the worker's channel open after the worker died, and the TUI reads a
/// closed channel as the crash.
#[derive(Default)]
pub struct Queue {
    state: Mutex<State>,
}

impl Queue {
    pub fn add(&self, text: &str, unlocked: &Unlocked) -> (Added, Option<Msg>) {
        let (found, rejected) = links(text, unlocked);
        let mut added = Added { rejected, ..Added::default() };
        let msg = self.change(|state| {
            let mut held: HashSet<String> = state.links.iter().chain(&state.running).map(|l| key(l)).collect();
            for link in found {
                if held.insert(key(&link)) {
                    state.links.push_back(link);
                    added.added += 1;
                } else {
                    added.repeats += 1;
                }
            }
            added.added > 0
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

    fn change(&self, edit: impl FnOnce(&mut State) -> bool) -> Option<Msg> {
        let mut state = self.lock();
        if !edit(&mut state) {
            return None;
        }
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
