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
    /// In the playlist and not on disk, because the run was never asked to
    /// fetch it. Distinct from `Failed`, which would overstate what went wrong.
    Skipped,
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
            Status::Skipped => "skipped",
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
            Status::Pending | Status::Have | Status::Kept | Status::Gone | Status::Skipped => {
                theme::DIM
            }
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
                | Status::Skipped
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

/// One already-downloaded playlist folder, read from its manifest alone so the
/// library opens with no network and no yt-dlp.
#[derive(Clone)]
pub struct Shelf {
    pub path: PathBuf,
    pub name: String,
    pub url: String,
    pub tracks: usize,
    /// Entries whose file has since been deleted. Not the same as a track that
    /// left the playlist, which needs a listing to know about.
    pub missing: usize,
    pub synced: Option<u64>,
    /// Filename and whether it is still on disk, in playlist order. Resolved
    /// here because the draw loop must not stat, and the filename carries the
    /// number and tags already, so the preview needs no tag read.
    pub files: Vec<(String, bool)>,
}

/// Which screen has the keys. The library is a screen and not a prompt because
/// its rows stay open: opening one and pressing Esc comes back to it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Tracks,
    Library,
}

/// Work the UI asks of the worker once the run itself has finished.
pub enum Cmd {
    Edit(usize),
    Cover(usize),
    /// Load a library row from its manifest, with no network behind it. The
    /// folder and not the row: the worker re-reads the library, and a row
    /// position would then name whatever had moved into it.
    Open(PathBuf),
    /// Sync the playlist that is open, from the URL its manifest recorded.
    SyncOne,
    ResyncAll,
    /// Ask for a URL and sync it, which is the only way into a new playlist
    /// once the library screen has replaced the opening question.
    Url,
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
    /// Track indices the run should download. Empty is a real answer: tag what
    /// is already on disk and fetch nothing.
    Picked(Vec<usize>),
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
    /// The playlists on disk. Re-sent after every sync, which is what changes
    /// the counts, and `show` is false then: the run's own tracks are what the
    /// user is looking at and must not be replaced under them.
    Library { shelves: Vec<Shelf>, show: bool },
    Ask(Prompt, Sender<Reply>),
    /// Hand the track list to the user to choose from before downloading.
    /// Not a `Prompt`: the answer is made with the list's own cursor, marks
    /// and filter, which a popup would cover up.
    Pick(Sender<Reply>),
    /// A run has settled. `stage` is the header word for a success, since not
    /// everything that settles has run: opening a folder from the library
    /// downloads nothing and "finished" would be a claim about work.
    Done {
        result: Result<String, String>,
        stage: &'static str,
    },
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

