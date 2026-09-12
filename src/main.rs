mod app;
mod config;
mod deps;
mod lookup;
mod manifest;
mod player;
mod tag;
mod theme;
mod ui;
mod worker;
mod ytdlp;

use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::{execute, terminal::SetTitle};

use app::{App, Cmd, Prompt, Reply, View};
use config::{Cli, Config};

fn main() -> Result<()> {
    let matches = Cli::command().get_matches();
    let cfg = Config::build(Cli::from_arg_matches(&matches)?, &matches)?;
    if cfg.check {
        return check(&cfg);
    }
    if cfg.list {
        list(&cfg);
        return Ok(());
    }
    /* Both ends, because the TUI needs to write frames and read keys. Piped
       or in CI, crossterm's raw mode fails with an OS error about a device
       that tells nobody which of the two is wrong or what does work here. */
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        anyhow::bail!("earworm needs a terminal · --list and --check work without one");
    }
    let settings = cfg.describe();
    // Read off before the worker takes the config: both belong to the UI.
    let (intro, notify) = (cfg.intro, cfg.notify);
    let (tx, rx) = mpsc::channel();
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    std::thread::spawn(move || worker::run(cfg, tx, worker_cancel, cmd_rx));

    let mut terminal = ratatui::init();
    let mut app = App::new(cmd_tx, settings);
    /* Switched off by saying it is already over, which is what every other
       thing that ends it does. A separate flag would be a second answer to
       the same question. */
    app.intro_done = !intro;
    let result = run(&mut terminal, &mut app, rx, notify);

    cancel.store(true, Ordering::SeqCst);
    ytdlp::stop();
    // Closes the command channel, which is what ends the worker's service loop.
    app.cmds.take();
    /* Exiting part-way through an in-place tag rewrite truncates the file, so
       give the worker a moment to finish one it had already started. */
    let deadline = Instant::now() + Duration::from_secs(5);
    while tag::writing() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    ratatui::restore();
    /* Left as it was found, near enough: a tab still reading "earworm · Focus
       · 12/40" an hour after the run is a lie the shell would otherwise have
       to notice. Most prompts set their own on the next line anyway. */
    let _ = execute!(std::io::stdout(), SetTitle(""));
    result?;

    match &app.done {
        Some(Ok(summary)) => println!("{summary}"),
        Some(Err(err)) => {
            eprintln!("earworm: {err}");
            std::process::exit(1);
        }
        None => {}
    }
    Ok(())
}

/* The first thing to run on a new machine and the first thing to ask for in
   a bug report, so it answers both: what is installed, what earworm read, and
   what this run would do. Exits non-zero when a tool it cannot work without
   is missing, so a script can act on it. */
fn check(cfg: &Config) -> Result<()> {
    let tools = deps::probe_all();
    let config = cfg.config_file.clone();
    let dir = cfg.dir.clone();
    let (text, ready) = deps::report(&deps::Facts {
        tools: &tools,
        config: config.as_deref(),
        config_exists: config.as_deref().is_some_and(Path::is_file),
        key: cfg.acoustid_key.is_some(),
        format: &cfg.format,
        extension: config::extension(&cfg.format),
        dir: &dir,
        playlists: dir.is_dir().then(|| worker::library(&dir).len()),
    });
    print!("{text}");
    if !ready {
        std::process::exit(1);
    }
    Ok(())
}

/* Plain stdout and no TUI at all, so it can be piped or grepped. The URL is
   on the line because it is the one thing a folder cannot tell you by name. */
