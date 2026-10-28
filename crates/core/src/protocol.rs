//! What the worker and a front end say to each other, and the data they say it about. Nothing here
//! draws or holds screen state, so the worker and every other non-UI module import from here and
//! never from `app`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

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
#[cfg(any(test, feature = "test-support"))]
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
    /// The recording's ISRC, from the lookup that tagged it. It is what lets a
    /// second video of the same song be recognised after the fact.
    pub isrc: Option<String>,
    /// Another copy of this song that earworm has noticed, said in words for
    /// the row. A field and not a `Status`: a duplicate is not a tagging
    /// outcome, and a status has to pass `settled()` and `tally()`.
    pub twin: Option<String>,
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
            isrc: None,
            twin: None,
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
    /// Whether a custom list started from here is what cliamp is on: its row, not the source folder, is marked.
    // cliamp reports a track and never a playlist, so this holds only while the track sits in one of the
    // list's folders. Switching playlists inside cliamp is not visible from here.
    pub fn list_on_air(&self, on_air: Option<&(String, Vec<PathBuf>)>, name: &str) -> bool {
        on_air.is_some_and(|(n, folders)| n == name && folders.iter().any(|f| self.on(f)))
    }

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

/// One track of a custom playlist as the screen shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistRow {
    /// What every command on the row carries, never its position: the order changes under a move.
    pub id: String,
    pub label: String,
    /// The folder it sits in, and the file in it, for the jump to the source.
    pub folder: String,
    pub file: String,
    /// `None` when no folder holds it now, which the row shows as `!`.
    pub path: Option<PathBuf>,
}

/// One artist on the artists screen, as the worker counted them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtistRow {
    pub name: String,
    pub tracks: usize,
    pub albums: usize,
    pub starred: bool,
    /// Where their tracks are, for Enter to narrow the library to.
    pub folders: Vec<PathBuf>,
}

/// One release on an artist's page, marked with how much of it the library holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlbumRow {
    pub id: u64,
    pub title: String,
    pub year: Option<u16>,
    pub kind: String,
    /// Tracks of it here; none is missing.
    pub have: usize,
    /// Its length on Deezer, asked only for one held, so `have < of` is partial.
    pub of: Option<usize>,
    /// Out since the artist's page was last read, from the startup check.
    pub new: bool,
}

impl AlbumRow {
    /// Known to be whole. A held release whose length never came is not: it may be two tracks of twelve, and
    /// the pick gate leaves what is already here unmarked anyway.
    pub fn complete(&self) -> bool {
        self.have > 0 && self.of.is_some_and(|of| self.have >= of)
    }
}

/// One of the artist's top tracks, and whether a track of that title is here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopRow {
    pub title: String,
    pub album: String,
    pub have: bool,
}

/// An artist Deezer calls related, with how much of them the library holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelatedRow {
    pub name: String,
    pub fans: u64,
    pub tracks: usize,
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
    /// Open a library row, then `SyncOne` it.
    Sync(PathBuf),
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
    /// Turn one of `config::SWITCHES` on or off, and remember it.
    Switch(&'static str, bool),
    Icons(crate::icons::Icons),
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
    /// Ask for a name and make an empty custom playlist.
    NewPlaylist,
    /// Put tracks into a custom playlist, chosen from the list or newly named. A folder and a filename each,
    /// and never a row position: the worker works out the ids, and adopts a folder that has none.
    // The picker's answer for an open list: what to add and what to take off, by folder and filename.
    EditPlaylist { name: String, add: Vec<(PathBuf, String)>, remove: Vec<(PathBuf, String)> },
    // With a list named, the chooser is skipped and the worker opens that list afterwards.
    AddToPlaylist(Vec<(PathBuf, String)>, Option<String>),
    /// Open a custom playlist's screen, by name as `Open` is by folder.
    OpenPlaylist(String),
    /// Move one entry a place. By id, because the rows move under the cursor.
    MoveEntry { name: String, id: String, up: bool },
    RemoveEntry { name: String, id: String },
    RenamePlaylist(String),
    /// The list and never the audio: the worker confirms first, and the tracks stay in their folders.
    DeletePlaylist(String),
    PlayPlaylist(String),
    Export(crate::export::Target),
    /// The artists screen's rows, read from the tags.
    Artists,
    /// Star an artist, or unstar one already starred. By name: the rows reorder when a star moves one.
    Star(String),
    /// An artist's Deezer releases, marked against the library.
    Discography(String),
    /// Find a release on YouTube Music and download it as a new playlist. By title, as Deezer names it.
    GetAlbum { artist: String, title: String },
    /// The artist's top tracks, marked against the library.
    Top(String),
    Related(String),
    /// Find the artist's missing top tracks on YouTube Music and download them as one playlist.
    GetTop { artist: String, titles: Vec<String> },
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
    /// The custom list just handed to cliamp, or `None` when a folder was.
    OnAir(Option<(String, Vec<PathBuf>)>),
    Tracks(Vec<Track>),
    Progress {
        index: usize,
        percent: u16,
    },
    /// Says a track has a twin, or clears it.
    Twin {
        index: usize,
        twin: Option<String>,
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
    /// A custom playlist's rows. `show` opens the screen, and a refresh after an edit does not: the user
    /// may have left it. `focus` is the id to land on, for a move that has to keep the cursor on its entry.
    Custom { name: String, rows: Vec<PlaylistRow>, show: bool, focus: Option<String> },
    /// The open list's new order after a move or a removal. The screen already holds the labels, so nothing
    /// is read from disk; an id it does not hold means the list changed behind its back and is ignored.
    Order { name: String, ids: Vec<String>, focus: Option<String> },
    /// Puts the library cursor on this row, for a list just made or renamed: the sort moves it to a new place.
    Select(PathBuf),
    /// A search's marked tracks were added to a list, so they are not marks any more.
    FoundUnmark,
    /// The artists screen's rows. `focus` is the artist to land on, since starring moves a row.
    Artists { rows: Vec<ArtistRow>, focus: Option<String> },
    /// An artist page's releases, or why there are none. `name` is the page it was asked for, so an answer
    /// for a page already left is dropped.
    Discography { name: String, found: Option<(String, u64)>, rows: Result<Vec<AlbumRow>, String> },
    /// Tracks the next pick gate opens unmarked: what a partial album already has. Sent just before `Pick`,
    /// so the marks are right from the first frame the gate is open.
    PickHeld(Vec<usize>),
    Top { name: String, rows: Result<Vec<TopRow>, String> },
    Related { name: String, rows: Result<Vec<RelatedRow>, String> },
    /// Starred artists' releases not seen before, as (artist, release id, title), newest first.
    NewReleases(Vec<(String, u64, String)>),
    /// One release's length, sent after the page as each request lands.
    Counted { name: String, id: u64, of: usize },
    /// A list was just made: open the track picker with it as the target.
    PickTracks(String),
    /// A list changed its name, so the one cliamp is on keeps its marker.
    Renamed { old: String, new: String },
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
        rows: Vec<crate::config::Row>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use super::test_support::at;

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
}

// For the front ends' tests too, which is why it is not inside `tests`.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::{Playback, Player};
    use std::path::PathBuf;

    pub fn at(player: Player, folder: Option<&str>, playing: bool) -> Playback {
        Playback {
            player,
            folder: folder.map(PathBuf::from),
            track: None,
            playing,
        }
    }
}
