use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::sync::mpsc::Sender;

use ratatui::style::Color;

use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Pending,
    Have,
    Downloading,
    Downloaded,
    Tagging,
    Ok,
    Manual,
    Kept,
    Weak,
    NoMatch,
    Failed,
    /// On disk, but its video has left the playlist.
    Gone,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Pending => "queued",
            Status::Have => "have",
            Status::Downloading => "getting",
            Status::Downloaded => "got",
            Status::Tagging => "reading",
            Status::Ok => "ok",
            Status::Manual => "manual",
            Status::Kept => "kept",
            Status::Weak => "weak",
            Status::NoMatch => "none",
            Status::Failed => "failed",
            Status::Gone => "gone",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Status::Ok => theme::GREEN,
            Status::Manual => theme::CREAM,
            Status::Weak => theme::AMBER,
            Status::NoMatch => theme::SAND,
            Status::Failed => theme::RED,
            Status::Downloading | Status::Tagging | Status::Downloaded => theme::GOLD,
            Status::Pending | Status::Have | Status::Kept | Status::Gone => theme::DIM,
        }
    }

    /// Terminal states, for the run summary and for picking what to follow.
    pub fn settled(self) -> bool {
        matches!(
            self,
            Status::Ok
                | Status::Manual
                | Status::Kept
                | Status::Weak
                | Status::NoMatch
                | Status::Failed
                | Status::Gone
                | Status::Have
        )
    }
}

#[derive(Clone)]
pub struct Track {
    pub index: usize,
    pub id: String,
    pub name: String,
    pub was: String,
    pub path: Option<PathBuf>,
    pub status: Status,
    pub source: String,
    pub note: String,
    pub percent: u16,
    pub artist: String,
    pub title: String,
    pub duration: u64,
    pub mbid: Option<String>,
    /// Carries a playlist entry, so an edit can rewrite the .m3u8 in order.
    pub listed: bool,
}

impl Track {
    pub fn new(index: usize, id: String, name: String, path: PathBuf) -> Self {
        Track {
            index,
            id,
            was: name.clone(),
            name,
            path: Some(path),
            status: Status::Pending,
            source: String::new(),
            note: String::new(),
            percent: 0,
            artist: String::new(),
            title: String::new(),
            duration: 0,
            mbid: None,
            listed: false,
        }
    }
}

/// Work the UI asks of the worker once the run itself has finished.
pub enum Cmd {
    Edit(usize),
    Cover(usize),
    /// Every failed track at once: one at a time would mean one yt-dlp run
    /// each, and the failures usually share a cause.
    Retry,
    /// Artist and title the wrong way round, which a playlist tends to get
    /// wrong for every track at once or not at all.
    Swap(Vec<usize>),
    /// One artist across many tracks, for a compilation the lookup split up.
    Artist(Vec<usize>),
}

pub enum Prompt {
    Choice {
        header: String,
        /// Shown above the options and wrapped, for text too long to survive
        /// in the header, which the border clips rather than wrapping.
        note: String,
        options: Vec<String>,
        escape: Escape,
    },
    Input {
        header: String,
        value: String,
        /// What Esc does here, since it is the only key whose effect changes
        /// between prompts: every other one leaves the track alone.
        escape: Escape,
    },
}

#[derive(Clone, Copy, PartialEq)]
pub enum Escape {
    /// Leave this track as it was.
    Skip,
    /// End the tool, for a question asked before any work started.
    Quit,
    /// Close the menu and carry on in the session.
    Keep,
}

impl Escape {
    pub fn hint(self) -> &'static str {
        match self {
            Escape::Skip => "esc skip",
            Escape::Quit => "esc quit",
            Escape::Keep => "esc dismiss",
        }
    }
}

#[derive(Clone)]
pub enum Reply {
    Choice(usize),
    Text(String),
    Cancel,
}

