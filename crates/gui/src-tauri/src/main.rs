//! The desktop front end: a Tauri window around the same worker the terminal runs. Every `Msg` goes to the
//! page as JSON on one event, the page answers through `send` and `answer`, and the list rules are core's
//! `view` functions called over IPC, so the two front ends cannot disagree about what a list shows.

// No console window behind the app in a Windows release build.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::{CommandFactory, FromArgMatches};
use earworm_core::cheats::{self, Feature, Unlocked};
use earworm_core::config::{self, Cli, Config};
use earworm_core::manifest::Kind;
use earworm_core::achievements::{self, Achievement, Facts};
use earworm_core::picker::{self, Hit, Marks};
use earworm_core::protocol::{self, AlbumRow, ArtistRow, Cmd, Msg, Playback, Reply, Shelf, Status, Track};
use earworm_core::theme::{blend, Palette, Themes};
use earworm_core::view::{self, Narrow, ShelfNarrow, Sort};
use earworm_core::{deps, lookup, tag, tools, update, worker, ytdlp};
use ratatui_core::style::Color;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, RunEvent, State};

/// The event every `Msg` goes out on, numbered as a `Sent`.
const MSG: &str = "earworm://msg";
/// Sent once the worker has dropped its sender, so the page can tell a finished worker from a quiet one.
const GONE: &str = "earworm://gone";

struct Look {
    themes: Themes,
    name: String,
    palette: Palette,
}

/// One message as the page receives it. `seq` lets a page that was handed the history drop the live
/// copies of messages the history already held.
#[derive(Clone, Serialize)]
struct Sent {
    seq: u64,
    msg: serde_json::Value,
}

/// What the first screens say about the setup before the worker has said anything.
#[derive(Clone, Default, Serialize)]
struct Setup {
    /// The header's right half, as the terminal's describes the run.
    text: String,
    format: String,
    rows: Vec<config::Row>,
    brief: Vec<(String, String)>,
    /// Where playlists land, with the home folder as `~`.
    dir: String,
    key: bool,
}

struct Shell {
    setup: Setup,
    // Taken by `start`, so the worker starts once the page is listening and its first messages reach it.
    config: Mutex<Option<Result<Config, String>>>,
    /// Every message sent, so a page that reloads rebuilds its state by applying them again.
    history: Arc<Mutex<Vec<Sent>>>,
    /// Questions the page has answered, which a replay must not ask again.
    answered: Mutex<HashSet<u64>>,
    cmds: Mutex<Option<Sender<Cmd>>>,
    cancel: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    look: Mutex<Look>,
    config_file: Option<PathBuf>,
    overridden: bool,
    unlocked: Arc<Unlocked>,
    /// The achievement state file and what it holds, or `None` when achievements are off for the session.
    achieved: Mutex<Option<(PathBuf, std::collections::BTreeSet<String>)>>,
    update_check: bool,
    /// Whether `latest` has answered, so a reloaded page is not told twice.
    told: AtomicBool,
}

/// The palette as the page's CSS variables, with the walk's names for the picker.
#[derive(Serialize)]
struct Theme {
    name: String,
    /// Whether the ground is light, for the page's form controls and scrollbars.
    light: bool,
    names: Vec<String>,
    vars: BTreeMap<&'static str, String>,
    /// What became of the switch, for the page to flash.
    said: Option<String>,
}

fn hex(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02X}{g:02X}{b:02X}"),
        // Every palette is built from RGB; a named colour has no fixed value to hand a stylesheet.
        other => panic!("a palette slot is not RGB: {other:?}"),
    }
}

// Read as WCAG luminance against black and white rather than by name, so a `[themes.*]` light theme counts.
fn light(p: &Palette) -> bool {
    earworm_core::theme::contrast(p.near, (0, 0, 0)) > earworm_core::theme::contrast(p.near, (255, 255, 255))
}

fn rgba((r, g, b): (u8, u8, u8), alpha: f32) -> String {
    format!("rgba({r}, {g}, {b}, {alpha})")
}

