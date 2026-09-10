use std::path::PathBuf;
use std::sync::mpsc::Sender;

use ratatui::style::Color;

use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
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
            Status::Pending | Status::Have | Status::Kept => theme::DIM,
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
}

pub enum Prompt {
    Choice {
        header: String,
        options: Vec<String>,
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
    Skip,
    Quit,
}

pub enum Reply {
    Choice(usize),
    Text(String),
    Cancel,
}

pub enum Msg {
    Stage(String),
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
    /// Nothing to report and nothing to look at, so close the UI outright.
    Quit,
}

/// Prompts are answered by the UI thread, so a question blocks only the worker.
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
        if !self.enabled {
            return None;
        }
        match self.ask(Prompt::Choice {
            header: header.into(),
            options,
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
    pub playlist: String,
    pub tracks: Vec<Track>,
    pub logs: Vec<String>,
    pub cursor: usize,
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
            playlist: String::new(),
            tracks: Vec::new(),
            logs: Vec::new(),
            cursor: 0,
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
            Msg::Stage(s) => self.stage = s,
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
                self.done = Some(result);
            }
            Msg::Quit => self.quit = true,
        }
    }

    /// Commands are only offered once the pipeline is done: the worker is busy
    /// until then, and a queued edit would look like nothing happened.
    pub fn can_command(&self) -> bool {
        self.done.is_some() && self.prompt.is_none() && !self.tracks.is_empty()
    }

    pub fn send(&self, cmd: Cmd) {
        if let Some(tx) = &self.cmds {
            let _ = tx.send(cmd);
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