fn list(cfg: &Config) {
    let shelves = worker::library(&cfg.dir);
    if shelves.is_empty() {
        println!("no playlists under {}", cfg.dir.display());
        return;
    }
    let widest = shelves.iter().map(|s| s.name.chars().count()).max().unwrap_or(0);
    for shelf in shelves {
        let synced = shelf
            .synced
            .map_or_else(|| "never synced".to_string(), manifest::ago);
        let missing = match shelf.missing {
            0 => String::new(),
            n => format!(", {n} missing"),
        };
        println!(
            "{:widest$}  {:>4} tracks{:<13}  {synced:<13}  {}",
            shelf.name, shelf.tracks, missing, shelf.url
        );
    }
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    rx: mpsc::Receiver<app::Msg>,
    notify: bool,
) -> Result<()> {
    let mut title = String::new();
    let mut rang = false;
    while !app.quit {
        loop {
            match rx.try_recv() {
                Ok(msg) => app.apply(msg),
                Err(TryRecvError::Empty) => break,
                /* The worker drops its sender on the way out, so a disconnect
                   before Done means it died. Without this the UI would keep
                   drawing a run that had already stopped. */
                Err(TryRecvError::Disconnected) => {
                    // A worker that asked to quit has already said its piece.
                    if app.done.is_none() && !app.quit {
                        app.done = Some(Err("worker stopped unexpectedly".into()));
                        app.quit = true;
                    }
                    break;
                }
            }
        }
        app.expire_flash();
        terminal.draw(|frame| ui::draw(frame, app))?;
        app.tick = app.tick.wrapping_add(1);

        /* Only on a change, or every frame writes an escape sequence nobody
           asked for. This is what the window tab says while the user is
           doing something else, which is most of a forty-minute sync. */
        let now = window_title(app);
        if now != title {
            let _ = execute!(std::io::stdout(), SetTitle(&now));
            title = now;
        }
        /* Decided once, on the frame the run settles: the elapsed time only
           means the run's length at that moment, and a menu left open for a
           minute would otherwise ring for a run that took five seconds.
           Re-armed by the next run, since a session can sync several. */
        if app.done.is_none() {
            rang = false;
        } else if !rang {
            rang = true;
            if notify && worth_a_bell(app) {
                /* A bell and not OSC 9: an escape sequence a terminal does
                   not know prints its own text into the middle of the
                   screen, and a notification is not worth a torn frame. */
                let _ = write!(std::io::stdout(), "\x07");
                let _ = std::io::stdout().flush();
            }
        }

        /* The intro is the only thing on screen that moves between messages,
           so it is the only thing that needs frames faster than the idle poll. */
        let wait = if app.intro() { 33 } else { 120 };
        if event::poll(Duration::from_millis(wait))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            handle_key(app, key.code, key.modifiers);
        }
    }
    Ok(())
}

/* A forty-minute sync is exactly when the user has gone to do something
   else, which is the whole point of saying so. A run that settles at once is
   not one anybody walked away from: opening a folder from the library
   downloads nothing, and a bell there is noise in the same second as the
   keypress that caused it. */
const NOTIFY_AFTER: Duration = Duration::from_secs(30);

fn worth_a_bell(app: &App) -> bool {
    app.done.is_some()
        && app
            .run_started
            .is_some_and(|at| at.elapsed() >= NOTIFY_AFTER)
}

/// What the terminal's tab strip says. The playlist names which run it is,
/// and the count is the only thing worth switching back for.
fn window_title(app: &App) -> String {
    let mut title = String::from("earworm");
    if !app.playlist.is_empty() {
        title.push_str(&format!(" · {}", app.playlist));
    }
    let (done, total) = app.progress();
    match &app.done {
        Some(Ok(_)) => title.push_str(" · finished"),
        Some(Err(_)) => title.push_str(" · failed"),
        // Nothing to count before the list arrives, and saying 0/0 would
        // make a scan that is working look like one that is stuck.
        None if total > 0 => title.push_str(&format!(" · {done}/{total}")),
        None => {}
    }
    title
}

fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    /* Swallows the key rather than also acting on it: skipping an animation
       must not double as a command the user never saw the screen for. */
    if app.intro() {
        app.intro_done = true;
        return;
    }
    if app.prompt.is_some() {
        handle_prompt_key(app, code, mods);
        return;
    }
    /* Ahead of every screen's keys: it is the UI's own question and the
       answer must not also reach the list behind it. */
    if app.confirm.is_some() {
        handle_confirm_key(app, code, mods);
        return;
    }
    /* The overlay swallows the next key rather than acting on it: anything
       else makes dismissing it a guess about what the key also did. */
    if app.show_help && !matches!(code, KeyCode::Char('q')) {
        app.show_help = false;
        return;
    }
    if app.view == View::Library {
        handle_library_key(app, code, mods);
        return;
    }
    /* The filter box takes every key while it is open, so `q` types a letter
       instead of ending the session. */
    if app.typing_filter {
        handle_filter_key(app, code, mods);
        return;
    }
    /* The pick owns the keys while it is open: the worker is blocked on the
       answer, so a key that started a command would queue work behind a
       question nothing is going to answer. */
    if app.picking.is_some() {
        handle_pick_key(app, code, mods);
        return;
    }
    match code {
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.quit = true,
        KeyCode::Char('h') | KeyCode::Char('?') => app.show_help = true,
        KeyCode::Char('q') => app.ask_quit(),
        /* Esc unwinds one step at a time: the filter first, since a hidden
           list is the thing most likely to have prompted the keypress, then
           the library, then the tool. */
        KeyCode::Esc if app.narrowed() => app.clear_filter(),
        KeyCode::Esc => app.leave_tracks(),
        KeyCode::Char('/') => app.typing_filter = true,
        KeyCode::Char('f') => app.follow = !app.follow,
        /* The same narrowing the finish menu offers, for the times you go
           looking rather than being asked. */
        KeyCode::Char('v') => {
            app.review = !app.review;
            app.snap();
            if app.review && app.shown() == 0 {
                app.review = false;
                app.say("nothing here needs a look");
            }
        }
        // Next and previous of the same set, without hiding what is around
        // them: the run is the context a guess is judged in.
        KeyCode::Char('n') => app.jump_attention(true),
        KeyCode::Char('N') => app.jump_attention(false),
        KeyCode::Char('u') if app.can_command() && app.undoable.is_some() => {
            app.send(Cmd::Undo);
        }
        KeyCode::Char('l') => app.show_logs = !app.show_logs,
        // Sized and scrolled only while it is open, or the keys act on a pane
        // nobody can see.
        KeyCode::Char('L') if app.show_logs => app.logs_tall = !app.logs_tall,
        KeyCode::Char('K') if app.show_logs => app.scroll_logs(true, 1),
        KeyCode::Char('J') if app.show_logs => app.scroll_logs(false, 1),
        KeyCode::Char('j') | KeyCode::Down => app.step(true),
        KeyCode::Char('k') | KeyCode::Up => app.step(false),
        KeyCode::PageDown => app.page(true),
        KeyCode::PageUp => app.page(false),
        KeyCode::Char('d') if mods.contains(KeyModifiers::CONTROL) => app.page(true),
        KeyCode::Char('u') if mods.contains(KeyModifiers::CONTROL) => app.page(false),
        KeyCode::Char('e') if app.can_command() => {
            if let Some(index) = app.selected() {
                app.send(Cmd::Edit(index));
            }
        }
        KeyCode::Char('c') if app.can_command() => {
            if let Some(index) = app.selected() {
                app.send(Cmd::Cover(index));
            }
        }
        KeyCode::Char('r') if app.can_command() => app.send(Cmd::Retry),
        KeyCode::Char('S') if app.can_command() => app.send(Cmd::SyncOne),
        KeyCode::Char('p') if app.can_command() && app.can_play() => {
            if let Some(folder) = app.folder.clone() {
                app.send(Cmd::Play(folder));
            }
        }
        KeyCode::Char(' ') => app.toggle_mark(),
        KeyCode::Char('m') => app.mark_like_cursor(),
        // The other question a run raises: one artist's tracks, for the
        // compilation a lookup split across twenty of them.
        KeyCode::Char('M') => app.mark_like_artist(),
        KeyCode::Char('s') if app.can_command() => {
            let targets = app.targets();
            app.send(Cmd::Swap(targets));
        }
        KeyCode::Char('A') if app.can_command() => {
            let targets = app.targets();
            app.send(Cmd::Artist(targets));
        }
        KeyCode::Char('g') => app.jump(false),
        KeyCode::Char('G') => app.jump(true),
        _ => {}
    }
}