// Every slot by its role name, which is also the stylesheets' variable name, plus the gradient's ends and
// the tones look.css derives from them. Mixed here rather than with `color-mix()`, which WebKit lacks before
// 16.2, and a Mac on 11 or 12 without a Safari update has an older one.
fn vars(p: &Palette) -> BTreeMap<&'static str, String> {
    let mut vars: BTreeMap<&'static str, String> = p.slots().into_iter().map(|(name, c)| (name, hex(c))).collect();
    let rgb = |c: Color| match c {
        Color::Rgb(r, g, b) => (r, g, b),
        other => panic!("a palette slot is not RGB: {other:?}"),
    };
    let tone = |(r, g, b): (u8, u8, u8)| hex(Color::Rgb(r, g, b));
    vars.insert("near", tone(p.near));
    vars.insert("far", tone(p.far));
    vars.insert("stop", format!("{:.0}%", p.stop * 100.0));
    vars.insert("card2", tone(p.card2().expect("surface is RGB, as hex requires of every slot")));
    vars.insert("glass", rgba(rgb(p.surface), 0.82));
    vars.insert("accent2", tone(blend(rgb(p.accent), p.far, 0.35)));
    vars.insert("glow", rgba(rgb(p.accent), 0.22));
    vars.insert("scrim", rgba(p.far, 0.78));
    vars
}

impl Shell {
    fn theme(&self, said: Option<String>) -> Theme {
        let look = self.look.lock().unwrap();
        Theme {
            name: look.name.clone(),
            light: light(&look.palette),
            names: look.themes.list().into_iter().map(String::from).collect(),
            vars: vars(&look.palette),
            said,
        }
    }

    // Saved the way the terminal's `^t` saves, through `config::keep_theme`.
    fn switch(&self, name: &str) -> Theme {
        {
            let mut look = self.look.lock().unwrap();
            let Some(palette) = look.themes.named(name) else {
                drop(look);
                return self.theme(Some(format!("no theme called {name}")));
            };
            look.palette = palette;
            look.name = name.to_string();
        }
        let said = config::keep_theme(self.config_file.as_deref(), name).said(name, self.overridden);
        self.theme(Some(said))
    }
}

/// What the page needs from core before the first message: the palette, and each status's word and rules,
/// so the page never keeps its own copy of either.
#[derive(Serialize)]
struct Boot {
    theme: Theme,
    statuses: Vec<Word>,
    setup: Setup,
    /// Everything the worker has sent so far, minus questions already answered. Empty on a first start.
    replay: Vec<Sent>,
    /// The YouTube Music unlock is on, which is what lets the artist page offer a download at all.
    music: bool,
}

#[derive(Serialize)]
struct Word {
    status: Status,
    label: &'static str,
    /// Nothing confirmed it: what the review keeps and `n` walks.
    look: bool,
    settled: bool,
    /// Past its download, and being worked on now: the run screen's gauge and the bar's current track.
    fetched: bool,
    in_hand: bool,
    /// Its column in the tally, or `None` while it is still in flight.
    tally: Option<u8>,
    /// The palette slot it is drawn in, which is also the CSS variable's name.
    slot: &'static str,
}

fn words() -> Vec<Word> {
    protocol::TALLY
        .iter()
        .map(|s| Word { status: *s, label: s.label(), look: s.wants_a_look(), settled: s.settled(), fetched: s.fetched(), in_hand: s.in_hand(), tally: s.tally(), slot: earworm_core::theme::status_slot(*s) })
        .collect()
}

/// Starts the worker on the first call. A later one is the page reloading: the worker carries on, and the
/// page gets the history to rebuild from, open questions included, so nothing the worker waits on is lost.
// Cancelling instead would answer the first URL prompt with Esc, which quits the worker.
#[tauri::command]
fn start(app: tauri::AppHandle, shell: State<Shell>) -> Result<Boot, String> {
    let mut slot = shell.config.lock().unwrap();
    match slot.take() {
        None => {}
        Some(Err(e)) => {
            *slot = Some(Err(e.clone()));
            return Err(e);
        }
        Some(Ok(cfg)) => {
            let (tx, rx) = mpsc::channel::<Msg>();
            let (cmd_tx, cmd_rx) = mpsc::channel();
            *shell.cmds.lock().unwrap() = Some(cmd_tx);
            let (cancel, stop) = (Arc::clone(&shell.cancel), Arc::clone(&shell.stop));
            std::thread::spawn(move || worker::run(cfg, tx, cancel, stop, cmd_rx));
            let history = Arc::clone(&shell.history);
            std::thread::spawn(move || {
                for msg in rx {
                    let Ok(msg) = serde_json::to_value(&msg) else { continue };
                    // Numbered and emitted under one lock, so a replay never overlaps the live stream out of order.
                    let mut history = history.lock().unwrap();
                    let sent = Sent { seq: history.len() as u64, msg };
                    let _ = app.emit(MSG, &sent);
                    history.push(sent);
                }
                let _ = app.emit(GONE, ());
            });
        }
    }
    drop(slot);
    let replay = replayable(&shell.history.lock().unwrap(), &shell.answered.lock().unwrap());
    let music = shell.unlocked.has(Feature::Music);
    Ok(Boot { theme: shell.theme(None), statuses: words(), setup: shell.setup.clone(), replay, music })
}