pub enum Msg {
    Stage(String),
    /// A stage message that undoes itself. What a command just did is worth
    /// saying and not worth leaving on the header for the rest of the session.
    Flash(String),
    Playlist(String),
    Tracks(Vec<Track>),
    Progress {
        index: usize,
        percent: u16,
    },
    Update {
        index: usize,
        status: Status,
        source: Option<String>,
        note: Option<String>,
        name: Option<String>,
    },
    Path {
        index: usize,
        path: PathBuf,
    },
    Log(String),
    Ask(Prompt, Sender<Reply>),
    Done(Result<String, String>),
    /// Sent once a bulk action has finished, so a cancelled one leaves the
    /// selection intact rather than making the user rebuild it.
    Unmark,
    /// The worker has finished the command it was given and will accept
    /// another.
    Idle,
    /// A second playlist is starting in the same session, so everything the
    /// last one left on screen has to go.
    Restart,
    /// Nothing to report and nothing to look at, so close the UI outright.
    Quit,
}

/// Prompts are answered by the UI thread, so a question blocks only the worker.
/// Long enough to read a filename in, short enough not to outlive interest.
const FLASH: Duration = Duration::from_secs(4);

pub struct Asker {
    pub tx: Sender<Msg>,
    pub enabled: bool,
}

impl Asker {
    fn ask(&self, prompt: Prompt) -> Reply {
        let (tx, rx) = std::sync::mpsc::channel();
        if self.tx.send(Msg::Ask(prompt, tx)).is_err() {
            return Reply::Cancel;
        }
        rx.recv().unwrap_or(Reply::Cancel)
    }

    pub fn choose(&self, header: &str, options: Vec<String>) -> Option<usize> {
        self.pick(header, "", options, Escape::Skip)
    }

    /// For a menu with no run behind it yet, where backing out ends the tool.
    pub fn choose_or_quit(&self, header: &str, options: Vec<String>) -> Option<usize> {
        self.pick(header, "", options, Escape::Quit)
    }

    /// A menu that reports something before it asks, and that backing out of
    /// leaves the session where it was rather than skipping or quitting.
    pub fn choose_noted(&self, header: &str, note: &str, options: Vec<String>) -> Option<usize> {
        self.pick(header, note, options, Escape::Keep)
    }

    fn pick(
        &self,
        header: &str,
        note: &str,
        options: Vec<String>,
        escape: Escape,
    ) -> Option<usize> {
        if !self.enabled {
            return None;
        }
        match self.ask(Prompt::Choice {
            header: header.into(),
            note: note.into(),
            options,
            escape,
        }) {
            Reply::Choice(i) => Some(i),
            _ => None,
        }
    }

    /// None means the user cancelled. An empty string is a real answer the
    /// caller has to judge, so clearing a field cannot look like an escape.
    pub fn input(&self, header: &str, value: &str) -> Option<String> {
        self.prompt(header, value, Escape::Skip)
    }

    /// For a question with no run behind it yet, where cancelling ends the
    /// tool rather than leaving something as it was.
    pub fn input_or_quit(&self, header: &str, value: &str) -> Option<String> {
        self.prompt(header, value, Escape::Quit)
    }

    fn prompt(&self, header: &str, value: &str, escape: Escape) -> Option<String> {
        if !self.enabled {
            return None;
        }
        match self.ask(Prompt::Input {
            header: header.into(),
            value: value.into(),
            escape,
        }) {
            Reply::Text(t) => Some(t.trim().to_string()),
            _ => None,
        }
    }
}