/* `y`, `q` and Enter all mean yes, so a second `q` answers the question the
   first one raised and nobody who meant it has to read the box. Everything
   else means no: this is the guard on the one key that can still lose work,
   so an unrecognised key must not be an accidental yes. */
fn handle_confirm_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let yes = matches!(
        code,
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('q') | KeyCode::Enter
    ) || (matches!(code, KeyCode::Char('c')) && mods.contains(KeyModifiers::CONTROL));
    app.confirm = None;
    if yes {
        app.quit = true;
    }
}

/* Moving, marking and filtering are the same keys as always, so the selection
   is made with the list the user is already reading. Only Enter and Esc are
   new, and both answer the question the worker is blocked on. */
fn handle_pick_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    match code {
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.quit = true,
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('h') | KeyCode::Char('?') => app.show_help = true,
        /* Enter on an empty selection downloads nothing, which is what Esc
           is for and not what Enter looks like. It is one `a` away from
           being an accident, so it says what both keys do and stays. */
        KeyCode::Enter if app.marked.is_empty() => {
            app.say("nothing selected  ·  a marks all  ·  esc downloads none");
        }
        KeyCode::Enter => app.confirm_pick(),
        KeyCode::Esc if !app.filter.is_empty() => app.clear_filter(),
        KeyCode::Esc => app.cancel_pick(),
        KeyCode::Char('/') => app.typing_filter = true,
        KeyCode::Char(' ') => app.toggle_mark(),
        KeyCode::Char('m') => app.mark_like_cursor(),
        KeyCode::Char('a') => app.toggle_all(),
        // "Everything except these three", which is otherwise `a` and then
        // marking the rest back by hand.
        KeyCode::Char('i') => app.invert_marks(),
        /* No `M` here: nothing has been tagged yet at the pick, so every
           track's artist is empty and the key could only ever refuse. */
        KeyCode::Char('l') => app.show_logs = !app.show_logs,
        KeyCode::Char('j') | KeyCode::Down => app.step(true),
        KeyCode::Char('k') | KeyCode::Up => app.step(false),
        KeyCode::PageDown => app.page(true),
        KeyCode::PageUp => app.page(false),
        KeyCode::Char('d') if mods.contains(KeyModifiers::CONTROL) => app.page(true),
        KeyCode::Char('u') if mods.contains(KeyModifiers::CONTROL) => app.page(false),
        KeyCode::Char('g') => app.jump(false),
        KeyCode::Char('G') => app.jump(true),
        _ => {}
    }
}

/* Narrows as you type rather than on Enter, so the list answers each letter.
   Enter only puts the keys back; the filter itself stays until Esc. */
fn handle_filter_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    match code {
        KeyCode::Esc => app.clear_filter(),
        KeyCode::Enter => app.typing_filter = false,
        KeyCode::Char('u') if mods.contains(KeyModifiers::CONTROL) => {
            app.filter.clear();
            app.snap();
        }
        // The same word delete the prompts take, since this is the same typing.
        KeyCode::Char('w') if mods.contains(KeyModifiers::CONTROL) => {
            let kept = app.filter.trim_end();
            let cut = kept.rfind(' ').map_or(0, |i| i + 1);
            app.filter.truncate(cut);
            app.snap();
        }
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.quit = true,
        KeyCode::Backspace => {
            app.filter.pop();
            app.snap();
        }
        // Every other control combination would arrive as a bare letter.
        KeyCode::Char(c) if !mods.contains(KeyModifiers::CONTROL) => {
            app.filter.push(c);
            app.snap();
        }
        _ => {}
    }
}