// A question shows up again on replay only while nobody has answered it.
fn replayable(history: &[Sent], answered: &HashSet<u64>) -> Vec<Sent> {
    let asked = |msg: &serde_json::Value| ["Ask", "Pick"].iter().find_map(|k| msg.get(k)?.get("id")?.as_u64());
    history.iter().filter(|s| asked(&s.msg).is_none_or(|id| !answered.contains(&id))).cloned().collect()
}

/// One row of `--check`'s tools table, for the first screen.
#[derive(Serialize)]
struct Tool {
    bin: &'static str,
    what: &'static str,
    required: bool,
    version: Option<String>,
    at: Option<String>,
    /// What to run to get it: the Homebrew line, or the `[tools]` path that is not there.
    fix: String,
}

fn probe_tools() -> Vec<Tool> {
    deps::probe_all()
        .into_iter()
        .map(|f| Tool {
            bin: f.dep.bin,
            what: f.dep.what,
            required: f.dep.required,
            fix: match &f.configured {
                Some(path) if f.at.is_none() => format!("not at {path} · check [tools] in the config"),
                _ => format!("brew install {}", f.dep.formula),
            },
            version: f.version,
            at: f.at,
        })
        .collect()
}

// Off the main thread: each tool gets up to three seconds to say its version.
#[tauri::command]
async fn tools() -> Vec<Tool> {
    tauri::async_runtime::spawn_blocking(probe_tools).await.unwrap_or_default()
}

// Off the main thread, since a stale cache asks GitHub. The app has no updater, as an unsigned build cannot
// verify one, so the news points at the release page rather than at cargo.
#[tauri::command]
async fn latest(shell: State<'_, Shell>) -> Result<Option<String>, ()> {
    if !first_ask(shell.update_check, &shell.told) {
        return Ok(None);
    }
    let latest = tauri::async_runtime::spawn_blocking(update::available).await.ok().flatten();
    Ok(latest.map(|v| format!("earworm v{v} is out at {}", update::RELEASES_PAGE)))
}

fn first_ask(on: bool, told: &AtomicBool) -> bool {
    on && !told.swap(true, Ordering::SeqCst)
}

