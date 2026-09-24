use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::sync::Arc;
use std::sync::mpsc::Sender;

use ratatui::style::Color;

use crate::cheats::{self, Cheat, Unlocked};
use crate::theme::{Palette, Themes};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Pending,
    Have,
    Downloading,
    Downloaded,
    Tagging,
    /// Being re-encoded into the current format by a sync, with the old file
    /// still on disk until the new one is written and verified.
    Converting,
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
    /// On disk and not in the playlist, and never was: a file the user put in
    /// the folder themselves, that a sync could not match to any video.
    /* Not `Gone`, which says a video left the playlist and, once proven,
       lets `D` delete the file. Neither half is true here: nothing left, and
       the file is the one thing in the folder earworm has no claim on at all.
       A separate status is also what keeps it out of `tag_tracks`, where a
       lookup would retag a file the playlist knows nothing about. */
    Local,
}

/* Every variant, for the tests that have to walk them all. Rust will not
   enumerate them and this list is hand-kept, so a new status left out of it
   costs coverage rather than correctness: nothing in the running tool reads
   it. `counts()` used to, which made forgetting one drop a whole column from
   the tally with nothing saying why, and it now builds itself from the
   tracks instead. */
#[cfg(test)]
pub const TALLY: [Status; 15] = [
    Status::Pending,
    Status::Have,
    Status::Downloading,
    Status::Downloaded,
    Status::Tagging,
    Status::Converting,
    Status::Ok,
    Status::Manual,
    Status::Kept,
    Status::Weak,
    Status::NoMatch,
    Status::Failed,
    Status::Gone,
    Status::Skipped,
    Status::Local,
];

impl Status {
    /// No wider than `STATUS_WIDTH`, and readable without the README: the old
    /// words (`have`, `got`, `kept`, `none`) were short enough to fit and
    /// private enough that only their colour said anything.
    /* One verb per activity, shared with the stage word in the header: the
       header said `downloading` while the row said `getting`, and `reading`
       on a row was tag reading while `reading playlist` in the header was the
       scan. Two words for one thing reads as two things happening. */
    pub fn label(self) -> &'static str {
        match self {
            Status::Pending => "queued",
            Status::Have => "on disk",
            Status::Downloading => "fetching",
            Status::Downloaded => "fetched",
            Status::Tagging => "tagging",
            // `converting` is 10 and STATUS_WIDTH is 8.
            Status::Converting => "convert",
            Status::Ok => "ok",
            Status::Manual => "manual",
            Status::Kept => "guessed",
            Status::Weak => "unsure",
            Status::NoMatch => "no match",
            Status::Failed => "failed",
            Status::Gone => "gone",
            Status::Skipped => "skipped",
            Status::Local => "local",
        }
    }

    /* Colour is confidence and the word is provenance, so `ok` and `manual`
       share one: both are tags somebody or something confirmed, and which of
       the two is the label's job. Anything unverified is sand or amber, so a
       guess never wears the colour of a settled track. */
    pub fn color(self, p: Palette) -> Color {
        match self {
            Status::Ok | Status::Manual => p.cursor,
            Status::Weak => p.warn,
            // Unverified, where `kept` used to be dim beside four states with
            // nothing to check at all.
            Status::Kept | Status::NoMatch => p.unsure,
            Status::Failed => p.error,
            Status::Downloading
            | Status::Tagging
            | Status::Downloaded
            | Status::Converting => p.accent,
            // Nothing to act on: not downloaded, or deliberately left alone.
            /* Nothing to act on: not downloaded, deliberately left alone, or
               the user's own file, which is settled by definition. */
            Status::Pending
            | Status::Have
            | Status::Gone
            | Status::Skipped
            | Status::Local => p.muted,
        }
    }

    /* Nothing has confirmed these: three are guesses at a tag and the
       fourth has no file. What `n` jumps between, what the review filter
       keeps, and what the finish menu counts when it offers to help. */
    pub fn wants_a_look(self) -> bool {
        matches!(
            self,
            Status::Kept | Status::Weak | Status::NoMatch | Status::Failed
        )
    }

    /* Where a settled status sits in the tally beside the header, or `None`
       for one that is still in flight. Exhaustive, unlike the hand-kept array
       this replaced: a settled status left out of that array was counted in
       the header's total and missing from the tally next to it, with nothing
       on screen saying why, and adding a variant did not fail to compile.
       Now it does. */
    pub fn tally(self) -> Option<u8> {
        match self {
            Status::Ok => Some(0),
            Status::Manual => Some(1),
            Status::Kept => Some(2),
            Status::Weak => Some(3),
            Status::NoMatch => Some(4),
            Status::Failed => Some(5),
            Status::Gone => Some(6),
            Status::Skipped => Some(7),
            Status::Have => Some(8),
            Status::Local => Some(9),
            Status::Pending
            | Status::Downloading
            | Status::Downloaded
            | Status::Tagging
            | Status::Converting => None,
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
                | Status::Local
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
    /// Written by the lookup or typed into the edit form. Not on the row,
    /// which has no column to spare, but the form prefills from it and the
    /// detail pane shows it.
    pub album: String,
    pub year: Option<u16>,
    pub duration: u64,
    pub mbid: Option<String>,
    /// Carries a playlist entry, so an edit can rewrite the .m3u8 in order.
    pub listed: bool,
    /* Set only by `note_departures`, which has just seen a complete,
       unnarrowed listing without this video in it. `mark_departed` reads the
       same fact off the last playlist file instead and leaves this false: a
       narrowed run writes a short `.m3u8`, so an offline read can call half a
       folder departed, and deleting on that would take music. Nothing else
       distinguishes the two `Gone`s, and this is what `D` is allowed to act
       on. */
    pub departure_proven: bool,
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
            album: String::new(),
            year: None,
            artist: String::new(),
            title: String::new(),
            duration: 0,
            mbid: None,
            listed: false,
            departure_proven: false,
        }
    }
}

/// One playlist folder under `--dir`: either one earworm downloaded, read from
/// its manifest alone so the library opens with no network and no yt-dlp, or
/// one somebody put there, read from the audio sitting in it.
/* `Default` is for the tests, which build a row to ask one question of it and
   would otherwise restate every field each time a new one is added. Nothing
   in the running tool builds a shelf that way: `library` fills all of them. */
#[derive(Clone, Default)]
pub struct Shelf {
    pub path: PathBuf,
    pub name: String,
    /// `None` for a folder earworm did not download: one somebody copied in,
    /// or one adopted by an edit and never attached to a playlist. Every key
    /// that means "the playlist upstream" has to answer for that case rather
    /// than assume a string is there.
    pub url: Option<String>,
    pub tracks: usize,
    /// Entries whose file has since been deleted. Not the same as a track that
    /// left the playlist, which needs a listing to know about.
    pub missing: usize,
    pub synced: Option<u64>,
    /// Filename and whether it is still on disk, in playlist order. Resolved
    /// here because the draw loop must not stat, and the filename carries the
    /// number and tags already, so the preview needs no tag read.
    pub files: Vec<(String, bool)>,
    /// The folder's `.m3u8`, whatever it is called. earworm names its own
    /// after the folder, but a folder somebody copied in brought its own
    /// playlist file with its own name, and `p` is what reads this: handing
    /// cliamp a path earworm invented would refuse a folder that has one.
    /// `None` when the folder has no playlist file, or more than one and so
    /// no single answer.
    pub playlist: Option<PathBuf>,
    /// What `manifest::kind_of` makes of the folder, from its name or from
    /// what a sync recorded. `None` is the answer for almost every folder and
    /// means playlist: the row says nothing, because playlist is the default
    /// rather than a finding.
    pub kind: Option<crate::manifest::Kind>,
}

/// What earworm knows about cliamp. `Missing` is also the starting guess, so
/// nothing about the player is claimed before it has been looked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Player {
    #[default]
    Missing,
    Stopped,
    Running,
}

/// What cliamp is doing, as one value so the poll behind it emits one message.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Playback {
    pub player: Player,
    /// Folder of the track cliamp has loaded, when it is a local file. cliamp
    /// reports the track and never the playlist, so this is what says which
    /// library row is the one on air; a radio stream has no folder.
    pub folder: Option<PathBuf>,
    /// What cliamp calls the loaded track. The folder says which playlist is
    /// on air and never which song, which is the question people actually
    /// have. `None` for a stream, or a cliamp that reports no title.
    pub track: Option<String>,
    pub playing: bool,
}

impl Playback {
    /* Compared composed, because this path came back out of cliamp and the
       shelf's came off disk. Which form cliamp emits is its business and can
       change; a Korean folder name silently never matching is not a failure
       anything on screen would explain. */
    /// Whether this row is the one cliamp has loaded.
    pub fn on(&self, folder: &Path) -> bool {
        let same = |path: &Path| crate::config::composed(&path.to_string_lossy());
        self.folder.as_deref().map(same) == Some(same(folder))
    }
}

impl Player {
    /// Whether a playlist can be handed over right now.
    pub fn ready(self) -> bool {
        self == Player::Running
    }
}

/// Remembers what was last reported, so the three-second poll behind it costs
/// one message per change rather than one per tick.
#[derive(Default)]
pub struct Watch(Option<Playback>);

impl Watch {
    pub fn poll(&mut self, now: Playback) -> Option<Playback> {
        (self.0.replace(now.clone()) != Some(now.clone())).then_some(now)
    }
}

/// Which screen has the keys. The library is a screen and not a prompt because
/// its rows stay open: opening one and pressing Esc comes back to it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Tracks,
    Library,
    /// Every track in the library whose filename matches the filter, flat,
    /// with the folder each one sits in. The library screen narrows to the
    /// folders holding a match; this says which tracks they are.
    Found,
}

/// Work the UI asks of the worker once the run itself has finished.
pub enum Cmd {
    Edit(usize),
    Cover(usize),
    /// Run the text search again for one track's current artist and title.
    /// Carries the track index, like `Edit`: the row is still there, and a
    /// cursor position would name whatever moved under it by answer time.
    Search(usize),
    /// Load a library row from its manifest, with no network behind it. The
    /// folder and not the row: the worker re-reads the library, and a row
    /// position would then name whatever had moved into it. The filename is
    /// the track to land on, for a folder opened from a search result: only
    /// the worker knows what index that file ends up with, since `open_shelf`
    /// numbers from the filename and renumbers around departures.
    Open(PathBuf, Option<String>),
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
    /// Hand a folder's .m3u8 to cliamp. Carries the folder rather than meaning
    /// "the open one", so the library can play a row without opening it.
    Play(PathBuf),
    /// Play/pause whatever cliamp has loaded, which need not be one of ours.
    Toggle,
    /// Choose the audio format new downloads arrive in, and remember it.
    Format,
    /// Put the last tag write back. One level: the point is the edit you
    /// have just seen land on the row, not a history to walk.
    Undo,
    /// Rename a playlist's folder. Carries the folder rather than the row,
    /// for the same reason `Open` does.
    Rename(PathBuf),
    /// Delete the files of every track a complete listing said had left the
    /// playlist. No indices: it is the whole set or nothing, and the worker
    /// re-checks each one rather than trusting the screen.
    Purge,
    /// Hand a folder to the platform's file manager. Everything downstream of
    /// earworm happens there or in a player, and the alternative is retyping
    /// a path the screen is already showing.
    Reveal(PathBuf),
    /// Remove a library folder from the disk: the worker confirms first,
    /// then either forgets the playlist or deletes the whole folder.
    /// Carries the folder rather than the row, like `Open`.
    Remove(PathBuf),
}