/* The library moves a different cursor and its rows are folders, so it takes
   almost none of the track keys. Everything that acts goes through the worker,
   which owns the folders as it owns every other file. */
fn handle_library_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    match code {
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.quit = true,
        KeyCode::Char('h') | KeyCode::Char('?') => app.show_help = true,
        KeyCode::Char('q') => app.quit = true,
        // Unwinds the filter first, like it does on the track list.
        KeyCode::Esc if app.narrowed() => app.clear_filter(),
        /* The library is the screen behind everything else, so Esc has nothing
           to go back to. It says so rather than quitting: Esc means the same
           thing on every screen or it means nothing anywhere. */
        KeyCode::Esc => app.say("this is the top  ·  q quits"),
        // Twenty folders is a page and sixty is a search, and the box is the
        // one the track list already uses.
        KeyCode::Char('/') => app.typing_filter = true,
        /* Name is what the worker hands over; the other two answer the
           question this screen is for, which is what needs syncing. */
        KeyCode::Char('o') => {
            app.sort = app.sort.next();
            let label = app.sort.label();
            app.say(format!("sorted by {label}"));
        }
        KeyCode::Char('l') => app.show_logs = !app.show_logs,
        KeyCode::Char('j') | KeyCode::Down => app.shelf_step(true),
        KeyCode::Char('k') | KeyCode::Up => app.shelf_step(false),
        KeyCode::PageDown => app.page_shelf(true),
        KeyCode::PageUp => app.page_shelf(false),
        KeyCode::Char('d') if mods.contains(KeyModifiers::CONTROL) => app.page_shelf(true),
        KeyCode::Char('u') if mods.contains(KeyModifiers::CONTROL) => app.page_shelf(false),
        KeyCode::Char('g') => app.shelf_jump(false),
        KeyCode::Char('G') => app.shelf_jump(true),
        KeyCode::Enter if app.can_browse() => {
            if let Some(folder) = app.selected_shelf().map(|s| s.path.clone()) {
                app.send(Cmd::Open(folder));
            }
        }
        /* Free here, unlike on the track list where it marks a row, and this
           is the screen that already says what cliamp is doing. */
        KeyCode::Char(' ') if app.can_browse() && app.can_play() => app.send(Cmd::Toggle),
        KeyCode::Char('p') if app.can_browse() && app.can_play() => {
            if let Some(folder) = app.selected_shelf().map(|s| s.path.clone()) {
                app.send(Cmd::Play(folder));
            }
        }
        /* Library-only, like space: `f` follows the active track on the track
           list, and the setting is about the next playlist rather than the
           one on screen. */
        KeyCode::Char('f') if app.can_browse() => app.send(Cmd::Format),
        /* `O` and not `o`, which already cycles the order. Everything
           downstream of earworm happens in a file manager or a player, and
           the alternative is retyping a path the screen is showing. */
        KeyCode::Char('O') if app.can_browse() => {
            if let Some(folder) = app.selected_shelf().map(|s| s.path.clone()) {
                app.send(Cmd::Reveal(folder));
            }
        }
        KeyCode::Char('R') if app.can_browse() => app.send(Cmd::ResyncAll),
        KeyCode::Char('n') if app.can_browse() => app.send(Cmd::Url),
        _ => {}
    }
}