/// Ends the app the way the terminal's `q` does; `shut_down` runs on the way out.
#[tauri::command]
fn quit(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn send(cmd: Cmd, shell: State<Shell>) -> bool {
    arm(&cmd, &shell.stop);
    shell.cmds.lock().unwrap().as_ref().is_some_and(|tx| tx.send(cmd).is_ok())
}

// A search clears the stop before it is sent, as the terminal's does, so an Esc cannot land between the
// two and a stop from an earlier search does not end this one.
fn arm(cmd: &Cmd, stop: &AtomicBool) {
    if matches!(cmd, Cmd::GetAlbum { .. } | Cmd::GetTop { .. }) {
        stop.store(false, Ordering::SeqCst);
    }
}

#[tauri::command]
fn answer(id: u64, reply: Reply, shell: State<Shell>) -> bool {
    shell.answered.lock().unwrap().insert(id);
    protocol::answer(id, reply)
}

/// Esc on a running search, as the terminal's: the worker checks it between searches.
#[tauri::command]
fn stop(shell: State<Shell>) {
    shell.stop.store(true, Ordering::SeqCst);
}

/// Who `*` stars for a track's artist credit: the lead, never a featured guest.
#[tauri::command]
fn lead_artist(artist: String) -> String {
    lookup::lead_artist(&artist)
}

/// The icon set after this one, for the settings row.
#[tauri::command]
fn next_icons(icons: earworm_core::icons::Icons) -> earworm_core::icons::Icons {
    icons.next()
}

/// The command-line flag that holds a switch, for saying why the settings screen cannot change it.
#[tauri::command]
fn switch_flag(key: String) -> String {
    config::SWITCHES.iter().find(|s| s.key == key).map_or(key.clone(), |s| s.flag.to_string()).replace('_', "-")
}

/// The AcoustID key typed on the first screen: saved, then the app starts again, since the worker read the
/// config once at its start and the run about to follow is the one the key is for.
#[tauri::command]
fn acoustid_key(key: String, app: tauri::AppHandle, shell: State<Shell>) -> Result<(), String> {
    let file = shell.config_file.as_deref().ok_or("--no-config keeps the config file out of this run")?;
    config::save_acoustid_key(file, &key).map_err(|e| e.to_string())?;
    app.restart()
}

#[tauri::command]
fn theme_next(shell: State<Shell>) -> Theme {
    let name = {
        let look = shell.look.lock().unwrap();
        look.themes.after(&look.name).0.to_string()
    };
    shell.switch(&name)
}

#[derive(Serialize)]
struct Unlock {
    said: String,
    /// Set when the code was a secret palette, which is switched to at once so the celebration is in it.
    theme: Option<Theme>,
    /// The YouTube Music unlock is on after this code, whichever code it was.
    music: bool,
    /// Set only by the call that turned a feature on, which is what gets a card rather than a flash.
    found: Option<Found>,
}

/// What the unlock card shows. `art` picks the card's animation; the words are core's `prize`.
#[derive(Serialize)]
struct Found {
    art: &'static str,
    prize: &'static str,
    note: &'static str,
}

impl Found {
    // Not `Feature::note`: a secret theme is switched to here at once, so the terminal's "^t walks to it" is wrong.
    fn of(feature: Feature) -> Found {
        let (art, note) = match feature {
            Feature::Music => ("music", feature.note()),
            Feature::Vaporwave => ("vaporwave", "on now  ·  this session only"),
            Feature::GameBoy => ("gameboy", "on now  ·  this session only"),
        };
        Found { art, prize: feature.prize(), note }
    }
}

#[tauri::command]
fn cheat(code: String, shell: State<Shell>) -> Unlock {
    let Some(cheat) = cheats::find(&code) else {
        return Unlock { said: "nothing happens".into(), theme: None, music: shell.unlocked.has(Feature::Music), found: None };
    };
    if !shell.unlocked.grant(cheat.feature) {
        return Unlock { said: format!("already found  ·  {}", cheat.feature.prize()), theme: None, music: shell.unlocked.has(Feature::Music), found: None };
    }
    let theme = cheat.feature.theme().map(|name| {
        // A `[themes.*]` already holding the name keeps it: it was there first.
        {
            let mut look = shell.look.lock().unwrap();
            if let Some((_, palette)) = earworm_core::theme::SECRETS.iter().find(|(n, _)| *n == name)
                && !look.themes.has(name)
            {
                look.themes.add(name.to_string(), *palette);
            }
        }
        shell.switch(name)
    });
    Unlock { said: cheat.feature.prize().into(), theme, music: shell.unlocked.has(Feature::Music), found: Some(Found::of(cheat.feature)) }
}

#[tauri::command]
fn rows(tracks: Vec<Track>, filter: String, review: bool) -> Vec<usize> {
    view::rows(&tracks, &Narrow::new(&filter, review))
}

#[tauri::command]
fn shelf_rows(library: Vec<Shelf>, filter: String, only: Option<Kind>, folders: Option<Vec<PathBuf>>, sort: Sort) -> Vec<usize> {
    let narrow = ShelfNarrow { filter: &filter, only, folders: folders.as_deref() };
    view::shelf_rows(&library, &narrow, sort)
}

/// One picker row: the match, the track it names, and whether it is marked and whether the open list holds it.
#[derive(Serialize)]
struct PickRow {
    #[serde(flatten)]
    hit: Hit,
    folder: PathBuf,
    // Not `file`, which the flattened hit already uses for the file's place in its folder.
    name: String,
    marked: bool,
    held: bool,
}

#[derive(Serialize)]
struct Picked {
    rows: Vec<PickRow>,
    total: usize,
}

/// The picker's rows for a query, by core's matcher, each with its marks; only the marked ones under F2.
// The page keeps the marks as the ordered list and nothing else, so `Marks` and its composed-name keys stay core's.
#[tauri::command]
fn picker_rows(library: Vec<Shelf>, query: String, marks: Vec<(PathBuf, String)>, held: Vec<(PathBuf, String)>, only_marked: bool) -> Picked {
    let (marks, held) = (Marks::of(&marks), Marks::of(&held));
    let mut hits = picker::rows(&library, &query);
    if only_marked {
        hits = picker::marked_only(&library, hits, &marks);
    }
    let rows = hits
        .into_iter()
        .map(|hit| {
            let (folder, file) = picker::target(&library, &hit);
            let (marked, held) = (marks.has(folder, file), held.has(folder, file));
            PickRow { folder: folder.to_path_buf(), name: file.to_string(), marked, held, hit }
        })
        .collect();
    Picked { rows, total: picker::addable(&library) }
}

/// The marks with these tracks marked, or unmarked when every one of them already is.
#[tauri::command]
fn pick_toggle(marks: Vec<(PathBuf, String)>, items: Vec<(PathBuf, String)>) -> Vec<(PathBuf, String)> {
    let mut marks = Marks::of(&marks);
    marks.toggle_all(items.iter().map(|(f, n)| (f.as_path(), n.as_str())));
    marks.picks()
}

/// What Enter on the picker sends: to add, and, for a list that was open, to take off.
#[tauri::command]
fn pick_changes(under: Option<(PathBuf, String)>, marks: Vec<(PathBuf, String)>, held: Option<Vec<(PathBuf, String)>>) -> (Vec<(PathBuf, String)>, Vec<(PathBuf, String)>) {
    let held = held.map(|h| Marks::of(&h));
    picker::changes(under.as_ref().map(|(f, n)| (f.as_path(), n.as_str())), &Marks::of(&marks), held.as_ref())
}

/// An achievement as the toast shows it.
#[derive(Serialize)]
struct Prize {
    id: &'static str,
    label: &'static str,
    hint: &'static str,
}

/// What the session has earned and the state file does not know yet. Nothing when achievements are off,
/// or the state file could not be read: announcements that cannot be recorded would only repeat.
#[tauri::command]
fn earned(library: Vec<Shelf>, tracks: Vec<Track>, done: Option<Result<String, String>>, stage: String, playback: Playback, shell: State<Shell>) -> Vec<Prize> {
    let achieved = shell.achieved.lock().unwrap();
    let Some((_, known)) = achieved.as_ref() else {
        return Vec::new();
    };
    let facts = Facts { shelves: &library, tracks: &tracks, done: done.as_ref().map(|d| (d, stage.as_str())), playback: &playback, secrets: shell.unlocked.count() };
    achievements::earned(&facts)
        .into_iter()
        .filter(|a| !known.contains(a.id()))
        .map(|a| Prize { id: a.id(), label: a.label(), hint: a.hint() })
        .collect()
}

/// Records the achievements the page showed.
#[tauri::command]
fn shown(ids: Vec<String>, shell: State<Shell>) {
    let mut achieved = shell.achieved.lock().unwrap();
    let Some((path, known)) = achieved.as_mut() else {
        return;
    };
    let ids: Vec<&'static str> = ids.iter().filter_map(|id| Achievement::from_id(id)).map(Achievement::id).collect();
    // Known for the session at once, as the terminal's are, so a file that cannot be written does not mean
    // announcing them again; written off this thread, which is the window's.
    known.extend(ids.iter().map(|id| id.to_string()));
    let path = path.clone();
    std::thread::spawn(move || {
        let _ = achievements::merge_at(&path, &ids);
    });
}

#[derive(Serialize)]
struct Search {
    hits: Vec<Hit>,
    total: usize,
}

#[tauri::command]
fn search(library: Vec<Shelf>, query: String, only: Option<Kind>) -> Search {
    Search { hits: view::found_hits(&library, &query, only), total: view::found_total(&library, only) }
}

#[tauri::command]
fn artist_rows(rows: Vec<ArtistRow>, query: String) -> Vec<usize> {
    view::artist_rows(&rows, &query)
}

#[tauri::command]
fn release_rows(rows: Vec<AlbumRow>, all_kinds: bool) -> Vec<usize> {
    view::release_rows(&rows, all_kinds)
}

/// Which tracks a marking key takes, as the terminal's `a`, `m` and artist keys choose them.
#[derive(Deserialize)]
enum Take {
    Showing,
    Status(Status),
    /// The artist of the track with this index.
    ArtistOf(usize),
}

/// The track indices `take` reaches through the narrowing, or `None` when the track has no artist yet.
#[tauri::command]
fn take(tracks: Vec<Track>, filter: String, review: bool, take: Take) -> Option<Vec<usize>> {
    let narrow = Narrow::new(&filter, review);
    Some(match take {
        Take::Showing => view::showing(&tracks, &narrow),
        Take::Status(status) => view::like_status(&tracks, &narrow, status),
        Take::ArtistOf(index) => {
            let artist = tracks.iter().find(|t| t.index == index).and_then(view::artist_of)?;
            view::like_artist(&tracks, &narrow, &artist)
        }
    })
}

/// Marks the group, or takes it off when all of it is marked already.
#[tauri::command]
fn flip(marked: Vec<usize>, group: Vec<usize>) -> Vec<usize> {
    let mut marked = marked.into_iter().collect();
    view::flip(&mut marked, &group);
    view::picked(&marked)
}

#[tauri::command]
fn invert(marked: Vec<usize>, tracks: Vec<usize>) -> Vec<usize> {
    let mut marked = marked.into_iter().collect();
    view::invert(&mut marked, &tracks);
    view::picked(&marked)
}

#[tauri::command]
fn gate_marks(tracks: Vec<Track>, held: Vec<usize>) -> Vec<usize> {
    view::picked(&view::gate_marks(&tracks, &held))
}

/// The startup check's new releases, less the artists whose page the page has already read.
#[tauri::command]
fn unread(found: Vec<(String, u64, String)>, read: Vec<String>) -> Vec<(String, u64, String)> {
    view::unread(found, &read.into_iter().collect())
}

/// Each artist row's new releases by title, matched as core matches an artist.
#[tauri::command]
fn new_titles(rows: Vec<ArtistRow>, fresh: Vec<(String, u64, String)>) -> Vec<Vec<String>> {
    view::new_titles(&rows, &fresh)
}

/// The row for an artist under any spelling of their name.
#[tauri::command]
fn artist_at(rows: Vec<ArtistRow>, name: String) -> Option<usize> {
    view::artist_at(&rows, &name)
}

/// An artist page's releases marked new, and what is left unread after it.
#[tauri::command]
fn mark_new(name: String, mut rows: Vec<AlbumRow>, mut fresh: Vec<(String, u64, String)>, read: Vec<String>) -> (Vec<AlbumRow>, Vec<(String, u64, String)>, Vec<String>) {
    let mut read: HashSet<String> = read.into_iter().collect();
    view::mark_new(&name, &mut rows, &mut fresh, &mut read);
    (rows, fresh, read.into_iter().collect())
}

#[tauri::command]
fn counts(tracks: Vec<Track>) -> Vec<(Status, usize)> {
    view::counts(&tracks)
}

#[tauri::command]
fn next_kind(only: Option<Kind>) -> (Option<Kind>, &'static str) {
    let next = view::next_kind(only);
    (next, view::kind_label(next))
}

#[tauri::command]
fn next_sort(sort: Sort) -> (Sort, &'static str) {
    (sort.next(), sort.next().label())
}

// Debug builds only: the page reports what it drew, so a check can read the window without a screenshot.
#[cfg(debug_assertions)]
#[tauri::command]
fn probe(report: serde_json::Value) {
    eprintln!("probe {report}");
}

#[cfg(not(debug_assertions))]
#[tauri::command]
fn probe() {}

// What the terminal does on its way out: stop the subprocesses, close the worker's channel, wake a question
// it is blocked on, and give a tag write that has started a moment to finish, since quitting mid-write
// truncates the file.
fn shut_down(shell: &Shell) {
    // Before the stops below, which also end the version probes it runs.
    record(shell);
    shell.cancel.store(true, Ordering::SeqCst);
    ytdlp::stop();
    lookup::stop();
    shell.cmds.lock().unwrap().take();
    protocol::cancel_open_questions();
    let deadline = Instant::now() + Duration::from_secs(5);
    while tag::writing() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}

// Debug builds with `EARWORM_SESSION` set write the session's messages there on the way out, which is what
// the page's browser preview replays to draw each screen.
#[cfg(debug_assertions)]
fn record(shell: &Shell) {
    if let Some(path) = std::env::var_os("EARWORM_SESSION") {
        let msgs: Vec<serde_json::Value> = shell.history.lock().unwrap().iter().map(|s| s.msg.clone()).collect();
        let session = serde_json::json!({
            "theme": shell.theme(None),
            "statuses": words(),
            "setup": shell.setup,
            "tools": probe_tools(),
            "msgs": msgs,
        });
        let _ = std::fs::write(path, serde_json::to_string_pretty(&session).unwrap_or_default());
    }
}

#[cfg(not(debug_assertions))]
fn record(_: &Shell) {}

// Debug builds with `EARWORM_DRIVE` set run each line appended to that file as JavaScript in the page, so a
// check can press keys in this window alone rather than typing into whichever app has the focus.
#[cfg(debug_assertions)]
fn drive(app: &tauri::AppHandle) {
    let Some(path) = std::env::var_os("EARWORM_DRIVE") else { return };
    let app = app.clone();
    std::thread::spawn(move || {
        let mut done = 0;
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let lines: Vec<&str> = text.lines().collect();
            for line in &lines[done.min(lines.len())..] {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.eval(*line);
                }
            }
            done = lines.len();
        }
    });
}