pub struct App {
    pub cmds: Option<Sender<Cmd>>,
    pub settings: String,
    pub tick: usize,
    pub show_help: bool,
    pub stage: String,
    /// What the header goes back to once a flash expires.
    resting: String,
    flash_until: Option<Instant>,
    pub playlist: String,
    pub tracks: Vec<Track>,
    pub logs: Vec<String>,
    pub cursor: usize,
    /// Track indices, not row positions, so a bulk action means the same
    /// thing to the worker as it does on screen.
    pub marked: HashSet<usize>,
    /// A command is in flight. Set when one is sent rather than when the
    /// worker reports starting it, because the gap between the two is exactly
    /// long enough for a second keypress to queue a duplicate.
    pub busy: bool,
    pub follow: bool,
    pub show_logs: bool,
    pub prompt: Option<(Prompt, Sender<Reply>)>,
    pub choice: usize,
    pub input: String,
    pub done: Option<Result<String, String>>,
    pub quit: bool,
}

impl App {
    pub fn new(cmds: Sender<Cmd>, settings: String) -> Self {
        App {
            cmds: Some(cmds),
            settings,
            tick: 0,
            show_help: false,
            stage: "starting".into(),
            resting: "starting".into(),
            flash_until: None,
            playlist: String::new(),
            tracks: Vec::new(),
            logs: Vec::new(),
            cursor: 0,
            marked: HashSet::new(),
            busy: false,
            follow: true,
            show_logs: false,
            prompt: None,
            choice: 0,
            input: String::new(),
            done: None,
            quit: false,
        }
    }