/// Esc cancels the question rather than the run: the worker treats a cancel as
/// "leave this track alone", which is the safe answer for every prompt here.
fn handle_prompt_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let Some((prompt, _)) = &app.prompt else {
        return;
    };
    /* Zero means a typing prompt, where every printable key is text rather
       than a command. A form is the same, with Tab moving between boxes. */
    let options = match prompt {
        Prompt::Choice { options, .. } => options.len(),
        Prompt::Input { .. } | Prompt::Form { .. } => 0,
    };
    let form = matches!(prompt, Prompt::Form { .. });

    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let reply = match (code, options) {
        (KeyCode::Esc, _) => Some(Reply::Cancel),
        // A prefilled field is unusable without a way to clear it.
        (KeyCode::Char('u'), 0) if ctrl => {
            app.input_clear();
            None
        }
        (KeyCode::Char('w'), 0) if ctrl => {
            app.input_kill_word();
            None
        }
        /* Fixing one letter in the middle of a title is the most common edit
           in the tool, and until the caret existed it meant deleting the rest
           of the line and typing it again. */
        (KeyCode::Left, 0) => {
            app.input_move(false);
            None
        }
        (KeyCode::Right, 0) => {
            app.input_move(true);
            None
        }
        (KeyCode::Home, 0) => {
            app.input_end(false);
            None
        }
        (KeyCode::End, 0) => {
            app.input_end(true);
            None
        }
        (KeyCode::Char('a'), 0) if ctrl => {
            app.input_end(false);
            None
        }
        (KeyCode::Char('e'), 0) if ctrl => {
            app.input_end(true);
            None
        }
        (KeyCode::Delete, 0) => {
            app.input_delete();
            None
        }
        (KeyCode::Tab, 0) if form => {
            app.next_field(true);
            None
        }
        (KeyCode::BackTab, 0) if form => {
            app.next_field(false);
            None
        }
        (KeyCode::Down, 0) if form => {
            app.next_field(true);
            None
        }
        (KeyCode::Up, 0) if form => {
            app.next_field(false);
            None
        }
        (KeyCode::Char('s'), 0) if form && ctrl => {
            app.swap_fields();
            None
        }
        (KeyCode::Enter, 0) if form => Some(Reply::Fields(app.answers())),
        (KeyCode::Enter, 0) => Some(Reply::Text(app.input().to_string())),
        (KeyCode::Enter, _) => Some(Reply::Choice(app.choice)),
        (KeyCode::Down | KeyCode::Tab | KeyCode::Char('j'), n) if n > 0 => {
            app.choice = (app.choice + 1) % n;
            None
        }
        (KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k'), n) if n > 0 => {
            app.choice = (app.choice + n - 1) % n;
            None
        }
        /* One key instead of a walk with j and k. Bounded by the options
           actually on screen, so a digit past the end does nothing rather
           than picking the last row, which on the finish menu is quit. */
        (KeyCode::Char(c @ '1'..='9'), n) if n > 0 && !ctrl => {
            let pick = c as usize - '1' as usize;
            (pick < n).then(|| {
                app.choice = pick;
                Reply::Choice(pick)
            })
        }
        (KeyCode::Char(c), 0) if !ctrl => {
            app.input_insert(c);
            None
        }
        (KeyCode::Backspace, 0) => {
            app.input_backspace();
            None
        }
        _ => None,
    };

    if let Some(reply) = reply {
        if let Some((_, tx)) = app.prompt.take() {
            let _ = tx.send(reply);
        }
        app.close_fields();
        app.choice = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Confirm;

    /* What the tab strip says while earworm is in a window nobody is
       looking at, which is most of a forty-minute sync. */
    #[test]
    fn the_window_title_carries_the_playlist_and_the_count() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        // Nothing to count yet: 0/0 would make a working scan look stuck.
        assert_eq!(window_title(&app), "earworm");

        app.playlist = "Focus".into();
        app.tracks = (1..=3)
            .map(|n| {
                crate::app::Track::new(n, "id".into(), "name".into(), std::path::PathBuf::new())
            })
            .collect();
        app.tracks[0].status = crate::app::Status::Ok;
        assert_eq!(window_title(&app), "earworm · Focus · 1/3");

        app.done = Some(Ok("12 tracks".into()));
        assert_eq!(window_title(&app), "earworm · Focus · finished");
        app.done = Some(Err("yt-dlp died".into()));
        assert_eq!(window_title(&app), "earworm · Focus · failed");
    }

    /* `o` already cycles the order, so the reveal is `O`. Both are library
       keys: a folder is what either of them acts on. */
    #[test]
    fn the_library_reveal_key_sends_the_folder_under_the_cursor() {
        let (tx, cmds) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        app.view = View::Library;
        app.apply(app::Msg::Library {
            shelves: vec![crate::app::Shelf {
                path: std::path::PathBuf::from("/music/Focus"),
                name: "Focus".into(),
                url: "u".into(),
                tracks: 3,
                missing: 0,
                synced: None,
                files: Vec::new(),
            }],
            show: true,
        });

        // Or the first key goes on skipping the intro instead.
        app.intro_done = true;

        handle_key(&mut app, KeyCode::Char('O'), KeyModifiers::NONE);
        assert!(matches!(
            cmds.try_recv(),
            Ok(Cmd::Reveal(path)) if path == std::path::Path::new("/music/Focus")
        ));

        // The lower-case key is the sort, and must not also open a window.
        handle_key(&mut app, KeyCode::Char('o'), KeyModifiers::NONE);
        assert!(cmds.try_recv().is_err(), "o sent a command as well as sorting");
        assert_eq!(app.sort, crate::app::Sort::Synced);
    }

    /* A forty-minute sync is when someone has gone to do something else,
       which is the whole point of the bell. A folder opened from the library
       settles in the same second as the keypress, and ringing for that is
       noise. */
    #[test]
    fn only_a_run_that_took_a_while_is_worth_a_bell() {
        let settled = |ago: Option<Duration>| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, String::new());
            app.done = Some(Ok(String::new()));
            app.run_started = ago.map(|d| Instant::now() - d);
            worth_a_bell(&app)
        };
        assert!(settled(Some(Duration::from_secs(600))));
        assert!(!settled(Some(Duration::from_secs(2))));
        // Nothing ran at all: the library screen never sets the clock.
        assert!(!settled(None));

        let (tx, _rx) = std::sync::mpsc::channel();
        let mut going = App::new(tx, String::new());
        going.run_started = Some(Instant::now() - Duration::from_secs(600));
        assert!(!worth_a_bell(&going), "it rang for a run still going");
    }

    /* Enter with nothing marked downloads nothing, which is what Esc is for.
       It is one stray `a` away from being an accident, so it says what both
       keys do and leaves the question open. */
    #[test]
    fn enter_at_the_pick_with_nothing_marked_stays_and_says_so() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        app.tracks = vec![crate::app::Track::new(
            1,
            "id".into(),
            "name".into(),
            std::path::PathBuf::new(),
        )];
        let (reply, answers) = mpsc::channel();
        app.picking = Some(reply);

        handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.picking.is_some(), "the pick was answered by accident");
        assert!(answers.try_recv().is_err(), "it answered anyway");
        assert!(app.stage.contains("esc downloads none"), "{}", app.stage);

        // Esc is still the way to say it, and it does answer.
        handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.picking.is_none());
        assert!(matches!(answers.try_recv(), Ok(Reply::Picked(picked)) if picked.is_empty()));
    }

    /* One level of undo is only usable if the key is live exactly when the
       worker is holding something, since nothing else on screen says so. */
    #[test]
    fn undo_is_only_a_key_while_there_is_something_to_put_back() {
        let sent = |undoable: Option<&str>| {
            let (tx, cmds) = std::sync::mpsc::channel();
            let mut app = App::new(tx, String::new());
            app.tracks = vec![crate::app::Track::new(
                1,
                "id".into(),
                "name".into(),
                std::path::PathBuf::new(),
            )];
            app.done = Some(Ok(String::new()));
            app.undoable = undoable.map(str::to_string);
            handle_key(&mut app, KeyCode::Char('u'), KeyModifiers::NONE);
            cmds.try_iter().count()
        };
        assert_eq!(sent(Some("the edit of track 1")), 1);
        assert_eq!(sent(None), 0, "u sent a command with nothing to undo");
    }

    /* The guard on the one key that can still lose work, so an unrecognised
       key must not be an accidental yes. */
    #[test]
    fn only_a_deliberate_key_confirms_the_quit() {
        let asked = || {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, String::new());
            app.confirm = Some(Confirm::Quit);
            app
        };

        for code in [
            KeyCode::Char('y'),
            KeyCode::Char('q'),
            KeyCode::Enter,
        ] {
            let mut app = asked();
            handle_confirm_key(&mut app, code, KeyModifiers::NONE);
            assert!(app.quit, "{code:?} was supposed to mean yes");
            assert_eq!(app.confirm, None, "{code:?} left the question open");
        }

        for code in [
            KeyCode::Esc,
            KeyCode::Char('n'),
            KeyCode::Char('j'),
            KeyCode::Char(' '),
            KeyCode::Backspace,
        ] {
            let mut app = asked();
            handle_confirm_key(&mut app, code, KeyModifiers::NONE);
            assert!(!app.quit, "{code:?} quit a run that was still going");
            assert_eq!(app.confirm, None, "{code:?} left the question open");
        }

        // ^c has always been the way out of anything and stays one press.
        let mut app = asked();
        handle_confirm_key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(app.quit);
    }

    /* A digit past the end used to be worth guarding: on the finish menu
       the last row is quit. */
    #[test]
    fn a_digit_picks_only_an_option_that_is_there() {
        let asked = |options: usize| {
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut app = App::new(tx, String::new());
            let (reply, answers) = mpsc::channel();
            app.prompt = Some((
                Prompt::Choice {
                    header: "finished".into(),
                    note: String::new(),
                    options: (0..options).map(|n| format!("option {n}")).collect(),
                    escape: crate::app::Escape::Keep,
                },
                reply,
            ));
            (app, answers)
        };

        let (mut app, answers) = asked(4);
        handle_prompt_key(&mut app, KeyCode::Char('3'), KeyModifiers::NONE);
        assert!(matches!(answers.try_recv(), Ok(Reply::Choice(2))));

        // Past the end: nothing answered, and the menu is still open.
        let (mut app, answers) = asked(4);
        handle_prompt_key(&mut app, KeyCode::Char('7'), KeyModifiers::NONE);
        assert!(answers.try_recv().is_err(), "a digit past the end answered");
        assert!(app.prompt.is_some(), "the menu closed on a key it ignored");

        // A digit is text in a box, not a command.
        let (mut app, answers) = asked(0);
        app.prompt = Some((
            Prompt::Input {
                note: String::new(),
                header: "Title".into(),
                value: String::new(),
                escape: crate::app::Escape::Skip,
            },
            mpsc::channel().0,
        ));
        drop(answers);
        handle_prompt_key(&mut app, KeyCode::Char('3'), KeyModifiers::NONE);
        assert_eq!(app.input(), "3", "a digit was eaten as a command");
    }

    /* The question owns the keys while it is up, or the answer also reaches
       the list behind it. */
    #[test]
    fn the_question_takes_the_key_from_the_screen_behind_it() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut app = App::new(tx, String::new());
        app.intro_done = true;
        app.tracks = vec![crate::app::Track::new(
            1,
            "id".into(),
            "name".into(),
            std::path::PathBuf::from("/tmp/x.opus"),
        )];
        app.confirm = Some(Confirm::Quit);

        handle_key(&mut app, KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(app.marked.is_empty(), "the answer also marked a track");
        assert_eq!(app.confirm, None);
    }
}