#[cfg(not(debug_assertions))]
fn drive(_: &tauri::AppHandle) {}

fn main() {
    let matches = Cli::command().get_matches();
    let built = Cli::from_arg_matches(&matches)
        .map_err(|e| e.to_string())
        .and_then(|cli| Config::build(cli, &matches).map_err(|e| format!("{e:#}")));
    // A config that does not build still opens the window, which is where somebody launched from the
    // Finder will see why, painted in the first built-in.
    let shell = match &built {
        Ok(cfg) => {
            tools::configure(&cfg.tools);
            Shell {
                setup: Setup {
                    text: cfg.describe(),
                    format: cfg.format.clone(),
                    rows: cfg.rows(),
                    brief: cfg.brief(),
                    dir: tilde(&cfg.dir),
                    key: cfg.acoustid_key.is_some(),
                },
                look: Mutex::new(Look { themes: cfg.themes.clone(), name: cfg.theme_name.clone(), palette: cfg.theme }),
                config_file: cfg.config_file.clone(),
                overridden: cfg.theme_overridden,
                unlocked: Arc::clone(&cfg.unlocked),
                // Read once at start, as the terminal does: a file that cannot be read turns the feature off.
                achieved: Mutex::new(
                    cfg.achievements
                        .then(achievements::state_path)
                        .flatten()
                        .and_then(|path| achievements::read_at(&path).ok().map(|known| (path, known))),
                ),
                update_check: cfg.update_check,
                ..Shell::bare()
            }
        }
        Err(_) => Shell::bare(),
    };
    *shell.config.lock().unwrap() = Some(built);
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(shell)
        // Thumbnails are files on disk, and the webview may read that one folder of them.
        .setup(|app| {
            drive(app.handle());
            if let Some(cache) = tag::thumb_cache() {
                let _ = std::fs::create_dir_all(&cache);
                let _ = app.asset_protocol_scope().allow_directory(&cache, false);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start, send, answer, stop, theme_next, cheat, rows, shelf_rows, search, artist_rows,
            release_rows, take, flip, invert, gate_marks, unread, mark_new, counts, picker_rows, pick_toggle,
            pick_changes, earned, shown, next_icons, switch_flag, lead_artist, new_titles, artist_at, next_kind, next_sort, probe, tools, latest, acoustid_key, quit
        ])
        .build(tauri::generate_context!())
        .expect("the window could not be built")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                shut_down(&app.state::<Shell>());
            }
        });
}