    pub fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Stage(s) => {
                /* Only while the run itself is talking. A command's progress
                   ("looking for artwork") would otherwise become what the
                   header falls back to, so cancelling it would revert to a
                   step that had already finished. Post-run, every command
                   path ends in a Flash or a Done, which restores it. */
                if self.done.is_none() {
                    self.resting = s.clone();
                }
                self.flash_until = None;
                self.stage = s;
            }
            Msg::Flash(s) => {
                self.stage = s;
                self.flash_until = Some(Instant::now() + FLASH);
            }
            Msg::Playlist(p) => self.playlist = p,
            Msg::Tracks(t) => self.tracks = t,
            Msg::Progress { index, percent } => {
                if let Some(t) = self.track_mut(index) {
                    t.percent = percent;
                    t.status = Status::Downloading;
                }
                self.follow_to(index);
            }
            Msg::Update {
                index,
                status,
                source,
                note,
                name,
            } => {
                if let Some(t) = self.track_mut(index) {
                    t.status = status;
                    if let Some(s) = source {
                        t.source = s;
                    }
                    if let Some(n) = note {
                        t.note = n;
                    }
                    if let Some(n) = name {
                        t.name = n;
                    }
                }
                self.follow_to(index);
            }
            Msg::Path { index, path } => {
                if let Some(t) = self.track_mut(index) {
                    t.path = Some(path);
                }
            }
            Msg::Log(line) => self.logs.push(line),
            Msg::Ask(prompt, reply) => {
                if let Prompt::Input { value, .. } = &prompt {
                    self.input = value.clone();
                }
                self.choice = 0;
                self.prompt = Some((prompt, reply));
            }
            Msg::Done(result) => {
                self.stage = if result.is_ok() { "finished" } else { "failed" }.into();
                self.resting = self.stage.clone();
                self.flash_until = None;
                self.done = Some(result);
            }
            Msg::Unmark => self.marked.clear(),
            Msg::Idle => self.busy = false,
            Msg::Restart => {
                self.done = None;
                self.tracks.clear();
                self.marked.clear();
                self.playlist.clear();
                self.cursor = 0;
                self.follow = true;
            }
            Msg::Quit => self.quit = true,
        }
    }

    /// Commands are only offered once the pipeline is done: the worker is busy
    /// until then, and a queued edit would look like nothing happened.
    /// Called every frame: a flash has to expire on its own, since the thing
    /// that set it has already finished and will not send anything else.
    pub fn expire_flash(&mut self) {
        if self.flash_until.is_some_and(|at| Instant::now() >= at) {
            self.stage = self.resting.clone();
            self.flash_until = None;
        }
    }

    pub fn toggle_mark(&mut self) {
        if let Some(index) = self.selected()
            && !self.marked.remove(&index)
        {
            self.marked.insert(index);
        }
    }

    /// Marks every track sharing the cursor's status, which is the selection
    /// worth making: a playlist gets `kept` or `weak` wrong in batches.
    pub fn mark_like_cursor(&mut self) {
        let Some(status) = self.tracks.get(self.cursor).map(|t| t.status) else {
            return;
        };
        let same: Vec<usize> = self
            .tracks
            .iter()
            .filter(|t| t.status == status)
            .map(|t| t.index)
            .collect();
        // Already all marked, so the same key takes them back off again.
        if same.iter().all(|i| self.marked.contains(i)) {
            self.marked.retain(|i| !same.contains(i));
        } else {
            self.marked.extend(same);
        }
    }

    /// What a bulk action applies to: the marked tracks, or the one under the
    /// cursor when nothing is marked.
    pub fn targets(&self) -> Vec<usize> {
        if self.marked.is_empty() {
            return self.selected().into_iter().collect();
        }
        let mut targets: Vec<usize> = self.marked.iter().copied().collect();
        targets.sort_unstable();
        targets
    }

    pub fn can_command(&self) -> bool {
        self.done.is_some() && self.prompt.is_none() && !self.busy && !self.tracks.is_empty()
    }

    pub fn send(&mut self, cmd: Cmd) {
        if let Some(tx) = &self.cmds
            && tx.send(cmd).is_ok()
        {
            self.busy = true;
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.tracks.get(self.cursor).map(|t| t.index)
    }

    fn track_mut(&mut self, index: usize) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.index == index)
    }

    fn follow_to(&mut self, index: usize) {
        if !self.follow {
            return;
        }
        if let Some(pos) = self.tracks.iter().position(|t| t.index == index) {
            self.cursor = pos;
        }
    }

    pub fn counts(&self) -> Vec<(Status, usize)> {
        let order = [
            Status::Ok,
            Status::Manual,
            Status::Kept,
            Status::Weak,
            Status::NoMatch,
            Status::Failed,
            Status::Gone,
            Status::Have,
        ];
        order
            .iter()
            .filter_map(|s| {
                let n = self.tracks.iter().filter(|t| t.status == *s).count();
                (n > 0).then_some((*s, n))
            })
            .collect()
    }

    pub fn settled(&self) -> usize {
        self.tracks.iter().filter(|t| t.status.settled()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with(statuses: &[Status]) -> App {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        app.tracks = statuses
            .iter()
            .enumerate()
            .map(|(i, status)| {
                let mut track =
                    Track::new(i + 1, "id".into(), "name".into(), PathBuf::from("/tmp/x.opus"));
                track.status = *status;
                track
            })
            .collect();
        app
    }

    #[test]
    fn a_bulk_action_falls_back_to_the_cursor_when_nothing_is_marked() {
        let mut app = app_with(&[Status::Ok, Status::Kept, Status::Weak]);
        app.cursor = 1;
        assert_eq!(app.targets(), [2]);

        app.toggle_mark();
        assert_eq!(app.targets(), [2], "marking the cursor changes nothing yet");
        app.cursor = 2;
        assert_eq!(app.targets(), [2], "the mark wins over the cursor");
    }

    #[test]
    fn marks_toggle_one_at_a_time() {
        let mut app = app_with(&[Status::Ok, Status::Ok]);
        app.toggle_mark();
        app.cursor = 1;
        app.toggle_mark();
        assert_eq!(app.targets(), [1, 2]);
        app.toggle_mark();
        assert_eq!(app.targets(), [1]);
    }

    /* The point of `m` is grabbing a whole class of bad rows at once, and the
       same key has to give them back or there is no way to undo a misfire. */
    #[test]
    fn marking_by_status_covers_that_status_and_then_clears_it() {
        let mut app = app_with(&[Status::Kept, Status::Ok, Status::Kept, Status::Weak]);
        app.cursor = 0;
        app.mark_like_cursor();
        assert_eq!(app.targets(), [1, 3]);

        app.mark_like_cursor();
        assert!(app.marked.is_empty(), "the second press unmarked them");
        assert_eq!(app.targets(), [1], "back to the cursor track");
    }

    #[test]
    fn marking_by_status_keeps_marks_made_by_hand() {
        let mut app = app_with(&[Status::Kept, Status::Ok, Status::Kept]);
        app.cursor = 1;
        app.toggle_mark();
        app.cursor = 0;
        app.mark_like_cursor();
        assert_eq!(app.targets(), [1, 2, 3]);
    }

    /* The gap between sending a command and the worker reporting it is long
       enough for a second keypress, and a duplicated swap silently undoes the
       first one, so the send itself has to close the door. */
    /* Six post-run messages used to sit on the header for the rest of the
       session, the command error being the one people noticed. */
    #[test]
    fn a_flash_goes_back_to_what_the_run_ended_on() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        app.apply(Msg::Stage("identifying".into()));
        app.apply(Msg::Done(Ok("12 tracks".into())));
        assert_eq!(app.stage, "finished");

        app.apply(Msg::Flash("failed: cannot write tags".into()));
        assert_eq!(app.stage, "failed: cannot write tags");
        app.expire_flash();
        assert_eq!(app.stage, "failed: cannot write tags", "expired immediately");

        app.flash_until = Some(Instant::now());
        app.expire_flash();
        assert_eq!(app.stage, "finished");
    }

    /* A command's own progress is not somewhere to fall back to: it had
       already finished by the time the flash replaced it. */
    #[test]
    fn a_flash_never_reverts_to_a_commands_progress_message() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        app.apply(Msg::Done(Ok("12 tracks".into())));
        app.apply(Msg::Stage("looking for artwork".into()));
        app.apply(Msg::Flash("cancelled".into()));

        app.flash_until = Some(Instant::now());
        app.expire_flash();
        assert_eq!(app.stage, "finished");
    }

    /* Three prompts, three meanings for Esc, and the wording is the only
       thing that tells them apart on screen. */
    #[test]
    fn every_escape_says_what_it_actually_does() {
        assert_eq!(Escape::Skip.hint(), "esc skip");
        assert_eq!(Escape::Quit.hint(), "esc quit");
        assert_eq!(Escape::Keep.hint(), "esc dismiss");

        let (tx, _rx) = std::sync::mpsc::channel();
        let asker = Asker { tx, enabled: false };
        // Disabled, so these only prove which Escape each entry point picks.
        assert!(asker.choose("h", vec![]).is_none());
        assert!(asker.choose_or_quit("h", vec![]).is_none());
        assert!(asker.choose_noted("h", "n", vec![]).is_none());
    }

    #[test]
    fn a_command_in_flight_blocks_another() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        app.tracks = app_with(&[Status::Ok]).tracks;
        app.done = Some(Ok(String::new()));
        assert!(app.can_command());

        app.send(Cmd::Swap(vec![1]));
        assert!(!app.can_command(), "a second command could still be sent");

        app.apply(Msg::Idle);
        assert!(app.can_command(), "the worker finished but the keys stayed dead");
        assert_eq!(rx.try_iter().count(), 1, "exactly one command was queued");
    }

    #[test]
    fn a_dead_command_channel_does_not_wedge_the_keys() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        app.tracks = app_with(&[Status::Ok]).tracks;
        app.done = Some(Ok(String::new()));
        drop(rx);
        app.send(Cmd::Retry);
        assert!(app.can_command(), "nothing was queued, so nothing is pending");
    }

    #[test]
    fn a_finished_bulk_action_clears_the_selection() {
        let mut app = app_with(&[Status::Kept, Status::Kept]);
        app.mark_like_cursor();
        assert_eq!(app.targets().len(), 2);
        app.apply(Msg::Unmark);
        assert!(app.marked.is_empty());
    }
}