/* A question the UI asks on its own account. Nothing is blocked on the
   answer, unlike `Prompt`, where the worker is sitting on a channel: the run
   carries on behind this and the only thing waiting is the keypress. */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Confirm {
    /// Quitting with a download still going, which takes yt-dlp with it.
    Quit,
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
    /// Several labelled boxes answered together. Artist and title were two
    /// prompts in a row, which made the commonest edit in the tool two
    /// questions, two Enters, and no way to see one while typing the other.
    Form {
        header: String,
        /// The video title the tags came from, shown above the boxes: it is
        /// what the answer is being corrected against.
        note: String,
        fields: Vec<(String, String)>,
        escape: Escape,
    },
    Input {
        header: String,
        /// A dim line above the box. The first question a new user is ever
        /// asked has room to say what an answer looks like, and nowhere else
        /// on that screen has anything on it yet.
        note: String,
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
    /// Every box of a form, in the order they were asked for.
    Fields(Vec<String>),
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
    /// The folder the run is writing into, once one is known.
    Folder(PathBuf),
    /// Sent only when it changes, since the worker re-checks on a timer.
    Player(Playback),
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
    /* Its own message rather than two more fields on `Update`: album and
       year change only when somebody edits them, where `Update` is sent for
       every status the run passes through. Same reason `Path` is separate. */
    Meta {
        index: usize,
        album: String,
        year: Option<u16>,
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
    /// The run's settings line for the help overlay, re-sent when one of them
    /// is changed from inside the tool rather than by a flag.
    Settings {
        text: String,
        /// Separately from the description, because the library preview
        /// counts files against it and parsing it back out of the prose
        /// would be a second place the two could disagree.
        format: String,
    },
    /// What `u` would put back, or `None` when there is nothing to undo. The
    /// worker holds the old values, so the UI cannot work this out for
    /// itself, and a key it cannot say is live is a key nobody presses.
    Undoable(Option<String>),
    /// Rows to take off the list, by track index, after their files have gone.
    /// Not `Tracks`, which restarts the estimate's clock and means "the scan
    /// is over"; these rows simply stop existing.
    Dropped(Vec<usize>),
    /// Narrow the list to the tracks nothing has confirmed and put the cursor
    /// on the first. The finish menu's answer to "what now" when the run left
    /// guesses behind.
    Review,
    /// Put the cursor on one track, for a folder opened at a search result.
    /// Following would drag it off again at the next progress message, so it
    /// goes off here rather than at the keypress that has no run behind it.
    Focus(usize),
    /// The art inside the files has changed while their paths have not. The
    /// detail pane keys its decoded cover on the path, so nothing else on
    /// this list would tell it the picture it is holding is out of date.
    Artwork,
    /// Nothing to report and nothing to look at, so close the UI outright.
    Quit,
}

/// Prompts are answered by the UI thread, so a question blocks only the worker.
/// Long enough to read a filename in, short enough not to outlive interest.
/* Four seconds fitted "saved track 12" and not a wrapped cliamp error, so it
   is a floor plus reading time rather than one number for every message. */
const FLASH: Duration = Duration::from_secs(2);
const PER_CHAR: Duration = Duration::from_millis(50);
const FLASH_MAX: Duration = Duration::from_secs(8);
/// Deep enough for a bulk action's outcomes, shallow enough that a run cannot
/// build a backlog nobody is still reading by the time it drains.
const QUEUE: usize = 3;

/// The log pane at rest: enough for one yt-dlp entry and its warning.
pub const LOG_ROWS: usize = 8;

/// Long enough to read the word, short enough that nobody reaches for a key.
pub const INTRO: Duration = Duration::from_millis(1900);
/// How long the reward popup stays up if no key dismisses it.
pub const REWARD: Duration = Duration::from_millis(3500);
/// Long enough for any code, short enough that the box never has to scroll.
pub const CODE_MAX: usize = 24;

/* The `^g` box. Its own text rather than a box in `fields`, because it opens
   over a prompt whose answer has to be there when it closes. */
#[derive(Debug, Default)]
pub struct Console {
    pub text: String,
    /// The answer to the last code, cleared by the next keystroke.
    pub said: Option<String>,
}

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

    /* One question for a whole edit. Artist and title were two prompts in a
       row, so correcting one track meant two Enters, and the second question
       covered the answer to the first just when you wanted to compare them. */
    pub fn form(
        &self,
        header: &str,
        note: &str,
        fields: Vec<(String, String)>,
        escape: Escape,
    ) -> Option<Vec<String>> {
        if !self.enabled {
            return None;
        }
        match self.ask(Prompt::Form {
            header: header.into(),
            note: note.into(),
            fields,
            escape,
        }) {
            Reply::Fields(values) => Some(values),
            _ => None,
        }
    }

    /// None means the user cancelled. An empty string is a real answer the
    /// caller has to judge, so clearing a field cannot look like an escape.
    pub fn input(&self, header: &str, value: &str) -> Option<String> {
        self.prompt(header, "", value, Escape::Skip)
    }

    /* With a line above the box saying what an answer looks like, and for a
       question whose cancel means something other than "leave this track
       alone": ending the tool, or going back to the screen behind it. */
    pub fn input_noted(
        &self,
        header: &str,
        note: &str,
        value: &str,
        escape: Escape,
    ) -> Option<String> {
        self.prompt(header, note, value, escape)
    }

    fn prompt(&self, header: &str, note: &str, value: &str, escape: Escape) -> Option<String> {
        if !self.enabled {
            return None;
        }
        match self.ask(Prompt::Input {
            header: header.into(),
            note: note.into(),
            value: value.into(),
            escape,
        }) {
            Reply::Text(t) => Some(t.trim().to_string()),
            _ => None,
        }
    }
}

/* yt-dlp labels its own lines, and a clean run still prints warnings about
   individual videos. Painting all of it red made every run look broken and
   left a real ERROR indistinguishable from the noise around it. */
pub fn log_color(line: &str, p: Palette) -> Color {
    if line.contains("ERROR") {
        p.error
    } else if line.contains("WARNING") {
        p.warn
    } else {
        p.muted
    }
}

fn matched(track: &Track, needle: &str) -> bool {
    track.name.to_lowercase().contains(needle) || track.status.label().contains(needle)
}

/* Rows of context kept either side of the cursor. `ListState` is built
   fresh every frame, so with no offset of our own ratatui scrolls the
   least it can to make the selection visible: past the first screenful
   the cursor sits on the last row for the rest of the list, and you move
   down through a list that never shows you what is coming. */
pub const SCROLLOFF: usize = 2;

/// Where the list starts drawing from, given where it started last frame.
/// Only moves when the cursor comes within `SCROLLOFF` of an edge, so the
/// rows stay put while the cursor crosses the middle of the pane.
pub fn scroll_to(offset: usize, at: usize, len: usize, height: usize) -> usize {
    if height == 0 || len <= height {
        return 0;
    }
    // Halved on a short pane, where the cursor cannot clear both edges.
    let pad = SCROLLOFF.min((height - 1) / 2);
    let last = len - height;
    let highest = at.saturating_sub(pad).min(last);
    let lowest = (at + pad + 1).saturating_sub(height).min(last);
    offset.clamp(lowest, highest)
}

/* How many of a folder's files are not written in `format`. Off the
   filenames the shelf already holds, so it costs no disk and no worker round
   trip, which is what lets the preview ask it per frame.

   `alac` and `m4a` share an extension, so a folder of one counted against the
   other reads as nothing to do. That is the honest answer from a filename:
   telling the two apart means opening every file to read its codec, which is
   what the sync does when it actually converts them. */
pub fn off_format(shelf: &Shelf, format: &str) -> usize {
    let want = crate::config::extension(format);
    shelf
        .files
        .iter()
        .filter(|(_, here)| *here)
        .filter(|(name, _)| {
            std::path::Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .is_none_or(|e| !e.eq_ignore_ascii_case(want))
        })
        .count()
}

/* A folder is on the list for its own name or for a track inside it. The
   names come free, but the filenames are the only way to answer "which
   playlist has that track" without opening folders one at a time, and
   `Shelf.files` is already in memory from the stat pass `library` does.
   Short-circuited on the name, and the filenames are lowercased per call
   rather than kept twice: this runs over every folder on every frame, and a
   second copy of every filename in the library is the costlier of the two. */
pub fn shelf_matches(shelf: &Shelf, needle: &str) -> bool {
    shelf.name.to_lowercase().contains(needle)
        || shelf
            .files
            .iter()
            .any(|(name, _)| name.to_lowercase().contains(needle))
}

/* The two boxes `^s` swaps, by label rather than by position: the edit form
   grew album and year after these, and swapping 0 and 1 would only have gone
   on working by accident. Shared with the draw, so the key and the hint that
   advertises it cannot disagree about whether this form has them. */
pub fn swappable(fields: &[(String, String)]) -> Option<(usize, usize)> {
    let at = |want: &str| fields.iter().position(|(label, _)| label == want);
    Some((at("artist")?, at("title")?))
}

/// Library order. Name is what the worker hands over, so it is the one that
/// costs nothing; the other two answer "which of these needs a sync".
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Sort {
    #[default]
    Name,
    Synced,
    Missing,
}

impl Sort {
    pub fn next(self) -> Sort {
        match self {
            Sort::Name => Sort::Synced,
            Sort::Synced => Sort::Missing,
            Sort::Missing => Sort::Name,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Sort::Name => "name",
            Sort::Synced => "last synced",
            Sort::Missing => "most missing",
        }
    }
}

pub struct App {
    pub cmds: Option<Sender<Cmd>>,
    pub settings: String,
    /// What every draw reads its colours from. Lives here rather than in a
    /// global because `^t` changes it between frames, and a `OnceLock` is the
    /// one shape that cannot.
    pub theme: Palette,
    /* What it is called. Tracked beside the colours rather than looked up
       from them, because a `[colors]` table makes the palette match no entry
       in the registry: the name is the base the overrides were painted on,
       which is both what `^t` walks from and what gets written back. */
    pub theme_name: String,
    /// Every palette `^t` can reach: the built-ins plus any `[themes.*]`.
    pub themes: Themes,
    /* How this terminal draws pictures, answered once at startup by the
       protocol's own handshake. `None` when the query found nothing worth
       using, which is also what switches the pane's cover off entirely. */
    pub picker: Option<Picker>,
    /* SPIKE: one cover, encoded on the UI thread the first frame the cursor
       rests on a new track. That is a ~60ms hitch in the render loop and the
       architecture forbids it; it is here only so the picture can be looked at
       before the thread and the cache that belong around it are rebuilt.
       Keyed on the area as well as the file, because the encode is for a
       given number of cells and the pane changes size with the terminal. */
    pub art: Option<(PathBuf, ratatui::layout::Rect, Protocol)>,
    /* Where a cover would go, published by the draw because only the layout
       knows: the same bargain `viewport` makes with `page`. `None` when the
       pane has no room for one. */
    pub art_area: Option<ratatui::layout::Rect>,
    /// Where `^t` writes the theme it lands on. Read off the config in `main`
    /// before the worker takes it, like `format`; `None` under --no-config,
    /// which asked for the file to stay out of the run.
    pub config_file: Option<PathBuf>,
    /// Whether a `[colors]` table is repainting whatever `^t` lands on, so
    /// the flash can say the name is not the whole story.
    pub theme_overridden: bool,
    /// What new downloads are written as, for the preview's count of files
    /// that are something else. Set at startup and by `Msg::Settings`.
    pub format: String,
    pub tick: usize,
    pub show_help: bool,
    pub stage: String,
    /// What the header goes back to once a flash expires.
    resting: String,
    flash_until: Option<Instant>,
    pub playlist: String,
    /// Where the open run wrote, for handing to an external player. Set by
    /// `Msg::Folder`, since the UI only ever sees a track's path by chance.
    pub folder: Option<PathBuf>,
    pub playback: Playback,
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
    /// Narrows the track list to what nothing has confirmed. A second
    /// predicate rather than a filter string, because the four statuses it
    /// keeps share no word to match on.
    pub review: bool,
    /// Narrows the library to one kind. The library's own predicate, as
    /// `review` is the track list's: neither word is in a folder's name, and
    /// a folder with no answer is a playlist, which no query can say.
    pub only: Option<crate::manifest::Kind>,
    /// Which order the library is in. Not applied to `library` itself, which
    /// stays as the worker built it: `shelf` is a position in that, so a
    /// re-sorted vector would move the row out from under the cursor.
    pub sort: Sort,
    /// Track indices, not row positions, so a bulk action means the same
    /// thing to the worker as it does on screen.
    pub marked: HashSet<usize>,
    /// A command is in flight. Set when one is sent rather than when the
    /// worker reports starting it, because the gap between the two is exactly
    /// long enough for a second keypress to queue a duplicate.
    pub busy: bool,
    pub follow: bool,
    pub show_logs: bool,
    /// Half the body rather than eight rows. The line worth reading is often
    /// well above the tail, and eight rows of yt-dlp is two entries.
    pub logs_tall: bool,
    /// Lines held back from the bottom of the log. Zero is the tail, which is
    /// where a running job's next line lands.
    pub log_scroll: usize,
    /// What `u` would put back, as the worker described it. `None` when there
    /// is nothing, which is what keeps the key off the bar.
    pub undoable: Option<String>,
    pub prompt: Option<(Prompt, Sender<Reply>)>,
    pub choice: usize,
    /// One box per field, so a form and a single question are the same state
    /// and every editing key works the same on both.
    pub fields: Vec<String>,
    /// Which box has the keys.
    pub field: usize,
    /* Which result the search screen is on, as the pair that names it rather
       than a row number: `found_rows` is recomputed from the filter like
       every other view, and a sync can rebuild the library underneath it. */
    pub found: Option<(usize, usize)>,
    /// Where each list last started drawing from. Held across frames so the
    /// rows only move when the cursor approaches an edge.
    pub scroll: usize,
    pub shelf_scroll: usize,
    pub found_scroll: usize,
    /// Messages that arrived while one was still being read.
    waiting: std::collections::VecDeque<String>,
    /// A question the UI raised itself, waiting on y or n.
    pub confirm: Option<Confirm>,
    /// When this run's track list arrived, which is the only clock the
    /// estimate can use: `started` includes the intro and the URL prompt.
    pub run_started: Option<Instant>,
    /// Where the next typed character lands, as a char index into the box.
    pub caret: usize,
    /// Rows the list pane last drew with, set by the draw that knows. A page
    /// key has to move by what is on screen, and only the layout knows that.
    pub viewport: usize,
    pub done: Option<Result<String, String>>,
    /// The worker is blocked waiting for the tracks to download. Holding the
    /// reply channel here is what makes the keys mean "choose" for as long as
    /// the question is open, and dropping it answers `Cancel`.
    pub picking: Option<Sender<Reply>>,
    /// Wall clock, not `tick`: the intro has to run at the same speed on a
    /// terminal that redraws on every message and one that sits idle.
    pub started: Instant,
    pub intro_done: bool,
    /// A newer release, as the probe thread reported it. `None` until it
    /// lands, and forever when the probe was off or found nothing.
    pub update: Option<String>,
    /// Shared with the worker, which asks it at the URL prompt.
    pub unlocked: Arc<Unlocked>,
    pub console: Option<Console>,
    /// The code just found and when, for the popup that celebrates it.
    pub reward: Option<(Instant, &'static Cheat)>,
    pub quit: bool,
}

impl App {
    pub fn new(cmds: Sender<Cmd>, settings: String) -> Self {
        App {
            cmds: Some(cmds),
            settings,
            theme: Palette::default(),
            theme_name: crate::config::DEFAULT_THEME.to_string(),
            themes: Themes::default(),
            picker: None,
            art: None,
            art_area: None,
            config_file: None,
            theme_overridden: false,
            format: crate::config::DEFAULT_FORMAT.to_string(),
            tick: 0,
            show_help: false,
            stage: "starting".into(),
            resting: "starting".into(),
            flash_until: None,
            playlist: String::new(),
            folder: None,
            playback: Playback::default(),
            tracks: Vec::new(),
            logs: Vec::new(),
            view: View::Tracks,
            library: Vec::new(),
            shelf: 0,
            cursor: 0,
            filter: String::new(),
            typing_filter: false,
            review: false,
            only: None,
            sort: Sort::default(),
            marked: HashSet::new(),
            busy: false,
            follow: true,
            show_logs: false,
            logs_tall: false,
            log_scroll: 0,
            undoable: None,
            prompt: None,
            choice: 0,
            fields: Vec::new(),
            field: 0,
            found: None,
            scroll: 0,
            shelf_scroll: 0,
            found_scroll: 0,
            waiting: std::collections::VecDeque::new(),
            confirm: None,
            run_started: None,
            caret: 0,
            viewport: 1,
            done: None,
            picking: None,
            started: Instant::now(),
            intro_done: false,
            update: None,
            unlocked: Arc::default(),
            console: None,
            reward: None,
            quit: false,
        }
    }

    pub fn celebrating(&self) -> Option<&'static Cheat> {
        self.reward
            .filter(|(at, _)| at.elapsed() < REWARD)
            .map(|(_, cheat)| cheat)
    }

    pub fn try_code(&mut self) {
        let Some(console) = &mut self.console else {
            return;
        };
        let said = match cheats::find(&console.text) {
            Some(cheat) if self.unlocked.grant(cheat.feature) => {
                self.console = None;
                self.reward = Some((Instant::now(), cheat));
                /* The worker only re-heads the prompt on the next answer, so
                   until then it would contradict the celebration. */
                if let Some((Prompt::Input { header, .. }, _)) = &mut self.prompt
                    && header == cheats::LOCKED
                {
                    *header = cheats::OPENED.into();
                }
                return;
            }
            Some(cheat) => format!("already found  ·  {} are open", cheat.prize),
            None => "nothing happens".into(),
        };
        console.text.clear();
        console.said = Some(said);
    }

    /* Holds its full window rather than yielding to the library, which on a
       bare start is ready in milliseconds and would leave the intro unseen.
       Nothing is waiting on the user behind a list. A prompt is the exception:
       the worker is blocked on an answer, and hiding the question is not
       polish. Everything else here is content worth more than an animation. */
    pub fn intro(&self) -> bool {
        !self.intro_done
            && self.tracks.is_empty()
            && self.prompt.is_none()
            && self.done.is_none()
            && !self.show_help
            && self.started.elapsed() < INTRO
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
                self.waiting.clear();
                self.stage = s;
            }
            Msg::Flash(s) => self.say(s),
            Msg::Playlist(p) => self.playlist = p,
            Msg::Folder(f) => self.folder = Some(f),
            Msg::Player(now) => self.playback = now,
            Msg::Tracks(t) => {
                // The scan is over and the work starts here, so this is when
                // the clock behind the estimate starts.
                self.run_started = Some(Instant::now());
                self.tracks = t;
            }
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
            Msg::Meta { index, album, year } => {
                if let Some(t) = self.track_mut(index) {
                    t.album = album;
                    t.year = year;
                }
            }
            Msg::Artwork => self.art = None,
            Msg::Path { index, path } => {
                if let Some(t) = self.track_mut(index) {
                    t.path = Some(path);
                }
            }
            Msg::Log(line) => {
                /* Scrolled back means reading something, and a line arriving
                   at the bottom must not shift it. The offset counts from the
                   tail, so holding the view still means growing it. */
                if self.log_scroll > 0 {
                    self.log_scroll += 1;
                }
                self.logs.push(line);
            }
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
                    /* Nothing the pick gate could ask about: already on
                       disk, departed, or a file the playlist never had. */
                    .filter(|t| !matches!(t.status, Status::Have | Status::Gone | Status::Local))
                    .map(|t| t.index)
                    .collect();
                self.follow = false;
                self.picking = Some(reply);
            }
            Msg::Ask(prompt, reply) => {
                match &prompt {
                    Prompt::Input { value, .. } => self.open_fields(vec![value.clone()]),
                    Prompt::Form { fields, .. } => {
                        self.open_fields(fields.iter().map(|(_, v)| v.clone()).collect());
                    }
                    Prompt::Choice { .. } => {}
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
            Msg::Settings { text, format } => {
                self.settings = text;
                self.format = format;
            }
            Msg::Undoable(what) => self.undoable = what,
            /* Not a jump on its own: the cursor lands on the first of them and
               the rest stay on screen, so the next one is `n` away rather
               than a search through the whole run. */
            Msg::Review => {
                self.review = self.tracks.iter().any(|t| t.status.wants_a_look());
                self.follow = false;
                if let Some(pos) = self.tracks.iter().position(|t| t.status.wants_a_look()) {
                    self.cursor = pos;
                }
            }
            Msg::Focus(index) => {
                if let Some(pos) = self.tracks.iter().position(|t| t.index == index) {
                    self.cursor = pos;
                    self.follow = false;
                }
            }
            Msg::Unmark => self.marked.clear(),
            Msg::Dropped(gone) => {
                self.tracks.retain(|t| !gone.contains(&t.index));
                for index in &gone {
                    self.marked.remove(index);
                }
                /* The cursor is a position in `tracks`, and the rows it sat
                   on or after have moved up. Clamped rather than snapped: the
                   filter's snap only moves a cursor that is on a hidden row,
                   and one past the end is not hidden, it is nowhere. */
                if !self.tracks.is_empty() {
                    self.cursor = self.cursor.min(self.tracks.len() - 1);
                }
            }
            Msg::Idle => self.busy = false,
            Msg::Restart => {
                self.done = None;
                self.run_started = None;
                self.tracks.clear();
                self.marked.clear();
                self.playlist.clear();
                self.folder = None;
                self.cursor = 0;
                self.follow = true;
                /* A filter from the last playlist would hide most of the next
                   one, which reads as a broken run rather than a filter. */
                self.filter.clear();
                self.typing_filter = false;
                self.review = false;
                self.undoable = None;
                self.log_scroll = 0;
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
        if !self.flash_until.is_some_and(|at| Instant::now() >= at) {
            return;
        }
        self.flash_until = None;
        match self.waiting.pop_front() {
            Some(next) => self.show_flash(next),
            None => self.stage = self.resting.clone(),
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

    /* "Everything except these three" is otherwise `a` and then marking the
       rest back by hand. Over the rows the filter shows, like `a`, so the two
       keys mean the same amount of list. */
    pub fn invert_marks(&mut self) {
        for track in &self.tracks {
            if !self.shows(track) {
                continue;
            }
            if !self.marked.remove(&track.index) {
                self.marked.insert(track.index);
            }
        }
    }

    /* The other question people have about a run, after "which of these went
       wrong": a compilation the lookup split across twenty artists, or one
       artist's tracks to swap. `/` then `a` does it with typing; this is the
       key for the artist already under the cursor. */
    pub fn mark_like_artist(&mut self) {
        let Some(artist) = self
            .tracks
            .get(self.cursor)
            .map(|t| t.artist.trim().to_lowercase())
            .filter(|a| !a.is_empty())
        else {
            // Before the tag pass there is no artist, and a key that quietly
            // does nothing reads as a broken key.
            self.say("no artist on this track yet");
            return;
        };
        let same: Vec<usize> = self
            .tracks
            .iter()
            .filter(|t| t.artist.trim().to_lowercase() == artist && self.shows(t))
            .map(|t| t.index)
            .collect();
        if same.iter().all(|i| self.marked.contains(i)) {
            self.marked.retain(|i| !same.contains(i));
        } else {
            self.marked.extend(same);
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

    /// Whether `p` is worth offering: cliamp has to be up to be handed a
    /// playlist, and a key that only ever errors is worse than no key.
    pub fn can_play(&self) -> bool {
        self.playback.player.ready()
    }

    pub fn can_command(&self) -> bool {
        self.view == View::Tracks
            && self.done.is_some()
            && self.prompt.is_none()
            && !self.busy
            && !self.tracks.is_empty()
    }

    /// Whether `D` has anything to delete. Proven departures only: a `Gone`
    /// read off the last playlist file is not enough to delete on, and the
    /// key is not drawn for it rather than drawn and refused. No stat here,
    /// since this runs in the draw; the worker checks the file is there.
    pub fn can_purge(&self) -> bool {
        self.can_command()
            && self
                .tracks
                .iter()
                .any(|t| t.status == Status::Gone && t.departure_proven)
    }

    /// How many `D` would delete, for the hint that names it.
    pub fn purgeable(&self) -> usize {
        self.tracks
            .iter()
            .filter(|t| t.status == Status::Gone && t.departure_proven)
            .count()
    }

    /// Library keys, which unlike the track ones need no finished run behind
    /// them: a bare start with playlists on disk lands here having synced
    /// nothing.
    pub fn selected_shelf(&self) -> Option<&Shelf> {
        self.library.get(self.shelf)
    }

    /// Whether `p` on the library has something to hand over. cliamp being up
    /// is only half of it: the folder under the cursor has to hold a playlist
    /// file, and a folder earworm did not download need not. A key that can
    /// only refuse is worse than no key, so the row goes with it.
    pub fn can_play_shelf(&self) -> bool {
        self.can_play() && self.selected_shelf().is_some_and(|s| s.playlist.is_some())
    }

    /* Not gated on the library having rows: `n` is the way out of an empty
       one, and `Cmd::Open` is guarded by there being a row to open. */
    pub fn can_browse(&self) -> bool {
        self.view == View::Library && self.prompt.is_none() && !self.busy
    }

    /// Esc in the track view. Goes back to the library when there is one, and
    /// otherwise means what it always did.
    /// Whether anything is in flight that quitting would take with it. One
    /// predicate for Esc and for `q`, or the two answer the same question
    /// differently and the safe key is safe on a different set of screens.
    pub fn working(&self) -> bool {
        self.busy || (self.done.is_none() && !self.tracks.is_empty())
    }

    /* Esc goes back one step and never ends the session. It used to quit
       whenever there was no library to fall back to, which is exactly the
       state a first run is in for its whole length: one keypress, aimed at
       the filter or a prompt that had already closed, took the download with
       it. `q` and ^c are the ways out, because a key that sometimes navigates
       and sometimes exits is the one that loses a run. */
    pub fn leave_tracks(&mut self) {
        if self.working() {
            self.say("still working  ·  q quits");
        } else if self.library.is_empty() {
            self.say("nothing behind this  ·  q quits");
        } else {
            self.view = View::Library;
        }
    }

    /* Esc unwinds one step at a time: the filter first, since a hidden list is
       the thing most likely to have prompted the keypress, then the marks,
       then the library. Marks sit in the middle because they survive a cleared
       filter, and leaving the screen with stale ones would act on rows the
       next `s` the user presses was never aimed at. */
    pub fn escape_tracks(&mut self) {
        if self.narrowed() {
            self.clear_filter();
        } else if !self.marked.is_empty() {
            self.clear_marks();
        } else {
            self.leave_tracks();
        }
    }

    /* The dots down the rows are a bulk selection, and until now the only way
       back was marking everything twice or running something: a stray `m`
       left marks nobody remembers making. Esc clears them and says how many,
       or the key reads as one that did nothing. */
    pub fn clear_marks(&mut self) {
        let n = self.marked.len();
        self.marked.clear();
        let plural = if n == 1 { "track" } else { "tracks" };
        self.say(format!("unmarked {n} {plural}"));
    }

    /* `q` is the deliberate way out and stays one key everywhere nothing is
       running. Mid-download it takes yt-dlp with it, which is the last thing
       in the tool that one keypress can lose, so it asks first and says what
       it would cost. A second `q` answers it, so anyone who meant it types
       `qq` and never reads the box. */
    pub fn ask_quit(&mut self) {
        if self.working() {
            self.confirm = Some(Confirm::Quit);
        } else {
            self.quit = true;
        }
    }

    /// How far along the run is, for the header bar and the quit question.
    pub fn progress(&self) -> (usize, usize) {
        (self.settled(), self.tracks.len())
    }

    /* Rough on purpose and never shown as anything else. Tracks that were
       already on disk settle the instant the list arrives, so the first
       estimate of a part-downloaded folder is always optimistic; it needs a
       few real ones behind it before it means anything, which is what the
       floor is for. */
    pub fn eta(&self) -> Option<Duration> {
        let (done, total) = self.progress();
        let elapsed = self.run_started?.elapsed();
        if done < 3 || done >= total {
            return None;
        }
        let left = elapsed.mul_f64((total - done) as f64 / done as f64);
        (left >= Duration::from_secs(30)).then_some(left)
    }

    /// Something the UI itself has to say, where the worker is not involved.
    /// A key that deliberately does nothing has to report that, or it reads
    /// as a key that is broken.
    pub fn say(&mut self, text: impl Into<String>) {
        let text = text.into();
        /* A second message used to overwrite the first, so a bulk action's
           outcomes arrived and left inside one blink and only the last of
           them was ever readable. */
        if self.flash_until.is_some() {
            if self.waiting.len() < QUEUE {
                self.waiting.push_back(text);
            }
            return;
        }
        self.show_flash(text);
    }

    fn show_flash(&mut self, text: String) {
        let reading = PER_CHAR * text.chars().count() as u32;
        self.flash_until = Some(Instant::now() + (FLASH + reading).min(FLASH_MAX));
        self.stage = text;
    }

    /* `^t`: walk the shipped palettes with the screen in front of you, which
       is the only way anyone picks a theme. Written back the way a format
       pick is, so the next launch keeps it.

       The write happens here rather than in the worker, which owns every
       other file earworm touches. Routing it through the command channel
       would mean the save waited for the run to end — `serve` is not reading
       that channel during a pipeline — and never happened at all for anyone
       who quit first, which is the one outcome a remembered setting exists
       to avoid. `write_atomically` is what makes a write off this thread
       safe; nothing else on it touches a file. */
    pub fn cycle_theme(&mut self) {
        let (name, palette) = self.themes.after(&self.theme_name);
        let name = name.to_string();
        self.theme = palette;
        self.theme_name = name.clone();
        /* The walk moves the base palette only, so what is on screen now is a
           clean built-in. The `[colors]` table stays in the file and is
           applied on top of whatever is named at the next start, which makes
           this screen and that one different colours. Future tense on purpose:
           "still repaints it" reads as a claim about the colours in front of
           you, and those are the ones it is not repainting. */
        let kept = if self.theme_overridden {
            "  ·  [colors] repaints it again next start"
        } else {
            ""
        };
        let Some(file) = self.config_file.clone() else {
            // --no-config asked for the file to stay out of the run.
            self.say(format!("theme {name}  ·  this session{kept}"));
            return;
        };
        match crate::config::save_theme(&file, &name) {
            Ok(()) => self.say(format!("theme {name}{kept}")),
            /* Not a failure of the key: the theme applied and only the
               remembering did not, which is a different thing to tell
               somebody and a different thing to do about it. */
            Err(e) => self.say(format!("theme {name}  ·  not saved: {e}")),
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

    /// Whether a list is narrowed or about to be. The library shares the
    /// filter box; review is the track list's alone.
    pub fn filtering(&self) -> bool {
        self.typing_filter
            || !self.filter.is_empty()
            || (self.view == View::Tracks && self.review)
            || (self.view == View::Library && self.only.is_some())
    }

    /// Whether anything is narrowing the track list, for the keys that clear
    /// it and for the word the empty pane uses.
    pub fn narrowed(&self) -> bool {
        !self.filter.is_empty() || self.review
    }

    /// The same question asked of the library, where the second narrowing is
    /// the kind rather than the review.
    pub fn shelf_narrowed(&self) -> bool {
        !self.filter.is_empty() || self.only.is_some()
    }

    /* Silence is the playlist answer everywhere else in this feature, so a
       folder with no kind is on the playlist list rather than on neither. */
    pub fn kind_shows(&self, shelf: &Shelf) -> bool {
        match self.only {
            None => true,
            Some(crate::manifest::Kind::Album) => shelf.kind == Some(crate::manifest::Kind::Album),
            Some(crate::manifest::Kind::Playlist) => {
                shelf.kind != Some(crate::manifest::Kind::Album)
            }
        }
    }

    /// Walks every folder, then albums, then playlists. Albums come first
    /// because they are the ones the pill already points at.
    pub fn cycle_kind(&mut self) -> &'static str {
        use crate::manifest::Kind;
        self.only = match self.only {
            None => Some(Kind::Album),
            Some(Kind::Album) => Some(Kind::Playlist),
            Some(Kind::Playlist) => None,
        };
        self.snap();
        self.kind_label()
    }

    fn kind_label(&self) -> &'static str {
        match self.only {
            None => "every folder",
            Some(crate::manifest::Kind::Album) => "albums",
            Some(crate::manifest::Kind::Playlist) => "playlists",
        }
    }

    /// Both narrowings at once, with the needle already lowercased: review is
    /// a predicate rather than a filter string, because the four statuses it
    /// keeps share no word for the filter to match on.
    fn keeps(&self, track: &Track, needle: &str) -> bool {
        (!self.review || track.status.wants_a_look())
            && (needle.is_empty() || matched(track, needle))
    }

    /// Whether a row survives the filter, matched against the name and the
    /// status word: the artist or title you are looking for, or `/failed`.
    pub fn shows(&self, track: &Track) -> bool {
        self.keeps(track, &self.filter.to_lowercase())
    }

    /// Positions in `tracks` the list is showing, in order.
    pub fn rows(&self) -> Vec<usize> {
        if !self.narrowed() {
            return (0..self.tracks.len()).collect();
        }
        // Lowercased once for the pass, not once per row.
        let needle = self.filter.to_lowercase();
        self.tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| self.keeps(t, &needle))
            .map(|(pos, _)| pos)
            .collect()
    }

    /// How many rows are showing, for the header. Counted rather than
    /// collected, since the positions themselves are not wanted.
    pub fn shown(&self) -> usize {
        if !self.narrowed() {
            return self.tracks.len();
        }
        let needle = self.filter.to_lowercase();
        self.tracks.iter().filter(|t| self.keeps(t, &needle)).count()
    }

    /// Library rows in the order and the narrowing the screen is showing.
    /// Positions in `library`, so `shelf` keeps meaning one folder however
    /// the order changes under it.
    pub fn shelf_rows(&self) -> Vec<usize> {
        let needle = self.filter.to_lowercase();
        let mut rows: Vec<usize> = self
            .library
            .iter()
            .enumerate()
            .filter(|(_, s)| self.kind_shows(s) && (needle.is_empty() || shelf_matches(s, &needle)))
            .map(|(pos, _)| pos)
            .collect();
        match self.sort {
            // Already what the worker built, which is name order.
            Sort::Name => {}
            /* Never synced sorts first, which is where it belongs: it is the
               folder furthest from matching the playlist upstream. */
            Sort::Synced => rows.sort_by_key(|pos| self.library[*pos].synced.unwrap_or(0)),
            Sort::Missing => {
                rows.sort_by_key(|pos| std::cmp::Reverse(self.library[*pos].missing));
            }
        }
        rows
    }

    /* Every matching track in the library, as the folder it is in and its
       place in that folder's files. Recomputed like `rows()` and
       `shelf_rows()` rather than stored, so a sync that rebuilds the library
       cannot leave a result list describing folders that have moved.
       An empty box is no results and not every track: a search screen with
       nothing typed is a list of the whole library, which is what the
       library screen already is. */
    pub fn found_rows(&self) -> Vec<(usize, usize)> {
        let needle = self.filter.to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        // The kind too, or `t` answers a different question from the rows.
        self.library
            .iter()
            .enumerate()
            .filter(|(_, s)| self.kind_shows(s))
            .flat_map(|(shelf, s)| {
                s.files
                    .iter()
                    .enumerate()
                    .filter(|(_, (name, _))| name.to_lowercase().contains(&needle))
                    .map(move |(file, _)| (shelf, file))
            })
            .collect()
    }

    /// The result under the cursor, or `None` when the list is empty.
    pub fn found_at(&self) -> Option<(&Shelf, &(String, bool))> {
        let (shelf, file) = self.found?;
        let shelf = self.library.get(shelf)?;
        Some((shelf, shelf.files.get(file)?))
    }

    /// Search keys, on the screen that has its own cursor and no run behind
    /// it. Exclusive with `can_command` and `can_browse` on `View`.
    pub fn can_find(&self) -> bool {
        self.view == View::Found && self.prompt.is_none() && !self.busy
    }

    /* Enters the search screen, which is only worth showing with something to
       look for: an empty box would put up a list of nothing and the filter
       band would say so with no way to read it as anything but a bug. */
    pub fn find_tracks(&mut self) {
        if self.filter.is_empty() {
            self.typing_filter = true;
            self.view = View::Found;
            self.found = None;
            return;
        }
        self.view = View::Found;
        self.found_snap();
        if self.found.is_none() {
            self.view = View::Library;
            self.say(format!("no track matches {}", self.filter));
        }
    }

    fn move_found(&mut self, down: bool, by: usize) {
        let rows = self.found_rows();
        let Some(at) = self.found.and_then(|at| rows.iter().position(|r| *r == at)) else {
            self.found_snap();
            return;
        };
        let next = if down {
            (at + by).min(rows.len() - 1)
        } else {
            at.saturating_sub(by)
        };
        self.found = Some(rows[next]);
    }

    pub fn found_step(&mut self, down: bool) {
        self.move_found(down, 1);
    }

    pub fn page_found(&mut self, down: bool) {
        self.move_found(down, self.viewport.max(1));
    }

    pub fn found_jump(&mut self, last: bool) {
        let rows = self.found_rows();
        if let Some(at) = if last { rows.last() } else { rows.first() } {
            self.found = Some(*at);
        }
    }

    /// Puts the search cursor on a row that is showing, for the same reason
    /// the other two do it: a selection nobody can see is one Enter still
    /// acts on. Falls to the first result, since editing the query is a new
    /// question rather than a move within the old answer.
    fn found_snap(&mut self) {
        let rows = self.found_rows();
        if self.found.is_some_and(|at| rows.contains(&at)) {
            return;
        }
        self.found = rows.first().copied();
    }

    /// Whole-library totals for the status bar: what a sync would have to
    /// fetch, and how stale the stalest folder is.
    pub fn library_totals(&self) -> (usize, usize, Option<u64>) {
        (
            self.library.iter().map(|s| s.tracks).sum(),
            self.library.iter().map(|s| s.missing).sum(),
            self.library.iter().filter_map(|s| s.synced).max(),
        )
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
    /// The focused box, or an empty one when no question is open.
    pub fn input(&self) -> &str {
        self.fields.get(self.field).map_or("", String::as_str)
    }

    /// A box to edit. A question with no boxes takes no editing keys, so one
    /// to scribble on is cheaper than an Option at every call site.
    fn box_mut(&mut self) -> &mut String {
        if self.fields.is_empty() {
            self.fields.push(String::new());
            self.field = 0;
        }
        let field = self.field.min(self.fields.len() - 1);
        &mut self.fields[field]
    }

    fn open_fields(&mut self, values: Vec<String>) {
        self.fields = values;
        self.field = 0;
        // Behind the prefilled text, which is where an edit starts.
        self.caret = self.input().chars().count();
    }

    /// Tab and shift-Tab, wrapping. The caret lands behind the text it moves
    /// to, which is where an edit to an existing value starts.
    pub fn next_field(&mut self, forward: bool) {
        let n = self.fields.len();
        if n < 2 {
            return;
        }
        self.field = if forward {
            (self.field + 1) % n
        } else {
            (self.field + n - 1) % n
        };
        self.caret = self.input().chars().count();
    }

    /* The commonest correction in the tool by a distance: a playlist whose
       titles all parsed the wrong way round. Doing it by retyping both boxes
       is the work this key exists to remove. */
    pub fn swap_fields(&mut self) {
        let Some((Prompt::Form { fields, .. }, _)) = &self.prompt else {
            return;
        };
        let Some((artist, title)) = swappable(fields) else {
            return;
        };
        if artist.max(title) < self.fields.len() {
            self.fields.swap(artist, title);
            self.caret = self.input().chars().count();
        }
    }

    /// Every box, trimmed, for the reply. Empty when nothing is open.
    pub fn answers(&self) -> Vec<String> {
        self.fields.iter().map(|f| f.trim().to_string()).collect()
    }

    pub fn close_fields(&mut self) {
        self.fields.clear();
        self.field = 0;
        self.caret = 0;
    }

    /// Byte offset of the caret, for slicing. The caret is a char index, since
    /// a byte one lands inside a multi-byte character the moment a title has
    /// an accent in it.
    fn caret_byte(&self) -> usize {
        self.input()
            .char_indices()
            .nth(self.caret)
            .map_or(self.input().len(), |(at, _)| at)
    }

    /// The text either side of the caret, for drawing the block between them.
    pub fn input_parts(&self) -> (&str, &str) {
        self.input().split_at(self.caret_byte())
    }

    pub fn input_insert(&mut self, c: char) {
        let at = self.caret_byte();
        self.box_mut().insert(at, c);
        self.caret += 1;
    }

    /// Backspace deletes behind the caret, Delete in front of it. Both used to
    /// be `pop`, so fixing a letter in the middle of a title meant deleting
    /// everything after it and typing it again.
    pub fn input_backspace(&mut self) {
        if self.caret == 0 {
            return;
        }
        self.caret -= 1;
        let at = self.caret_byte();
        self.box_mut().remove(at);
    }

    pub fn input_delete(&mut self) {
        if self.caret < self.input().chars().count() {
            let at = self.caret_byte();
            self.box_mut().remove(at);
        }
    }

    pub fn input_move(&mut self, right: bool) {
        self.caret = if right {
            (self.caret + 1).min(self.input().chars().count())
        } else {
            self.caret.saturating_sub(1)
        };
    }

    pub fn input_end(&mut self, end: bool) {
        self.caret = if end { self.input().chars().count() } else { 0 };
    }

    /// The focused box only: on a form the other one is an answer you are
    /// keeping, and losing it to ^u would be its own small disaster.
    pub fn input_clear(&mut self) {
        self.box_mut().clear();
        self.caret = 0;
    }

    /// The word in front of the caret, not the end of the line: ^w after
    /// moving back would otherwise cut text the caret had already passed.
    pub fn input_kill_word(&mut self) {
        let before: String = self.input().chars().take(self.caret).collect();
        let kept = before.trim_end();
        let start = kept
            .rfind(' ')
            .map_or(0, |at| kept[..=at].chars().count());
        let tail: String = self.input().chars().skip(self.caret).collect();
        *self.box_mut() = before.chars().take(start).collect::<String>() + &tail;
        self.caret = start;
    }

    pub fn snap(&mut self) {
        self.shelf_snap();
        self.found_snap();
        // Nothing is hidden, so the cursor is already on a row that shows.
        if !self.narrowed() {
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

    /* Following is what drags the cursor to whatever is downloading, so a
       movement key has to end it or the next progress message undoes the
       move. That is the right behaviour and an invisible one: the cursor
       simply stops being dragged, which is indistinguishable from the key
       having broken it. */
    fn stop_following(&mut self) {
        if !self.follow {
            return;
        }
        self.follow = false;
        /* Only while it is doing something. Once the run has settled nothing
           is moving the cursor anyway, so announcing that it has stopped is
           news about nothing, and it would land on the first `j` of every
           session spent fixing tags. */
        if self.working() {
            self.say("following off  ·  f follows again");
        }
    }

    /// Whether the bar should say so: the same condition as the flash, or the
    /// word and the message disagree about when the mode is live.
    pub fn following(&self) -> bool {
        self.follow && self.working() && self.view == View::Tracks
    }

    /// One row down or up through what is showing.
    pub fn step(&mut self, down: bool) {
        self.stop_following();
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
        self.stop_following();
        let rows = self.rows();
        let at = if last { rows.last() } else { rows.first() };
        if let Some(pos) = at {
            self.cursor = *pos;
        }
    }

    /// A screenful at a time, clamped at both ends. Stops following for the
    /// same reason `j` and `G` do.
    pub fn page(&mut self, down: bool) {
        self.stop_following();
        let rows = self.rows();
        let Some(at) = rows.iter().position(|pos| *pos == self.cursor) else {
            self.snap();
            return;
        };
        let step = self.viewport.max(1);
        let next = if down {
            (at + step).min(rows.len().saturating_sub(1))
        } else {
            at.saturating_sub(step)
        };
        self.cursor = rows[next];
    }

    pub fn page_shelf(&mut self, down: bool) {
        self.move_shelf(down, self.viewport.max(1));
    }

    /// One row down or up through the library as it is ordered and narrowed.
    pub fn shelf_step(&mut self, down: bool) {
        self.move_shelf(down, 1);
    }

    fn move_shelf(&mut self, down: bool, by: usize) {
        let rows = self.shelf_rows();
        let Some(at) = rows.iter().position(|pos| *pos == self.shelf) else {
            self.shelf_snap();
            return;
        };
        let next = if down {
            (at + by).min(rows.len() - 1)
        } else {
            at.saturating_sub(by)
        };
        self.shelf = rows[next];
    }

    pub fn shelf_jump(&mut self, last: bool) {
        let rows = self.shelf_rows();
        if let Some(pos) = if last { rows.last() } else { rows.first() } {
            self.shelf = *pos;
        }
    }

    /// Puts the library cursor back on a row that is showing, for the same
    /// reason `snap` does it for tracks: a selection nobody can see is one
    /// every key still acts on.
    fn shelf_snap(&mut self) {
        let rows = self.shelf_rows();
        if rows.is_empty() || rows.contains(&self.shelf) {
            // Still clamped, since the library is rebuilt after every sync.
            self.shelf = self.shelf.min(self.library.len().saturating_sub(1));
            return;
        }
        self.shelf = rows
            .iter()
            .copied()
            .min_by_key(|pos| pos.abs_diff(self.shelf))
            .unwrap_or(self.shelf);
    }

    /* Filtering to the guesses hides the tracks around them, and the run they
       came from is the context that says whether a guess is plausible. Jumping
       keeps both: the list stays whole and the cursor walks the problems. */
    pub fn jump_attention(&mut self, forward: bool) {
        let rows = self.rows();
        // A filter that matches nothing, which is exactly when someone reaches
        // for this key: `rows[at + 1..]` on an empty list is a panic.
        if rows.is_empty() {
            self.say("nothing here needs a look");
            return;
        }
        let at = self.row_of_cursor(&rows).unwrap_or(0);
        let hit = |pos: &usize| self.tracks[*pos].status.wants_a_look();
        let next = if forward {
            rows[at + 1..].iter().find(|p| hit(p))
        } else {
            rows[..at].iter().rev().find(|p| hit(p))
        };
        match next {
            Some(pos) => {
                self.stop_following();
                self.cursor = *pos;
            }
            /* Says which end it stopped at rather than wrapping: a jump that
               silently starts again reads as a key that did nothing. */
            None if rows.iter().any(hit) => {
                let end = if forward { "last" } else { "first" };
                self.say(format!("that is the {end} one to look at"));
            }
            None => self.say("nothing here needs a look"),
        }
    }

    /// Rows of yt-dlp output the pane gives up to, once `L` has grown it.
    /// Half the body, so the list it covers is still the larger half.
    pub fn log_rows(&self, body: usize) -> usize {
        if self.logs_tall {
            (body / 2).max(3)
        } else {
            LOG_ROWS.min(body)
        }
    }

    /// Moves the window over the log, clamped at the tail and at the first
    /// line. `by` is a screenful, since that is what the caller can see.
    pub fn scroll_logs(&mut self, up: bool, by: usize) {
        let ceiling = self.logs.len().saturating_sub(1);
        self.log_scroll = if up {
            (self.log_scroll + by).min(ceiling)
        } else {
            self.log_scroll.saturating_sub(by)
        };
    }

    /// Whether anything on stderr was a real error rather than the warnings
    /// a good run also produces.
    pub fn log_errors(&self) -> bool {
        self.logs.iter().any(|l| l.contains("ERROR"))
    }

    /// Esc on a narrowed list. Every narrowing goes together: they are one
    /// thing on screen, a band saying the list is not all of it, and leaving
    /// half of it on would make the key look like it had failed.
    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.typing_filter = false;
        self.review = false;
        self.only = None;
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

    /* Built from the tracks rather than from a list of every status, which
       is the whole point: a list is hand-kept and adding a variant does not
       fail to compile against it, so the first version of this still had the
       bug it was meant to remove. `tally()` is exhaustive and decides both
       whether a status is counted and where it sits, so a new one is either
       given a column or deliberately left out, and neither can be forgotten. */
    pub fn counts(&self) -> Vec<(Status, usize)> {
        let mut seen: Vec<(u8, Status, usize)> = Vec::new();
        for track in &self.tracks {
            let Some(rank) = track.status.tally() else {
                continue;
            };
            match seen.iter_mut().find(|(_, s, _)| *s == track.status) {
                Some((_, _, n)) => *n += 1,
                None => seen.push((rank, track.status, 1)),
            }
        }
        seen.sort_unstable_by_key(|(rank, _, _)| *rank);
        seen.into_iter().map(|(_, s, n)| (s, n)).collect()
    }

    pub fn settled(&self) -> usize {
        self.tracks.iter().filter(|t| t.status.settled()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An `App` with nothing in it, for the keys that answer by themselves.
    fn bare() -> App {
        let (tx, _rx) = std::sync::mpsc::channel();
        App::new(tx, String::new())
    }

    /* The cover is decoded once and held against the path it came from, so a
       rewrite of that same file is invisible to the pane: it keeps drawing the
       picture it already has. `c` and the restore row both change art behind a
       path that does not move, and this message is the only thing that tells
       the pane so. */
    #[test]
    fn changed_artwork_drops_the_cover_the_pane_is_holding() {
        let picker = ratatui_image::picker::Picker::halfblocks();
        let protocol = picker
            .new_protocol(
                image::DynamicImage::ImageRgb8(image::RgbImage::new(8, 8)),
                ratatui::layout::Size::new(4, 2),
                ratatui_image::Resize::Fit(None),
            )
            .expect("the halfblocks encoder refused a plain image");
        let mut app = bare();
        app.art = Some((
            PathBuf::from("/music/Focus/01 - A.opus"),
            ratatui::layout::Rect::new(0, 0, 4, 2),
            protocol,
        ));

        app.apply(Msg::Artwork);
        assert!(
            app.art.is_none(),
            "the pane kept a cover that had been replaced"
        );
    }

    /* A path of its own per call: the harness runs these in parallel, and two
       sharing a name is one test deleting another's fixture. */
    fn scratch_path(tag: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!("earworm-theme-{tag}-{}-{n}", std::process::id()))
    }

    /// The same, with a config already in it for a save to edit rather than
    /// create: the file `^t` writes to is one somebody wrote by hand.
    fn scratch_config(tag: &str) -> PathBuf {
        let path = scratch_path(tag).with_extension("toml");
        std::fs::write(&path, "format = \"opus\"\n").unwrap();
        path
    }

    /* `^t` is only usable as a walk if it goes round the houses and comes
       back: a key that stops at the last palette is one that cannot return to
       the one somebody started on. */
    #[test]
    fn cycling_the_theme_walks_every_shipped_palette_and_returns() {
        let mut app = bare();
        assert_eq!(app.theme, crate::theme::WARM);

        let mut seen = vec![app.theme_name.clone()];
        for _ in 1..crate::theme::BUILT_INS.len() {
            app.cycle_theme();
            seen.push(app.theme_name.clone());
        }
        assert_eq!(seen, ["warm", "light", "cool", "neon"]);

        app.cycle_theme();
        assert_eq!(app.theme, crate::theme::WARM, "the walk did not come back");
    }

    /* The whole point of walking with the screen in front of you is stopping
       on one, so the one you stop on has to still be there next launch. */
    #[test]
    fn the_theme_a_walk_lands_on_is_written_back() {
        let path = scratch_config("saves");
        let mut app = bare();
        app.config_file = Some(path.clone());

        app.cycle_theme();
        assert_eq!(app.theme_name, "light");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("theme = \"light\""), "{text}");
        assert!(text.contains("format = \"opus\""), "another key went: {text}");
        assert!(app.stage.contains("theme light"), "the flash said nothing: {}", app.stage);

        // A second press replaces the line rather than adding another.
        app.cycle_theme();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches("theme =").count(), 1, "a second key: {text}");
        assert!(text.contains("theme = \"cool\""), "{text}");
        std::fs::remove_file(&path).unwrap();
    }

    /* The walk has to reach a theme somebody defined, or `--theme` is the
       only way to select one and half the feature is missing. Written back by
       its own name, so the next launch opens on it. */
    #[test]
    fn the_walk_reaches_a_defined_theme_and_writes_its_name_back() {
        let path = scratch_config("defined");
        let mut app = bare();
        app.config_file = Some(path.clone());

        let mut mine = crate::theme::COOL;
        mine.accent = ratatui::style::Color::Rgb(255, 92, 213);
        app.themes.add("midnight".into(), mine);
        // Parked on the last shipped theme, so one press steps off the end.
        app.theme_name = "neon".into();
        app.theme = crate::theme::NEON;

        app.cycle_theme();
        assert_eq!(app.theme_name, "midnight", "the walk stopped at the built-ins");
        assert_eq!(app.theme, mine, "it took the name without the colours");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("theme = \"midnight\""), "{text}");

        // And carries on round to the start rather than stopping there.
        app.cycle_theme();
        assert_eq!(app.theme_name, "warm");
        std::fs::remove_file(&path).unwrap();
    }

    /* --no-config asked for the file to stay out of the run, so there is
       nowhere to write. The theme still applies, and the flash has to say the
       part that is different, or the next launch is a surprise. */
    #[test]
    fn with_no_config_the_walk_says_it_lasts_one_session() {
        let mut app = bare();
        app.cycle_theme();
        assert_eq!(app.theme_name, "light");
        assert!(app.stage.contains("this session"), "{}", app.stage);
    }

    /* A `[colors]` table outlives the walk: it is applied on top of whatever
       is named at the next launch, so the screen now and the screen then are
       different colours. Saying only the name would be the wrong half.

       The walk itself is no longer confused by one. It goes by name, and the
       name is the base the overrides were painted on, so `^t` from a
       repainted `neon` reaches `warm` because warm is what follows neon — not
       because a palette matching no built-in dropped the walk back to its
       start, which is what it used to do. */
    #[test]
    fn a_walk_over_a_repainted_palette_says_the_table_is_still_there() {
        let path = scratch_config("repainted");
        let mut app = bare();
        app.config_file = Some(path.clone());
        app.theme_overridden = true;
        // What a `[colors]` table leaves behind: a named base, repainted.
        app.theme_name = "cool".into();
        app.theme = crate::theme::COOL;
        app.theme.cursor = ratatui::style::Color::Rgb(0, 255, 136);

        app.cycle_theme();
        assert_eq!(app.theme_name, "neon", "the walk lost its place");
        assert_eq!(app.theme, crate::theme::NEON, "the overrides outlived the walk");
        assert!(app.stage.contains("[colors]"), "the table went unmentioned: {}", app.stage);
        /* And says it about the next start, not this screen: the walk landed
           on a clean built-in, so the table is the one thing here that is not
           repainting what is in front of you. */
        assert!(app.stage.contains("next start"), "read as the present: {}", app.stage);
        std::fs::remove_file(&path).unwrap();
    }

    /* The theme applied and only the remembering failed, which is a different
       thing to tell somebody: saying nothing would leave the next launch
       quietly disagreeing with the screen. */
    #[test]
    fn a_theme_that_cannot_be_saved_still_applies_and_says_so() {
        let mut app = bare();
        /* A directory where the config should be, so the read fails. Through
           the same counter as every other scratch path here: the harness runs
           these in parallel and a shared name is a test deleting another
           test's fixture. */
        let dir = scratch_path("unwritable");
        std::fs::create_dir_all(&dir).unwrap();
        app.config_file = Some(dir.clone());

        app.cycle_theme();
        assert_eq!(app.theme_name, "light", "the theme did not apply");
        assert!(app.stage.contains("not saved"), "{}", app.stage);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn stocked(name: &str, files: &[&str]) -> Shelf {
        Shelf {
            files: files.iter().map(|f| ((*f).to_string(), true)).collect(),
            tracks: files.len(),
            ..folder(name, 0, None)
        }
    }

    fn folder(name: &str, missing: usize, synced: Option<u64>) -> Shelf {
        Shelf {
            path: PathBuf::from("/tmp").join(name),
            name: name.into(),
            url: Some("u".into()),
            tracks: 10,
            missing,
            synced,
            ..Shelf::default()
        }
    }

    fn library_app(shelves: Vec<Shelf>) -> App {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        app.library = shelves;
        app.view = View::Library;
        app
    }

    /* Following drags the cursor to whatever is downloading, so a movement
       key has to end it. That is invisible: the cursor simply stops being
       dragged, which looks exactly like the key having broken it. */
    #[test]
    fn a_movement_key_says_it_has_turned_following_off() {
        let mut app = app_with(&[Status::Ok, Status::Pending]);
        assert!(app.following(), "a run in flight is not following");
        app.step(true);
        assert!(!app.follow);
        assert!(app.stage.contains("following off"), "{}", app.stage);
        assert!(!app.following(), "the bar still says it is following");

        // Said once: the second key has nothing left to turn off.
        app.stage = "downloading".into();
        app.step(true);
        assert_eq!(app.stage, "downloading");

        /* Nothing is moving the cursor once the run has settled, so the
           mode is not live and saying it stopped is news about nothing. */
        let mut settled = app_with(&[Status::Ok]);
        settled.done = Some(Ok(String::new()));
        assert!(!settled.following(), "a finished run was still following");
        settled.stage = "finished".into();
        settled.step(true);
        assert_eq!(settled.stage, "finished", "it announced a mode nobody could see");
    }

    /* `a` then marking nineteen of twenty back by hand is the work this
       removes, and it has to mean the same amount of list that `a` does. */
    #[test]
    fn inverting_marks_stops_at_the_filter_like_marking_all_does() {
        let mut app = app_with(&[Status::Ok, Status::Failed, Status::Ok]);
        app.tracks[0].name = "Neu! - Hallogallo".into();
        app.tracks[1].name = "Can - Vitamin C".into();
        app.tracks[2].name = "Neu! - Negativland".into();
        app.marked.insert(1);

        app.invert_marks();
        // The one that was marked comes off, the two that were not go on.
        assert_eq!(app.marked, [2, 3].into_iter().collect());

        app.marked.clear();
        app.filter = "neu".into();
        app.invert_marks();
        assert_eq!(app.marked, [1, 3].into_iter().collect(), "it reached past the filter");
    }

    /* A compilation the lookup split across twenty artists, or one artist's
       rows to swap: the question `m` cannot answer. */
    #[test]
    fn marking_by_artist_takes_the_cursors_artist_and_no_other() {
        let mut app = app_with(&[Status::Ok, Status::Ok, Status::Ok]);
        for (pos, artist) in ["Neu!", "Can", "neu!"].into_iter().enumerate() {
            app.tracks[pos].artist = artist.into();
        }
        app.cursor = 0;
        app.mark_like_artist();
        // Case is a tagging accident, not a different artist.
        assert_eq!(app.marked, [1, 3].into_iter().collect());

        // The same key takes them off again, like `m` does.
        app.mark_like_artist();
        assert!(app.marked.is_empty());

        /* Before the tag pass there is no artist to match on, and a key that
           silently does nothing reads as a broken one. */
        let mut bare = app_with(&[Status::Pending, Status::Pending]);
        bare.mark_like_artist();
        assert!(bare.marked.is_empty());
        assert!(bare.stage.contains("no artist"), "{}", bare.stage);
    }

    /* The header said `downloading` while the row said `getting`, and
       `reading` on a row was tag reading while `reading playlist` in the
       header was the scan. Two words for one thing reads as two things. */
    #[test]
    fn the_status_words_use_the_verbs_the_header_does() {
        assert_eq!(Status::Downloading.label(), "fetching");
        assert_eq!(Status::Downloaded.label(), "fetched");
        assert_eq!(Status::Tagging.label(), "tagging");
        // Still inside the column every row is laid out against.
        for status in [
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
        ] {
            assert!(
                status.label().chars().count() <= 8,
                "{} does not fit the status column",
                status.label()
            );
        }
    }

    /* Filtering to the guesses hides the tracks either side of them, and a
       guess is judged against the run it came from. `n` keeps both. */
    #[test]
    fn n_walks_the_tracks_nothing_confirmed_and_stops_at_the_ends() {
        let mut app = app_with(&[
            Status::Ok,
            Status::Kept,
            Status::Ok,
            Status::Failed,
            Status::Ok,
        ]);
        // Already off, as it is by the time anyone is walking the guesses;
        // the flash it would otherwise raise is its own test.
        app.follow = false;
        app.jump_attention(true);
        assert_eq!(app.cursor, 1);
        app.jump_attention(true);
        assert_eq!(app.cursor, 3);

        // No wrap: a jump that quietly starts again reads as a dead key.
        app.jump_attention(true);
        assert_eq!(app.cursor, 3, "it wrapped");
        assert!(app.stage.contains("last one"), "{}", app.stage);

        app.jump_attention(false);
        assert_eq!(app.cursor, 1);
        // Following would drag the cursor straight back to the active track.
        assert!(!app.follow, "the jump left follow on");
    }

    /// A run with nothing to check says so rather than moving the cursor.
    #[test]
    fn n_on_a_clean_run_says_there_is_nothing_to_look_at() {
        let mut app = app_with(&[Status::Ok, Status::Manual, Status::Have]);
        app.cursor = 1;
        app.jump_attention(true);
        assert_eq!(app.cursor, 1);
        assert!(app.stage.contains("nothing here"), "{}", app.stage);
    }

    /* A filter that matches nothing is the state the key is most likely to be
       pressed in, since the pane is showing that it matched nothing. The jump
       used to index past the end of an empty list and take the tool with it. */
    #[test]
    fn n_on_a_list_the_filter_emptied_says_so_rather_than_panicking() {
        let mut app = app_with(&[Status::Kept, Status::Failed]);
        app.filter = "zzzz".into();
        assert!(app.rows().is_empty());

        app.jump_attention(true);
        app.jump_attention(false);
        assert!(app.stage.contains("nothing here"), "{}", app.stage);
    }

    /* The four statuses worth reviewing share no word for the filter to
       match, so the review is a predicate beside it rather than a string in
       the box. Both narrowings have to survive being combined. */
    #[test]
    fn review_keeps_only_the_unconfirmed_and_composes_with_the_filter() {
        let mut app = app_with(&[Status::Ok, Status::Kept, Status::Failed, Status::Have]);
        app.tracks[1].name = "Neu! - Hallogallo".into();
        app.tracks[2].name = "Can - Vitamin C".into();
        app.apply(Msg::Review);

        assert!(app.review);
        assert_eq!(app.rows(), vec![1, 2]);
        assert_eq!(app.cursor, 1, "the cursor did not land on the first of them");
        assert_eq!(app.shown(), 2);

        app.filter = "neu".into();
        assert_eq!(app.rows(), vec![1], "the filter stopped at the review");

        /* One key clears both: they are one thing on screen, a band saying
           the list is not all of it. */
        app.clear_filter();
        assert!(!app.review && app.filter.is_empty());
        assert_eq!(app.rows().len(), 4);
    }

    /// A clean run has nothing to review, and the menu row that asks for one
    /// must not leave an empty list behind it.
    #[test]
    fn review_of_a_clean_run_narrows_to_nothing() {
        let mut app = app_with(&[Status::Ok, Status::Have]);
        app.apply(Msg::Review);
        assert!(!app.review, "an empty list with nothing saying why");
    }

    /* `shelf` stays a position in `library`, so the order is a view over it:
       a sorted vector would move the folder out from under the cursor every
       time a sync changed a count. */
    #[test]
    fn the_library_order_is_a_view_and_the_cursor_survives_it() {
        let mut app = library_app(vec![
            folder("Focus", 0, Some(500)),
            folder("Road trip", 4, None),
            folder("Sleep", 1, Some(100)),
        ]);
        assert_eq!(app.shelf_rows(), vec![0, 1, 2]);

        // Never synced first: it is the furthest from matching upstream.
        app.sort = Sort::Synced;
        assert_eq!(app.shelf_rows(), vec![1, 2, 0]);
        app.sort = Sort::Missing;
        assert_eq!(app.shelf_rows(), vec![1, 2, 0]);

        app.shelf = 2;
        app.shelf_step(true);
        assert_eq!(app.shelf, 0, "the step walked the library, not the screen");
        app.shelf_jump(false);
        assert_eq!(app.shelf, 1);
        assert_eq!(app.library[app.shelf].name, "Road trip");
    }

    /// The same box the track list uses, over shelf names, with the cursor
    /// put back on a row that is still showing.
    #[test]
    fn the_library_filter_narrows_the_rows_and_moves_the_cursor_onto_one() {
        let mut app = library_app(vec![
            folder("Focus", 0, None),
            folder("Road trip", 0, None),
            folder("Rowing", 0, None),
        ]);
        app.shelf = 0;
        app.filter = "ro".into();
        app.snap();

        assert_eq!(app.shelf_rows(), vec![1, 2]);
        assert_eq!(app.shelf, 1, "the cursor stayed on a row nobody can see");
        assert!(app.filtering(), "the band would not say the list is narrowed");
        assert_eq!(app.library_totals(), (30, 0, None));
    }

    /* The kind is a predicate and not a query, so it has to keep the folder
       that answered nothing: silence is the playlist answer everywhere else
       in this feature, and dropping it from both lists would leave rows no
       setting of this key can reach. */
    #[test]
    fn the_kind_key_walks_albums_then_playlists_and_keeps_the_unmarked() {
        use crate::manifest::Kind;
        let kinded = |name: &str, kind: Option<Kind>| Shelf { kind, ..folder(name, 0, None) };
        let mut app = library_app(vec![
            kinded("Autobahn", Some(Kind::Album)),
            kinded("Road trip", Some(Kind::Playlist)),
            kinded("Sleep", None),
        ]);

        assert_eq!(app.shelf_rows(), vec![0, 1, 2]);
        assert!(!app.filtering(), "the band is up with nothing narrowing");

        assert_eq!(app.cycle_kind(), "albums");
        assert_eq!(app.shelf_rows(), vec![0]);
        assert!(app.filtering(), "the band would not say the list is narrowed");
        assert!(app.shelf_narrowed(), "Esc would have nothing to clear");

        assert_eq!(app.cycle_kind(), "playlists");
        assert_eq!(app.shelf_rows(), vec![1, 2]);

        assert_eq!(app.cycle_kind(), "every folder");
        assert_eq!(app.shelf_rows(), vec![0, 1, 2]);
        assert!(!app.shelf_narrowed());
    }

    /// Both narrowings at once, and one Esc for the pair: half of it left on
    /// is a band saying the list is short with nothing to turn off.
    #[test]
    fn the_kind_composes_with_the_filter_and_esc_clears_the_pair() {
        use crate::manifest::Kind;
        let kinded = |name: &str, kind: Option<Kind>| Shelf { kind, ..folder(name, 0, None) };
        let mut app = library_app(vec![
            kinded("Autobahn", Some(Kind::Album)),
            kinded("Radio-Activity", Some(Kind::Album)),
            kinded("Road trip", None),
        ]);
        app.shelf = 2;
        app.only = Some(Kind::Album);
        app.filter = "ra".into();
        app.snap();

        assert_eq!(app.shelf_rows(), vec![1]);
        assert_eq!(app.shelf, 1, "the cursor stayed on a row nobody can see");

        app.clear_filter();
        assert_eq!(app.only, None, "the kind survived the key that clears it");
        assert_eq!(app.shelf_rows(), vec![0, 1, 2]);
    }

    /* Sixty folders and no way to ask which one holds a track: the answer was
       opening them one at a time. The filenames are already in memory from
       the stat pass, so the box that narrows by folder name narrows by track
       as well. */
    #[test]
    fn the_library_filter_finds_a_folder_by_a_track_inside_it() {
        let mut app = library_app(vec![
            stocked("Focus", &["01 Steve Reich - Music for 18.opus"]),
            stocked("Road trip", &["01 Neu! - Hallogallo.opus", "02 Can - Spoon.opus"]),
            stocked("Sleep", &["01 Eno - Ascent.opus"]),
        ]);

        // A track nobody would guess the folder of from its name.
        app.filter = "hallogallo".into();
        assert_eq!(app.shelf_rows(), vec![1]);

        // Case is a tagging accident here as much as anywhere else.
        app.filter = "NEU!".into();
        assert_eq!(app.shelf_rows(), vec![1]);

        // The folder's own name still matches, and matching both is one row.
        app.filter = "o".into();
        assert_eq!(app.shelf_rows(), vec![0, 1, 2]);
        app.filter = "focus".into();
        assert_eq!(app.shelf_rows(), vec![0]);

        // And something in neither place narrows to nothing rather than all.
        app.filter = "zzzz".into();
        assert!(app.shelf_rows().is_empty());
    }

    /* The library screen answers which folders hold a match and its preview
       answers which tracks, one folder at a time and only where there is room
       to draw it. This is the same question asked of everything at once. */
    #[test]
    fn the_search_screen_lists_every_matching_track_with_its_folder() {
        let mut app = library_app(vec![
            stocked("Focus", &["01 Neu! - Hallogallo.opus", "02 Eno - Ascent.opus"]),
            stocked("Road trip", &["01 Can - Spoon.opus"]),
            stocked("Sleep", &["01 Neu! - Weissensee.opus"]),
        ]);

        // Nothing typed is no results, not the whole library: that list is
        // the library screen, which this one was reached from.
        assert!(app.found_rows().is_empty());

        app.filter = "neu!".into();
        app.find_tracks();
        assert_eq!(app.view, View::Found);
        assert_eq!(app.found_rows(), vec![(0, 0), (2, 0)]);
        assert_eq!(app.found, Some((0, 0)), "the cursor did not land on the first");

        // The pair names the track, so the folder comes back with it.
        let (shelf, (name, _)) = app.found_at().unwrap();
        assert_eq!(shelf.name, "Focus");
        assert_eq!(name, "01 Neu! - Hallogallo.opus");

        app.found_step(true);
        assert_eq!(app.found_at().unwrap().0.name, "Sleep");
        // Clamped at both ends rather than wrapping, like every other list.
        app.found_step(true);
        assert_eq!(app.found, Some((2, 0)));
        app.found_jump(false);
        assert_eq!(app.found, Some((0, 0)));
    }

    /// Same box, same answer: with the kind on, `t` must not turn up tracks
    /// from the folders the rows just hid.
    #[test]
    fn the_search_screen_honours_the_kind_the_rows_are_narrowed_to() {
        use crate::manifest::Kind;
        let mut app = library_app(vec![
            Shelf { kind: Some(Kind::Album), ..stocked("Autobahn", &["01 Kraftwerk - Autobahn.opus"]) },
            stocked("Road trip", &["01 Kraftwerk - Neonlicht.opus"]),
        ]);
        app.filter = "kraftwerk".into();
        app.only = Some(Kind::Album);
        assert_eq!(app.shelf_rows(), vec![0]);
        assert_eq!(app.found_rows(), vec![(0, 0)]);

        app.only = Some(Kind::Playlist);
        assert_eq!(app.found_rows(), vec![(1, 0)]);
    }

    /* The cursor is the pair and not a row number, because a sync rebuilds
       the library underneath it and editing the query rebuilds the list. */
    #[test]
    fn the_search_cursor_survives_the_list_being_rebuilt() {
        let mut app = library_app(vec![
            stocked("Focus", &["01 Neu! - Hallogallo.opus"]),
            stocked("Sleep", &["01 Neu! - Weissensee.opus"]),
        ]);
        app.filter = "neu!".into();
        app.find_tracks();
        app.found_step(true);
        assert_eq!(app.found, Some((1, 0)));

        // A narrower query drops the row the cursor was on.
        app.filter = "hallogallo".into();
        app.snap();
        assert_eq!(app.found, Some((0, 0)), "the cursor stayed on a row nobody can see");

        // And a query matching nothing leaves nothing for Enter to act on.
        app.filter = "zzzz".into();
        app.snap();
        assert_eq!(app.found, None);
        assert!(app.found_at().is_none());
    }

    /* A screen listing nothing, with the box that fills it already closed,
       is a dead end: the key says so and stays where it was. */
    #[test]
    fn searching_for_something_that_is_not_there_stays_on_the_library() {
        let mut app = library_app(vec![stocked("Focus", &["01 Eno - Ascent.opus"])]);
        app.filter = "hallogallo".into();
        app.find_tracks();
        assert_eq!(app.view, View::Library);
        assert!(app.stage.contains("no track matches"), "{}", app.stage);

        // With nothing typed it opens the box instead, which is the one thing
        // that can turn an empty screen into a useful one.
        app.filter.clear();
        app.find_tracks();
        assert_eq!(app.view, View::Found);
        assert!(app.typing_filter);
    }

    /* Scrolled back means reading something, and the tail is where a running
       job writes: a line arriving must not shift what is being read. */
    #[test]
    fn the_log_window_holds_still_while_new_lines_arrive() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        for n in 0..10 {
            app.apply(Msg::Log(format!("line {n}")));
        }
        app.scroll_logs(true, 4);
        assert_eq!(app.log_scroll, 4);
        app.apply(Msg::Log("line 10".into()));
        assert_eq!(app.log_scroll, 5, "the window moved under the reader");

        // The tail is the resting position and the first line is the ceiling.
        app.scroll_logs(false, 99);
        assert_eq!(app.log_scroll, 0);
        app.scroll_logs(true, 99);
        assert_eq!(app.log_scroll, app.logs.len() - 1);

        assert_eq!(app.log_rows(30), LOG_ROWS);
        app.logs_tall = true;
        assert_eq!(app.log_rows(30), 15, "the list kept less than half the body");
    }

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
        assert!(asker.input_noted("h", "", "", Escape::Quit).is_none());
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

    /* `Dropped` takes rows off the list without `Tracks`'s side effects: the
       estimate's clock stays where it is, marks on the dropped rows go, and
       the cursor cannot be left pointing past the end. */
    #[test]
    fn dropped_rows_leave_and_take_their_marks_with_them() {
        let mut app = named(&["A", "B", "C", "D"]);
        app.run_started = Some(Instant::now() - Duration::from_secs(60));
        let clock = app.run_started;
        app.marked.insert(2);
        app.marked.insert(4);
        app.cursor = 3;

        app.apply(Msg::Dropped(vec![2, 4]));
        assert_eq!(app.tracks.iter().map(|t| t.index).collect::<Vec<_>>(), [1, 3]);
        assert!(!app.marked.contains(&2) && !app.marked.contains(&4), "a mark outlived its row");
        assert_eq!(app.cursor, 1, "the cursor points past the end");
        assert_eq!(app.run_started, clock, "the estimate's clock restarted");
    }

    /* The key is drawn only for departures a listing proved. One read off
       the playlist file looks the same on the row and must not light it. */
    #[test]
    fn purge_is_offered_only_for_proven_departures() {
        let mut app = named(&["A", "B"]);
        app.done = Some(Ok(String::new()));
        assert!(!app.can_purge(), "offered with nothing departed");

        app.tracks[1].status = Status::Gone;
        assert!(!app.can_purge(), "offered for a departure read offline");
        assert_eq!(app.purgeable(), 0);

        app.tracks[1].departure_proven = true;
        assert!(app.can_purge());
        assert_eq!(app.purgeable(), 1);

        // Never while the worker is busy or the run is still going.
        app.busy = true;
        assert!(!app.can_purge());
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

    /* `p` plays `folder`, and a second playlist in the same session reuses the
       App: a folder left over from the last run would play its tracks under
       the new run's name. */
    #[test]
    fn the_open_folder_does_not_survive_a_new_playlist() {
        let mut app = app_with(&[Status::Ok]);
        app.apply(Msg::Folder(PathBuf::from("/music/Focus")));
        assert_eq!(app.folder.as_deref(), Some(std::path::Path::new("/music/Focus")));

        app.apply(Msg::Restart);
        assert!(app.folder.is_none(), "the last playlist's folder stayed behind");
    }

    fn at(player: Player, folder: Option<&str>, playing: bool) -> Playback {
        Playback {
            player,
            folder: folder.map(PathBuf::from),
            track: None,
            playing,
        }
    }

    /* The guard inside `Watch` is the only thing between a three-second poll
       and a message every three seconds for the life of the session, and
       dropping it breaks nothing visible. */
    #[test]
    fn the_player_watch_reports_changes_and_nothing_else() {
        let mut watch = Watch::default();
        let up = || at(Player::Running, Some("/music/Focus"), true);
        assert_eq!(watch.poll(up()), Some(up()), "the first look is news");
        assert_eq!(watch.poll(up()), None, "an unchanged state was re-sent");
        assert_eq!(watch.poll(up()), None);

        // The folder alone moving is a change: it is what marks the row.
        let moved = at(Player::Running, Some("/music/Road trip"), true);
        assert_eq!(watch.poll(moved.clone()), Some(moved), "a new playlist went unsaid");

        // So is pausing, with everything else the same.
        let paused = at(Player::Running, Some("/music/Road trip"), false);
        assert_eq!(watch.poll(paused.clone()), Some(paused), "pausing went unsaid");

        let gone = at(Player::Stopped, None, false);
        assert_eq!(watch.poll(gone.clone()), Some(gone), "cliamp quitting went unsaid");
        assert_eq!(watch.poll(at(Player::Stopped, None, false)), None);
    }

    /// `p` is offered on exactly one of the three, and only that one.
    #[test]
    fn only_a_running_player_is_ready() {
        assert!(Player::Running.ready());
        assert!(!Player::Stopped.ready());
        assert!(!Player::Missing.ready());
    }

    /* cliamp reports the track and never the playlist, so the row is matched
       by the folder the track sits in. A radio stream belongs to none. */
    #[test]
    fn only_the_loaded_folder_is_on_air() {
        let now = at(Player::Running, Some("/music/Focus"), true);
        assert!(now.on(Path::new("/music/Focus")));
        assert!(!now.on(Path::new("/music/Road trip")));
        assert!(!now.on(Path::new("/music")), "the parent claimed the track");

        let stream = at(Player::Running, None, true);
        assert!(!stream.on(Path::new("/music/Focus")), "a stream marked a row");
    }

    /* The shelf's path came off disk and this one came back out of cliamp, so
       a Korean folder can arrive in either form and the row would silently
       never light up. Both spellings are the same folder. */
    #[test]
    fn a_korean_folder_is_on_air_in_either_form() {
        let decomposed = "/music/음모와 사극";
        let composed = "/music/음모와 사극";
        assert_ne!(decomposed, composed, "the fixture has nothing to decompose");
        for reported in [decomposed, composed] {
            let now = at(Player::Running, Some(reported), true);
            assert!(now.on(Path::new(composed)), "composed shelf missed {reported:?}");
            assert!(now.on(Path::new(decomposed)), "decomposed shelf missed {reported:?}");
            assert!(!now.on(Path::new("/music/Focus")), "it matched another folder");
        }
    }

    fn shelf(name: &str) -> Shelf {
        Shelf {
            path: PathBuf::from("/music").join(name),
            name: name.into(),
            url: Some("u".into()),
            tracks: 3,
            ..Shelf::default()
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

    /* One question for the whole edit. Two prompts in a row meant two Enters
       and the second covering the answer to the first. */
    #[test]
    fn a_form_edits_both_fields_and_answers_once() {
        let mut app = App::new(std::sync::mpsc::channel().0, String::new());
        app.apply(Msg::Ask(
            Prompt::Form {
                header: "track 4".into(),
                note: "Roygbiv - Boards Of Canada (Official)".into(),
                fields: vec![
                    ("artist".into(), "Roygbiv".into()),
                    ("title".into(), "Boards Of Canada".into()),
                ],
                escape: Escape::Skip,
            },
            std::sync::mpsc::channel().0,
        ));
        assert_eq!(app.field, 0);
        assert_eq!(app.input(), "Roygbiv");
        assert_eq!(app.caret, 7, "the caret starts behind the value");

        // The whole reason this playlist needs an edit at all.
        app.swap_fields();
        assert_eq!(app.answers(), ["Boards Of Canada", "Roygbiv"]);
        assert_eq!(app.caret, 16, "the caret outran the value it moved to");

        // Tab wraps, and every editing key follows the focus.
        app.next_field(true);
        assert_eq!(app.field, 1);
        app.input_clear();
        assert_eq!(app.answers(), ["Boards Of Canada", ""], "^u cleared both boxes");
        for c in "Roygbiv".chars() {
            app.input_insert(c);
        }
        app.next_field(true);
        assert_eq!(app.field, 0, "tab did not wrap");
        app.next_field(false);
        assert_eq!(app.field, 1);

        assert_eq!(app.answers(), ["Boards Of Canada", "Roygbiv"]);
        app.close_fields();
        assert_eq!(app.answers(), Vec::<String>::new());
        assert_eq!(app.caret, 0);
    }

    /* `^s` is the commonest correction in the tool, and the edit form grew
       album and year after the two boxes it swaps: by position it would have
       gone on working only by accident, and only until somebody reordered
       them. `tag::manual` still asks two, so both shapes have to work. */
    #[test]
    fn the_swap_key_finds_its_two_boxes_by_name() {
        let form = |fields: Vec<(&str, &str)>| {
            let mut app = App::new(std::sync::mpsc::channel().0, String::new());
            app.apply(Msg::Ask(
                Prompt::Form {
                    header: "track 4".into(),
                    note: String::new(),
                    fields: fields
                        .into_iter()
                        .map(|(l, v)| (l.to_string(), v.to_string()))
                        .collect(),
                    escape: Escape::Skip,
                },
                std::sync::mpsc::channel().0,
            ));
            app
        };

        let mut wide = form(vec![
            ("artist", "Roygbiv"),
            ("title", "Boards Of Canada"),
            ("album", "Music Has the Right to Children"),
            ("year", "1998"),
        ]);
        wide.swap_fields();
        assert_eq!(
            wide.answers(),
            [
                "Boards Of Canada",
                "Roygbiv",
                "Music Has the Right to Children",
                "1998"
            ],
            "the swap reached a box it was not aimed at"
        );

        // The two-box form `tag::manual` asks is the same key.
        let mut pair = form(vec![("artist", "Roygbiv"), ("title", "Boards Of Canada")]);
        pair.swap_fields();
        assert_eq!(pair.answers(), ["Boards Of Canada", "Roygbiv"]);

        /* Named and not positional, which is the whole claim: artist and
           title sit at 0 and 1 in both forms above, so those two pass either
           way and cannot say which rule is in force. */
        let mut moved = form(vec![
            ("album", "Music Has the Right to Children"),
            ("artist", "Roygbiv"),
            ("title", "Boards Of Canada"),
        ]);
        moved.swap_fields();
        assert_eq!(
            moved.answers(),
            [
                "Music Has the Right to Children",
                "Boards Of Canada",
                "Roygbiv"
            ]
        );

        /* A form with neither box has nothing to swap, and the hint that
           advertises the key reads the same answer. */
        let mut other = form(vec![("url", "https://example.com")]);
        other.swap_fields();
        assert_eq!(other.answers(), ["https://example.com"]);
    }

    /* With no offset of its own, ratatui scrolls the least it can to make
       the selection visible, so the cursor sat on the last row for the whole
       of a long list and you moved down through rows you could not see. */
    #[test]
    fn the_list_keeps_rows_ahead_of_the_cursor() {
        let height = 10;
        let len = 100;
        let at = |row: usize, from: usize| scroll_to(from, row, len, height);

        // Near the top there is nothing to scroll away from.
        assert_eq!(at(0, 0), 0);
        assert_eq!(at(5, 0), 0);
        // Two rows from the bottom edge is where it starts moving.
        assert_eq!(at(7, 0), 0, "it moved before the cursor reached the edge");
        assert_eq!(at(8, 0), 1);
        assert_eq!(at(9, 0), 2);
        // Coming back up, it holds until the cursor nears the other edge.
        assert_eq!(at(5, 3), 3, "the rows moved while the cursor had room");
        assert_eq!(at(4, 3), 2);

        // Neither end runs past the list.
        assert_eq!(at(99, 50), len - height);
        assert_eq!(at(0, 90), 0);
        // A list that fits needs no offset at all.
        assert_eq!(scroll_to(4, 3, 6, 10), 0);
        // A pane too short for the padding still has to put the cursor on it.
        assert_eq!(scroll_to(0, 40, len, 1), 40);
        assert_eq!(scroll_to(0, 0, len, 0), 0);
    }

    /* Four seconds fitted "saved" and not a wrapped error, and a second
       message used to overwrite the first inside one blink. */
    #[test]
    fn a_message_is_shown_for_as_long_as_it_takes_to_read() {
        let mut app = App::new(std::sync::mpsc::channel().0, String::new());
        app.apply(Msg::Stage("downloading".into()));

        app.say("saved");
        let short = app.flash_until.unwrap();
        assert_eq!(app.stage, "saved");

        // A queued message waits rather than replacing the one being read.
        app.say("cliamp: playlist import failed, the name is already taken");
        assert_eq!(app.stage, "saved", "the second message cut the first short");
        assert_eq!(app.flash_until, Some(short));

        app.flash_until = Some(Instant::now());
        app.expire_flash();
        assert!(app.stage.starts_with("cliamp:"), "{:?}", app.stage);
        let long = app.flash_until.unwrap();
        assert!(long > short, "a longer message got no longer to be read in");

        // And the run's own message outranks anything still queued.
        app.say("one");
        app.apply(Msg::Stage("identifying".into()));
        assert_eq!(app.stage, "identifying");
        app.flash_until = Some(Instant::now());
        app.expire_flash();
        assert_eq!(app.stage, "identifying", "a stale outcome came back");
    }

    /// The queue is a courtesy, not a backlog nobody is still reading.
    #[test]
    fn a_run_cannot_build_a_queue_of_stale_outcomes() {
        let mut app = App::new(std::sync::mpsc::channel().0, String::new());
        app.say("first");
        for n in 0..20 {
            app.say(format!("outcome {n}"));
        }
        for _ in 0..QUEUE {
            app.flash_until = Some(Instant::now());
            app.expire_flash();
            assert!(app.flash_until.is_some(), "the queue drained early");
        }
        app.flash_until = Some(Instant::now());
        app.expire_flash();
        assert_eq!(app.flash_until, None, "the queue outlasted its depth");
    }

    /* The last key in the tool that can still lose work in one press. A
       second `q` answers it, so meaning it costs nothing. */
    #[test]
    fn quitting_asks_first_only_while_something_is_running() {
        let idle = || {
            let mut app = app_with(&[Status::Ok]);
            app.done = Some(Ok(String::new()));
            app
        };

        let mut app = idle();
        app.ask_quit();
        assert!(app.quit, "a finished run made quitting a two-step");
        assert_eq!(app.confirm, None);

        let mut app = app_with(&[Status::Downloading, Status::Ok]);
        app.ask_quit();
        assert!(!app.quit, "q took the download with it");
        assert_eq!(app.confirm, Some(Confirm::Quit));
        // What the question has to report, or it cannot say what is at stake.
        assert_eq!(app.progress(), (1, 2));

        // A command in flight counts too: a retry is downloading as well.
        let mut app = idle();
        app.busy = true;
        app.ask_quit();
        assert_eq!(app.confirm, Some(Confirm::Quit));
    }

    /* A hundred-track playlist is a hundred presses of `j` without this, and
       the page has to be the list's own height or it scrolls past what the
       user was reading. */
    #[test]
    fn paging_moves_by_the_height_the_list_drew_with() {
        let mut app = app_with(&[Status::Ok; 30]);
        app.viewport = 10;

        app.page(true);
        assert_eq!(app.cursor, 10);
        app.page(true);
        assert_eq!(app.cursor, 20);
        // Clamped at the end rather than wrapping or running off.
        app.page(true);
        app.page(true);
        assert_eq!(app.cursor, 29);
        app.page(false);
        assert_eq!(app.cursor, 19);
        for _ in 0..5 {
            app.page(false);
        }
        assert_eq!(app.cursor, 0);

        // It moves over the filter's rows, not over the tracks behind them.
        app.tracks[3].name = "Autobahn".into();
        app.tracks[25].name = "Autobahn".into();
        app.filter = "autobahn".into();
        app.snap();
        assert_eq!(app.cursor, 3);
        app.page(true);
        assert_eq!(app.cursor, 25, "paging reached a row the filter hides");

        // And it stops following, like every other cursor move by hand.
        app.follow = true;
        app.page(false);
        assert!(!app.follow);
    }

    /* Every edit is relative to the caret, and a title with an accent in it
       is what turns a byte index into a panic. */
    #[test]
    fn the_prompt_edits_where_the_caret_is() {
        let mut app = App::new(std::sync::mpsc::channel().0, String::new());
        app.apply(Msg::Ask(
            Prompt::Input {
                note: String::new(),
                header: "Title".into(),
                value: "Café del Mar".into(),
                escape: Escape::Skip,
            },
            std::sync::mpsc::channel().0,
        ));
        assert_eq!(app.caret, 12, "the caret starts behind the prefilled text");

        for _ in 0..3 {
            app.input_move(false);
        }
        app.input_insert('X');
        assert_eq!(app.input(), "Café del XMar");
        assert_eq!(app.input_parts(), ("Café del X", "Mar"));

        app.input_backspace();
        assert_eq!(app.input(), "Café del Mar", "backspace took the wrong side");
        app.input_delete();
        assert_eq!(app.input(), "Café del ar", "delete took the wrong side");

        // ^w cuts the word in front of the caret, not the end of the line.
        app.input_end(false);
        app.input_move(true);
        app.input_move(true);
        app.input_move(true);
        app.input_move(true);
        app.input_kill_word();
        assert_eq!(app.input(), " del ar");
        assert_eq!(app.caret, 0);

        // Both ends are clamped, so neither key can walk off the string.
        app.input_move(false);
        assert_eq!(app.caret, 0);
        app.input_end(true);
        app.input_move(true);
        assert_eq!(app.caret, app.input().chars().count());
    }

    /* Esc goes back a step or reports that there is none. It used to quit
       whenever there was no library behind the run, which is the state every
       first run is in from start to finish. */
    #[test]
    fn escape_never_ends_the_session() {
        let finished = || {
            let mut app = App::new(std::sync::mpsc::channel().0, String::new());
            app.done = Some(Ok(String::new()));
            app
        };

        let mut app = finished();
        app.leave_tracks();
        assert!(!app.quit, "Esc quit with no library to go back to");
        assert!(app.stage.contains("q quits"), "{:?}", app.stage);

        let mut app = finished();
        app.library = vec![shelf("Focus")];
        app.leave_tracks();
        assert_eq!(app.view, View::Library);
        assert!(!app.quit);

        // The one that cost a download: a run still going, Esc pressed at it.
        let mut app = app_with(&[Status::Downloading]);
        app.library = vec![shelf("Focus")];
        app.leave_tracks();
        assert!(!app.quit, "Esc quit in the middle of a run");
        assert_eq!(app.view, View::Tracks, "Esc left a run that was still going");

        let mut app = finished();
        app.library = vec![shelf("Focus")];
        app.busy = true;
        app.leave_tracks();
        assert_eq!(app.view, View::Tracks, "Esc left a command mid-flight");
    }

    /* Esc unwinds one step at a time: the filter first, since it hides the
       list, then the marks, which survive a cleared filter, then the library.
       Before the middle step the dots had no key that removed them. */
    #[test]
    fn escape_unwinds_the_filter_then_the_marks_then_the_library() {
        let mut app = app_with(&[Status::Ok, Status::Ok, Status::Ok]);
        app.done = Some(Ok(String::new()));
        app.library = vec![shelf("Focus")];
        app.filter = "ok".to_string();
        app.marked.insert(1);
        app.marked.insert(2);

        app.escape_tracks();
        assert!(app.filter.is_empty(), "Esc left the filter up");
        assert_eq!(app.marked.len(), 2, "clearing the filter took the marks");
        assert_eq!(app.view, View::Tracks);

        app.escape_tracks();
        assert!(app.marked.is_empty(), "Esc left the marks on");
        assert!(app.stage.contains("unmarked 2 tracks"), "{:?}", app.stage);
        assert_eq!(app.view, View::Tracks, "Esc left the screen while unmarking");

        app.escape_tracks();
        assert_eq!(app.view, View::Library);
    }

    #[test]
    fn clearing_one_mark_says_track_not_tracks() {
        let mut app = app_with(&[Status::Ok]);
        app.done = Some(Ok(String::new()));
        app.marked.insert(1);

        app.escape_tracks();
        assert!(app.marked.is_empty());
        assert!(app.stage.contains("unmarked 1 track"), "{:?}", app.stage);
        assert!(!app.stage.contains("tracks"), "{:?}", app.stage);
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
        // `TALLY` is the one list of every variant; a copy of it here was
        // already a version behind.
        let settled: Vec<Status> = TALLY.into_iter().filter(|s| s.settled()).collect();

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

    /* A settled status left out of the tally was counted in the header's
       total and missing from the row beside it, with nothing saying why.
       `tally()` is exhaustive so it cannot be forgotten, and this is what
       pins the other half: that the two agree about which are settled. */
    #[test]
    fn every_settled_status_has_a_column_of_its_own() {
        let mut seen: Vec<u8> = Vec::new();
        for status in TALLY {
            match (status.settled(), status.tally()) {
                (true, Some(rank)) => seen.push(rank),
                (true, None) => panic!("{status:?} is settled and has no tally column"),
                (false, Some(_)) => panic!("{status:?} is in flight and has a tally column"),
                (false, None) => {}
            }
        }
        let mut unique = seen.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), seen.len(), "two statuses share a tally column");
    }

    /* A terminal status left out of `settled()` means the run never reaches
       its own count and the UI waits forever. The match below has no
       wildcard, so a new variant fails to compile here first. */
    #[test]
    fn every_status_is_either_in_flight_or_settled() {
        // `TALLY` rather than a second list: two hand-kept arrays of every
        // variant is one more than can be kept in step.
        for status in TALLY {
            let in_flight = match status {
                Status::Pending
                | Status::Downloading
                | Status::Downloaded
                | Status::Tagging
                | Status::Converting => true,
                Status::Have
                | Status::Ok
                | Status::Manual
                | Status::Kept
                | Status::Weak
                | Status::NoMatch
                | Status::Failed
                | Status::Gone
                | Status::Skipped
                | Status::Local => false,
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