fn tilde(path: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

impl Shell {
    fn bare() -> Shell {
        let themes = Themes::default();
        let (name, palette) = themes.after("");
        let look = Look { name: name.to_string(), palette, themes };
        Shell {
            setup: Setup::default(),
            config: Mutex::new(None),
            history: Arc::new(Mutex::new(Vec::new())),
            answered: Mutex::new(HashSet::new()),
            cmds: Mutex::new(None),
            cancel: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            look: Mutex::new(look),
            config_file: None,
            overridden: false,
            unlocked: Arc::new(Unlocked::default()),
            achieved: Mutex::new(None),
            update_check: false,
            told: AtomicBool::new(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use earworm_core::protocol::test_support::{every_cmd, every_msg};

    // The page's reducer tests run on this file, so a message the worker can send and the file lacks is
    // one the page was never tested against. `EARWORM_RECORD=1` rewrites it.
    #[test]
    fn the_recorded_messages_are_every_message_the_worker_sends() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/lib/msgs.json");
        let json = serde_json::to_string_pretty(&serde_json::json!({ "msgs": every_msg(), "cmds": every_cmd(), "statuses": words() })).unwrap() + "\n";
        if std::env::var_os("EARWORM_RECORD").is_some() {
            std::fs::write(path, &json).unwrap();
            return;
        }
        let recorded = std::fs::read_to_string(path).unwrap_or_default();
        assert!(recorded == json, "crates/gui/src/lib/msgs.json is stale: EARWORM_RECORD=1 cargo test -p earworm-gui rewrites it");
    }

    // `paint` overwrites the variables look.css's `:root` defines, so the two sets of names have to be the same:
    // a slot the stylesheet lacks is never used, and one the palette lacks keeps graphite's colour.
    #[test]
    fn the_palette_paints_exactly_the_colours_the_stylesheets_use() {
        let look = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../src/look.css")).unwrap();
        let root = look.split(":root {").nth(1).unwrap().split('}').next().unwrap();
        let used: std::collections::BTreeSet<&str> =
            root.split("--").skip(1).map(|v| v.split(':').next().unwrap().trim()).collect();
        // Every other variable look.css reads is a measure `body` sets, or an alias `.look` defines, read under
        // `.look`, where it resolves.
        let named = |s: &str| s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').count();
        let set_in = |rule: &str| -> std::collections::BTreeSet<&str> {
            let block = look.split(rule).nth(1).unwrap().split('}').next().unwrap();
            block.split("--").skip(1).map(|v| &v[..named(v)]).collect()
        };
        let (aliases, measures) = (set_in("\n.look {"), set_in("\nbody {"));
        let mut stray = vec![];
        for (at, _) in look.match_indices("var(--") {
            let v = &look[at + 6..][..named(&look[at + 6..])];
            let open = look[..at].rfind('{').unwrap();
            let rule = look[..open].rfind(['{', '}']).map_or(0, |i| i + 1);
            // The selector's own line, past any comment above it.
            let selector = look[rule..open].trim().lines().last().unwrap_or("");
            if !(v == "stop" || used.contains(v) || measures.contains(v) || aliases.contains(v) && selector.starts_with(".look")) {
                stray.push((selector, v));
            }
        }
        assert!(stray.is_empty(), "{stray:?}");
        let themes = Themes::default();
        for name in themes.list() {
            let vars = vars(&themes.named(name).unwrap());
            let painted: std::collections::BTreeSet<&str> = vars.keys().copied().filter(|k| *k != "stop").collect();
            assert_eq!(painted, used, "{name}");
            assert!(vars.values().filter(|v| v.starts_with('#')).all(|v| v.len() == 7), "{name}");
        }
        // Before `paint`, and under `?tokens`, the page shows graphite, or light on a light system.
        for (block, theme) in look.split(":root {").skip(1).zip(["graphite", "light"]) {
            let want = vars(&themes.named(theme).unwrap());
            for slot in block.split('}').next().unwrap().split("--").skip(1) {
                let (name, value) = slot.split_once(':').unwrap();
                assert_eq!(value.split(';').next().unwrap().trim(), want[name.trim()], "{theme} {name}");
            }
        }
        assert_eq!(look.matches(":root {").count(), 2);
    }

    // The mockups' stylesheets styled the same class names as look.css, and their rules leaked onto the new
    // screens, so no file under src may bring in styles of its own: no other .css import, `<style>` or `@import`.
    #[test]
    fn the_page_imports_look_css_and_the_fonts_and_no_other_stylesheet() {
        fn files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() { files(&path, out) } else { out.push(path) }
            }
        }
        let src = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../src"));
        let mut all = vec![];
        files(src, &mut all);
        let mut stray = vec![];
        for path in all.iter().filter(|p| p.extension().is_some_and(|e| ["ts", "svelte", "css"].contains(&e.to_str().unwrap()))) {
            let text = std::fs::read_to_string(path).unwrap();
            for line in text.lines() {
                let sheet = line.contains(".css\"") && !line.contains("@fontsource/") && line.trim() != "import \"./look.css\";";
                if sheet || line.contains("<style") || line.trim_start().starts_with("@import") {
                    stray.push(format!("{}: {}", path.strip_prefix(src).unwrap().display(), line.trim()));
                }
            }
        }
        assert!(stray.is_empty(), "{stray:#?}");
        let main = std::fs::read_to_string(src.join("main.ts")).unwrap();
        assert!(main.lines().any(|l| l == "import \"./look.css\";"));
    }

    #[test]
    fn a_light_ground_is_told_apart_by_its_colour_and_not_its_name() {
        let themes = Themes::default();
        assert!(light(&themes.named("light").unwrap()));
        assert!(!light(&themes.named("graphite").unwrap()));
    }

    // Nothing else clears it, so one Esc on a search would otherwise end every later one at once.
    #[test]
    fn a_search_clears_the_stop_and_nothing_else_does() {
        let stop = AtomicBool::new(true);
        arm(&Cmd::Retry, &stop);
        assert!(stop.load(Ordering::SeqCst), "a command that is not a search cleared the stop");
        arm(&Cmd::GetTop { artist: "Can".into(), titles: vec!["Vitamin C".into()] }, &stop);
        assert!(!stop.load(Ordering::SeqCst));
        stop.store(true, Ordering::SeqCst);
        arm(&Cmd::GetAlbum { artist: "Can".into(), title: "Ege Bamyasi".into() }, &stop);
        assert!(!stop.load(Ordering::SeqCst));
    }

    // The page asks on every boot, and a reload boots it again.
    #[test]
    fn the_update_news_is_given_once_a_session_and_never_when_the_check_is_off() {
        let told = AtomicBool::new(false);
        assert!(!first_ask(false, &told));
        assert!(first_ask(true, &told));
        assert!(!first_ask(true, &told));
    }

    // A reloaded page is handed the history; a question it already answered must not be asked again.
    #[test]
    fn a_replay_leaves_out_the_questions_already_answered() {
        let sent = |seq, msg: Msg| Sent { seq, msg: serde_json::to_value(msg).unwrap() };
        let history = vec![
            sent(0, Msg::Stage("library".into())),
            sent(1, Msg::Pick { id: 4 }),
            sent(2, Msg::Pick { id: 5 }),
        ];
        let replay = replayable(&history, &HashSet::from([4]));
        assert_eq!(replay.iter().map(|s| s.seq).collect::<Vec<_>>(), [0, 2]);
    }
}