    /// For a question whose cancel means something other than "leave this
    /// track alone": ending the tool, or going back to the screen behind it.
    pub fn input_with(&self, header: &str, value: &str, escape: Escape) -> Option<String> {
        self.prompt(header, value, escape)
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

fn matched(track: &Track, needle: &str) -> bool {
    track.name.to_lowercase().contains(needle) || track.status.label().contains(needle)
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
    pub view: View,
    pub library: Vec<Shelf>,
    /// Cursor in the library, kept apart from the track cursor so coming back
    /// to the library lands on the row you left from.
    pub shelf: usize,
    pub cursor: usize,
    /// Narrows what the track list shows. A view over `tracks` and nothing
    /// more: `cursor` stays a position in `tracks` and `marked` stays a set of
    /// track indices, so no command means anything different under a filter.
    pub filter: String,
    /// The filter box has the keys, so `q` types a letter rather than quitting.
    pub typing_filter: bool,
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
    /// The worker is blocked waiting for the tracks to download. Holding the
    /// reply channel here is what makes the keys mean "choose" for as long as
    /// the question is open, and dropping it answers `Cancel`.
    pub picking: Option<Sender<Reply>>,
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
            view: View::Tracks,
            library: Vec::new(),
            shelf: 0,
            cursor: 0,
            filter: String::new(),
            typing_filter: false,
            marked: HashSet::new(),
            busy: false,
            follow: true,
            show_logs: false,
            prompt: None,
            choice: 0,
            input: String::new(),
            done: None,
            picking: None,
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
            Msg::Library { shelves, show } => {
                self.library = shelves;
                self.shelf = self.shelf.min(self.library.len().saturating_sub(1));
                if show {
                    self.view = View::Library;
                }
            }
            /* Opens with everything that needs fetching already marked, so
               Enter is the whole playlist and the work is unmarking what you
               do not want. Starting empty would make "download all" the
               laborious answer and the accidental one a silent no-op. */
            Msg::Pick(reply) => {
                self.marked = self
                    .tracks
                    .iter()
                    .filter(|t| !matches!(t.status, Status::Have | Status::Gone))
                    .map(|t| t.index)
                    .collect();
                self.follow = false;
                self.picking = Some(reply);
            }
            Msg::Ask(prompt, reply) => {
                if let Prompt::Input { value, .. } = &prompt {
                    self.input = value.clone();
                }
                self.choice = 0;
                self.prompt = Some((prompt, reply));
            }
            Msg::Done { result, stage } => {
                self.stage = if result.is_ok() { stage } else { "failed" }.into();
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
                /* A filter from the last playlist would hide most of the next
                   one, which reads as a broken run rather than a filter. */
                self.filter.clear();
                self.typing_filter = false;
                // Whatever is starting has tracks, and they are the point.
                self.view = View::Tracks;
            }
            Msg::Quit => self.quit = true,
        }
        /* Any message can change what the filter shows: `Update` carries a new
           name, and it and `Progress` change the status word. A cursor left on
           a row that has stopped matching is a selection nobody can see, and
           every command reads as dead until the next keypress moves it. */
        self.snap();
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
        /* Only what is on screen: `/` then `m` is how you mark one artist's
           bad rows, and reaching past the filter would mark the whole run. */
        let same: Vec<usize> = self
            .tracks
            .iter()
            .filter(|t| t.status == status && self.shows(t))
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

    /// Every row the filter shows, marked or unmarked together. `m` stops at
    /// the filter for the same reason, so this is the only way to reach the
    /// whole playlist in one key once a filter is on.
    pub fn toggle_all(&mut self) {
        let shown: Vec<usize> = self
            .tracks
            .iter()
            .filter(|t| self.shows(t))
            .map(|t| t.index)
            .collect();
        if shown.iter().all(|i| self.marked.contains(i)) {
            self.marked.retain(|i| !shown.contains(i));
        } else {
            self.marked.extend(shown);
        }
    }

    /// Answers the pick with the marked set. The marks and not `targets()`:
    /// falling back to the cursor would turn "I unmarked everything" into
    /// "download the row I happen to be on".
    pub fn confirm_pick(&mut self) {
        let Some(reply) = self.picking.take() else {
            return;
        };
        let mut picked: Vec<usize> = self.marked.iter().copied().collect();
        picked.sort_unstable();
        let _ = reply.send(Reply::Picked(picked));
        self.marked.clear();
        self.follow = true;
    }

    /// Esc: download nothing and tag what is already there. Not a quit, since
    /// a folder of existing files is still worth the rest of the run.
    pub fn cancel_pick(&mut self) {
        if let Some(reply) = self.picking.take() {
            let _ = reply.send(Reply::Picked(Vec::new()));
            self.marked.clear();
            self.follow = true;
        }
    }

    pub fn can_command(&self) -> bool {
        self.view == View::Tracks
            && self.done.is_some()
            && self.prompt.is_none()
            && !self.busy
            && !self.tracks.is_empty()
    }

    /// Library keys, which unlike the track ones need no finished run behind
    /// them: a bare start with playlists on disk lands here having synced
    /// nothing.
    pub fn selected_shelf(&self) -> Option<&Shelf> {
        self.library.get(self.shelf)
    }

    /* Not gated on the library having rows: `n` is the way out of an empty
       one, and `Cmd::Open` is guarded by there being a row to open. */
    pub fn can_browse(&self) -> bool {
        self.view == View::Library && self.prompt.is_none() && !self.busy
    }

    /// Esc in the track view. Goes back to the library when there is one, and
    /// otherwise means what it always did.
    pub fn leave_tracks(&mut self) {
        if self.library.is_empty() || self.busy {
            self.quit = true;
        } else {
            self.view = View::Library;
        }
    }

    pub fn send(&mut self, cmd: Cmd) {
        if let Some(tx) = &self.cmds
            && tx.send(cmd).is_ok()
        {
            self.busy = true;
        }
    }

    /// Filtered out means not selected: otherwise `e` would edit a track that
    /// is not on screen.
    pub fn selected(&self) -> Option<usize> {
        self.tracks
            .get(self.cursor)
            .filter(|t| self.shows(t))
            .map(|t| t.index)
    }

    /// Whether the track list is narrowed or about to be. The library screen
    /// has its own cursor and the filter does not reach it.
    pub fn filtering(&self) -> bool {
        self.view == View::Tracks && (self.typing_filter || !self.filter.is_empty())
    }

    /// Whether a row survives the filter, matched against the name and the
    /// status word: the artist or title you are looking for, or `/failed`.
    pub fn shows(&self, track: &Track) -> bool {
        self.filter.is_empty() || matched(track, &self.filter.to_lowercase())
    }

    /// Positions in `tracks` the list is showing, in order.
    pub fn rows(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.tracks.len()).collect();
        }
        // Lowercased once for the pass, not once per row.
        let needle = self.filter.to_lowercase();
        self.tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| matched(t, &needle))
            .map(|(pos, _)| pos)
            .collect()
    }

    /// How many rows are showing, for the header. Counted rather than
    /// collected, since the positions themselves are not wanted.
    pub fn shown(&self) -> usize {
        if self.filter.is_empty() {
            return self.tracks.len();
        }
        let needle = self.filter.to_lowercase();
        self.tracks.iter().filter(|t| matched(t, &needle)).count()
    }

    /// Which drawn row the cursor is on, given the view `rows` describes. Takes
    /// it rather than recomputing, so the caller's view and the highlight can
    /// never be answers to two different questions.
    pub fn row_of_cursor(&self, rows: &[usize]) -> Option<usize> {
        rows.iter().position(|pos| *pos == self.cursor)
    }

    /// Puts the cursor back on a row that shows. Called after every change to
    /// the filter, since a cursor on a hidden track is a selection nobody can
    /// see and every command would still act on it.
    pub fn snap(&mut self) {
        // Nothing is hidden, so the cursor is already on a row that shows.
        if self.filter.is_empty() {
            return;
        }
        let rows = self.rows();
        if rows.is_empty() || rows.contains(&self.cursor) {
            return;
        }
        self.cursor = rows
            .iter()
            .copied()
            .min_by_key(|pos| pos.abs_diff(self.cursor))
            .unwrap_or(self.cursor);
    }

    /// One row down or up through what is showing.
    pub fn step(&mut self, down: bool) {
        self.follow = false;
        let rows = self.rows();
        let Some(at) = rows.iter().position(|pos| *pos == self.cursor) else {
            self.snap();
            return;
        };
        let next = if down {
            (at + 1).min(rows.len().saturating_sub(1))
        } else {
            at.saturating_sub(1)
        };
        self.cursor = rows[next];
    }

    /// Stops following, like a step does: `g` and `G` are someone taking the
    /// cursor somewhere, and the next progress message would otherwise drag it
    /// straight back to the active track.
    pub fn jump(&mut self, last: bool) {
        self.follow = false;
        let rows = self.rows();
        let at = if last { rows.last() } else { rows.first() };
        if let Some(pos) = at {
            self.cursor = *pos;
        }
    }

    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.typing_filter = false;
        self.snap();
    }

    fn track_mut(&mut self, index: usize) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.index == index)
    }

    fn follow_to(&mut self, index: usize) {
        if !self.follow {
            return;
        }
        if let Some(pos) = self
            .tracks
            .iter()
            .position(|t| t.index == index && self.shows(t))
        {
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
            Status::Skipped,
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
        app.apply(Msg::Done {
            result: Ok("12 tracks".into()),
            stage: "finished",
        });
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
        app.apply(Msg::Done {
            result: Ok("12 tracks".into()),
            stage: "finished",
        });
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
        assert!(asker.choose_noted("h", "n", vec![]).is_none());
        assert!(asker.input_with("h", "", Escape::Quit).is_none());
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

    /// Tracks with names to filter on, statuses in order Ok, Failed, Ok...
    fn named(names: &[&str]) -> App {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        app.tracks = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let mut track = Track::new(
                    i + 1,
                    "id".into(),
                    (*name).into(),
                    PathBuf::from("/tmp/x.opus"),
                );
                track.status = if i % 2 == 0 { Status::Ok } else { Status::Failed };
                track
            })
            .collect();
        app
    }

    /* The cursor stays a position in `tracks` while the list shows a subset,
       so every movement has to step over what is hidden. A cursor left on a
       hidden row is a selection nobody can see that every command still acts
       on. */
    #[test]
    fn moving_under_a_filter_only_lands_on_rows_that_show() {
        let mut app = named(&["Aphex Twin - Xtal", "Boards of Canada - Roygbiv", "Aphex Twin - Avril"]);
        app.filter = "aphex".into();
        assert_eq!(app.rows(), [0, 2]);

        app.cursor = 0;
        app.step(true);
        assert_eq!(app.cursor, 2, "j landed on the hidden middle row");
        assert_eq!(app.selected(), Some(3));
        app.step(true);
        assert_eq!(app.cursor, 2, "j ran off the end of the filtered list");
        app.step(false);
        assert_eq!(app.cursor, 0);

        app.jump(true);
        assert_eq!(app.cursor, 2, "G left the filtered list");
        app.jump(false);
        assert_eq!(app.cursor, 0);
    }

    /* Every other way of moving the cursor by hand stops the run dragging it
       back, and `g`/`G` reading as "jump, then get pulled to wherever the
       download is" makes the keys look broken during a run. */
    #[test]
    fn jumping_to_an_end_stops_following_the_active_track() {
        let mut app = named(&["One", "Two", "Three"]);
        assert!(app.follow, "a run follows by default");

        app.jump(true);
        assert_eq!(app.cursor, 2);
        assert!(!app.follow, "G kept following");

        app.apply(Msg::Progress { index: 1, percent: 40 });
        assert_eq!(app.cursor, 2, "the run pulled the cursor off the end");

        app.jump(false);
        assert_eq!(app.cursor, 0);
        app.apply(Msg::Progress { index: 3, percent: 40 });
        assert_eq!(app.cursor, 0, "the run pulled the cursor off the top");

        // f puts it back, which is the only way follow turns on again.
        app.follow = true;
        app.apply(Msg::Progress { index: 3, percent: 60 });
        assert_eq!(app.cursor, 2);
    }

    /* Editing the track you are on is the everyday way to filter it out from
       under yourself: `Update` carries the new name. Nothing in the key
       handlers runs then, so without a snap in `apply` the cursor stays on a
       row that is no longer drawn, no `▌` appears anywhere, and `e`, `c` and
       `space` all go quiet until the next `j`. */
    #[test]
    fn a_message_that_filters_the_cursors_track_out_moves_the_cursor() {
        let mut app = named(&["Aphex Twin - Xtal", "Boards - Olson", "Aphex Twin - Avril"]);
        app.filter = "aphex".into();
        app.cursor = 0;
        assert_eq!(app.selected(), Some(1));

        app.apply(Msg::Update {
            index: 1,
            status: Status::Manual,
            source: Some("typed".into()),
            note: None,
            name: Some("Edited - Fixed".into()),
        });

        assert_eq!(app.rows(), [2], "only the other match should be left");
        assert!(
            app.selected().is_some(),
            "the cursor is on a row that is not drawn, so every command is dead"
        );
        assert_eq!(app.cursor, 2);
        assert_eq!(app.row_of_cursor(&app.rows()), Some(0), "nothing is highlighted");
    }

    /* A status change does it too: `/have` stops matching the moment a track
       starts downloading. */
    #[test]
    fn a_status_change_under_a_status_filter_also_moves_the_cursor() {
        let mut app = named(&["One", "Two", "Three"]);
        app.filter = "ok".into();
        assert_eq!(app.rows(), [0, 2]);
        app.cursor = 0;

        // Track 2, the one the filter already hides, not the cursor's own.
        app.apply(Msg::Progress { index: 2, percent: 0 });
        assert_eq!(app.cursor, 0, "an unrelated track moved the cursor");

        app.apply(Msg::Progress { index: 2, percent: 50 });
        app.apply(Msg::Update {
            index: 2,
            status: Status::Ok,
            source: None,
            note: None,
            name: None,
        });
        assert_eq!(app.rows(), [0, 1, 2], "the track joined the filter");
        assert!(app.selected().is_some());
    }

    /* Narrowing the filter under the cursor must move it, or the highlighted
       row is off screen and `e` edits a track the user cannot see. */
    #[test]
    fn a_cursor_on_a_hidden_row_is_neither_shown_nor_selected() {
        let mut app = named(&["Aphex Twin - Xtal", "Boards of Canada - Roygbiv"]);
        app.cursor = 1;
        app.filter = "aphex".into();
        assert_eq!(app.selected(), None, "a hidden track was still selectable");

        app.snap();
        assert_eq!(app.cursor, 0);
        assert_eq!(app.selected(), Some(1));
    }

    /* `/` then `m` is how you mark one artist's bad rows. Reaching past the
       filter would mark the whole run instead, which is the opposite. */
    #[test]
    fn marking_by_status_stops_at_the_filter() {
        let mut app = named(&["Aphex Twin - Xtal", "Aphex Twin - Avril", "Boards - Olson"]);
        app.tracks[2].status = Status::Ok;
        app.filter = "aphex".into();
        app.cursor = 0;
        app.mark_like_cursor();
        assert_eq!(app.targets(), [1], "the hidden Ok track was marked too");
    }

    /* A mark is an explicit choice and a filter is a view, so marks made
       before one still count: the bar's "N marked" is what says so. */
    #[test]
    fn a_filter_does_not_drop_marks_made_before_it() {
        let mut app = named(&["Aphex Twin - Xtal", "Boards - Olson"]);
        app.cursor = 1;
        app.toggle_mark();
        app.filter = "aphex".into();
        app.snap();
        assert_eq!(app.targets(), [2]);
    }

    #[test]
    fn a_filter_matches_the_status_word_as_well_as_the_name() {
        let mut app = named(&["One", "Two", "Three"]);
        app.filter = "failed".into();
        assert_eq!(app.rows(), [1]);
        app.filter = "APHEX".into();
        assert!(app.rows().is_empty());
    }

    /* A filter left over from the last playlist hides most of the next one,
       which reads as a broken run rather than as a filter. */
    #[test]
    fn a_new_playlist_starts_unfiltered() {
        let mut app = named(&["One", "Two"]);
        app.filter = "one".into();
        app.typing_filter = true;
        app.apply(Msg::Restart);
        assert!(app.filter.is_empty());
        assert!(!app.typing_filter);
    }

    fn shelf(name: &str) -> Shelf {
        Shelf {
            path: PathBuf::from("/music").join(name),
            name: name.into(),
            url: "u".into(),
            tracks: 3,
            missing: 0,
            synced: None,
            files: Vec::new(),
        }
    }

    /* A sync sends the library again to update its counts, and doing that
       while the user is watching the run it belongs to would swap the screen
       out from under them. */
    #[test]
    fn refreshing_the_library_does_not_take_over_the_screen() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        app.apply(Msg::Library {
            shelves: vec![shelf("Focus")],
            show: true,
        });
        assert_eq!(app.view, View::Library);

        app.apply(Msg::Restart);
        assert_eq!(app.view, View::Tracks, "a starting run stayed hidden");
        app.apply(Msg::Library {
            shelves: vec![shelf("Focus"), shelf("Road trip")],
            show: false,
        });
        assert_eq!(app.view, View::Tracks);
        assert_eq!(app.library.len(), 2, "the new counts were dropped");
    }

    /* The cursor is an index into a list that a sync can shorten, and a stale
       one would draw a selection past the end of the rows. */
    #[test]
    fn the_library_cursor_stays_inside_a_list_that_shrank() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        app.apply(Msg::Library {
            shelves: vec![shelf("A"), shelf("B"), shelf("C")],
            show: true,
        });
        app.shelf = 2;
        app.apply(Msg::Library {
            shelves: vec![shelf("A")],
            show: false,
        });
        assert_eq!(app.shelf, 0);
    }

    #[test]
    fn escape_leaves_the_tracks_for_the_library_and_the_tool_without_one() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        app.leave_tracks();
        assert!(app.quit, "with no library there is nowhere to go back to");

        let mut app = App::new(std::sync::mpsc::channel().0, String::new());
        app.library = vec![shelf("Focus")];
        app.leave_tracks();
        assert_eq!(app.view, View::Library);
        assert!(!app.quit);
    }

    /* The two screens move different cursors and offer different keys, so
       neither set may be live while the other is showing. */
    #[test]
    fn each_screen_only_answers_for_its_own_keys() {
        let mut app = app_with(&[Status::Ok]);
        app.done = Some(Ok(String::new()));
        assert!(app.can_command());
        assert!(!app.can_browse(), "library keys were live over the tracks");

        app.apply(Msg::Library {
            shelves: vec![shelf("Focus")],
            show: true,
        });
        assert!(app.can_browse());
        assert!(!app.can_command(), "e would edit a track that is not shown");
    }

    #[test]
    fn a_finished_bulk_action_clears_the_selection() {
        let mut app = app_with(&[Status::Kept, Status::Kept]);
        app.mark_like_cursor();
        assert_eq!(app.targets().len(), 2);
        app.apply(Msg::Unmark);
        assert!(app.marked.is_empty());
    }

    /* `counts()` is a hand-kept array where `settled()` is an exhaustive
       match, so a new variant reaches the header's total without reaching the
       tally beside it, and the two disagree with nothing saying why. */
    #[test]
    fn every_settled_status_is_counted_in_the_tally() {
        let settled: Vec<Status> = [
            Status::Pending,
            Status::Have,
            Status::Downloading,
            Status::Downloaded,
            Status::Tagging,
            Status::Ok,
            Status::Manual,
            Status::Kept,
            Status::Weak,
            Status::NoMatch,
            Status::Failed,
            Status::Gone,
            Status::Skipped,
        ]
        .into_iter()
        .filter(|s| s.settled())
        .collect();

        let app = app_with(&settled);
        let counted: Vec<Status> = app.counts().into_iter().map(|(s, _)| s).collect();
        for status in &settled {
            assert!(counted.contains(status), "{status:?} is missing from the tally");
        }
        assert_eq!(
            app.counts().iter().map(|(_, n)| n).sum::<usize>(),
            app.settled(),
            "the tally and the header total disagree"
        );
    }

    /* A terminal status left out of `settled()` means the run never reaches
       its own count and the UI waits forever. The match below has no
       wildcard, so a new variant fails to compile here first. */
    #[test]
    fn every_status_is_either_in_flight_or_settled() {
        let all = [
            Status::Pending,
            Status::Have,
            Status::Downloading,
            Status::Downloaded,
            Status::Tagging,
            Status::Ok,
            Status::Manual,
            Status::Kept,
            Status::Weak,
            Status::NoMatch,
            Status::Failed,
            Status::Gone,
            Status::Skipped,
        ];
        for status in all {
            let in_flight = match status {
                Status::Pending | Status::Downloading | Status::Downloaded | Status::Tagging => {
                    true
                }
                Status::Have
                | Status::Ok
                | Status::Manual
                | Status::Kept
                | Status::Weak
                | Status::NoMatch
                | Status::Failed
                | Status::Gone
                | Status::Skipped => false,
            };
            assert_eq!(status.settled(), !in_flight, "{status:?}");
        }
    }

    fn picking_app(statuses: &[Status]) -> (App, std::sync::mpsc::Receiver<Reply>) {
        let mut app = app_with(statuses);
        let (tx, rx) = std::sync::mpsc::channel();
        app.apply(Msg::Pick(tx));
        (app, rx)
    }

    /* Enter has to mean the whole playlist, so the gate opens with everything
       that needs fetching already marked and the work is unmarking. Starting
       empty makes the accidental answer a silent no-op. */
    #[test]
    fn the_pick_opens_with_everything_that_needs_downloading() {
        let (app, _rx) = picking_app(&[Status::Pending, Status::Have, Status::Gone, Status::Pending]);
        let mut marked: Vec<usize> = app.marked.iter().copied().collect();
        marked.sort_unstable();
        assert_eq!(marked, [1, 4], "on-disk and departed tracks were selected");
        assert!(!app.follow, "following would drag the cursor off the choice");
    }

    /* `targets()` falls back to the cursor when nothing is marked, which here
       would turn "I want none of these" into "download the row I am on". */
    #[test]
    fn unmarking_everything_answers_with_nothing() {
        let (mut app, rx) = picking_app(&[Status::Pending, Status::Pending]);
        app.toggle_all();
        assert!(app.marked.is_empty());
        app.confirm_pick();
        assert!(matches!(rx.recv().unwrap(), Reply::Picked(p) if p.is_empty()));
        assert!(app.picking.is_none(), "the question stayed open");
    }

    /* Esc before any download still leaves a folder worth tagging, so it
       answers rather than quitting. */
    #[test]
    fn cancelling_the_pick_downloads_nothing_and_carries_on() {
        let (mut app, rx) = picking_app(&[Status::Pending]);
        app.cancel_pick();
        assert!(matches!(rx.recv().unwrap(), Reply::Picked(p) if p.is_empty()));
        assert!(app.picking.is_none());
    }

    #[test]
    fn confirming_answers_with_the_marks_in_order() {
        let (mut app, rx) = picking_app(&[Status::Pending, Status::Pending, Status::Pending]);
        app.marked.remove(&2);
        app.confirm_pick();
        assert!(matches!(rx.recv().unwrap(), Reply::Picked(p) if p == [1, 3]));
    }

    /* `a` reaching past the filter is the same bug `m` already guards, and
       under a pick it selects a whole playlist instead of one artist. */
    #[test]
    fn marking_everything_stops_at_the_filter() {
        let mut app = app_with(&[Status::Ok, Status::Failed, Status::Ok]);
        app.tracks[0].name = "Aphex Twin - Xtal".into();
        app.tracks[1].name = "Boards - Olson".into();
        app.tracks[2].name = "Aphex Twin - Avril".into();
        app.marked.clear();
        app.filter = "aphex".into();

        app.toggle_all();
        let mut marked: Vec<usize> = app.marked.iter().copied().collect();
        marked.sort_unstable();
        assert_eq!(marked, [1, 3], "the filtered-out track was marked");

        // The same key takes the shown rows back off, leaving the rest alone.
        app.toggle_all();
        assert!(app.marked.is_empty());
    }
}
