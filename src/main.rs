mod app;
mod config;
mod lookup;
mod manifest;
mod player;
mod tag;
mod theme;
mod ui;
mod worker;
mod ytdlp;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use app::{App, Cmd, Prompt, Reply, View};
use config::{Cli, Config};

fn main() -> Result<()> {
    let matches = Cli::command().get_matches();
    let cfg = Config::build(Cli::from_arg_matches(&matches)?, &matches)?;
    if cfg.list {
        list(&cfg);
        return Ok(());
    }
    let settings = cfg.describe();
    let (tx, rx) = mpsc::channel();
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    std::thread::spawn(move || worker::run(cfg, tx, worker_cancel, cmd_rx));

    let mut terminal = ratatui::init();
    let mut app = App::new(cmd_tx, settings);
    let result = run(&mut terminal, &mut app, rx);

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
) -> Result<()> {
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
        KeyCode::Char('q') => app.quit = true,
        /* Esc unwinds one step at a time: the filter first, since a hidden
           list is the thing most likely to have prompted the keypress, then
           the library, then the tool. */
        KeyCode::Esc if !app.filter.is_empty() => app.clear_filter(),
        KeyCode::Esc => app.leave_tracks(),
        KeyCode::Char('/') => app.typing_filter = true,
        KeyCode::Char('f') => app.follow = !app.follow,
        KeyCode::Char('l') => app.show_logs = !app.show_logs,
        KeyCode::Char('j') | KeyCode::Down => app.step(true),
        KeyCode::Char('k') | KeyCode::Up => app.step(false),
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

/* Moving, marking and filtering are the same keys as always, so the selection
   is made with the list the user is already reading. Only Enter and Esc are
   new, and both answer the question the worker is blocked on. */
fn handle_pick_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    match code {
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.quit = true,
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('h') | KeyCode::Char('?') => app.show_help = true,
        KeyCode::Enter => app.confirm_pick(),
        KeyCode::Esc if !app.filter.is_empty() => app.clear_filter(),
        KeyCode::Esc => app.cancel_pick(),
        KeyCode::Char('/') => app.typing_filter = true,
        KeyCode::Char(' ') => app.toggle_mark(),
        KeyCode::Char('m') => app.mark_like_cursor(),
        KeyCode::Char('a') => app.toggle_all(),
        KeyCode::Char('l') => app.show_logs = !app.show_logs,
        KeyCode::Char('j') | KeyCode::Down => app.step(true),
        KeyCode::Char('k') | KeyCode::Up => app.step(false),
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
        KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
        KeyCode::Char('l') => app.show_logs = !app.show_logs,
        KeyCode::Char('j') | KeyCode::Down => {
            app.shelf = (app.shelf + 1).min(app.library.len().saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => app.shelf = app.shelf.saturating_sub(1),
        KeyCode::Char('g') => app.shelf = 0,
        KeyCode::Char('G') => app.shelf = app.library.len().saturating_sub(1),
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
    let options = match prompt {
        Prompt::Choice { options, .. } => options.len(),
        Prompt::Input { .. } => 0,
    };

    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let reply = match (code, options) {
        (KeyCode::Esc, _) => Some(Reply::Cancel),
        // A prefilled field is unusable without a way to clear it.
        (KeyCode::Char('u'), 0) if ctrl => {
            app.input.clear();
            None
        }
        (KeyCode::Char('w'), 0) if ctrl => {
            let kept = app.input.trim_end();
            let cut = kept.rfind(' ').map_or(0, |i| i + 1);
            app.input.truncate(cut);
            None
        }
        (KeyCode::Enter, 0) => Some(Reply::Text(app.input.clone())),
        (KeyCode::Enter, _) => Some(Reply::Choice(app.choice)),
        (KeyCode::Down | KeyCode::Tab | KeyCode::Char('j'), n) if n > 0 => {
            app.choice = (app.choice + 1) % n;
            None
        }
        (KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k'), n) if n > 0 => {
            app.choice = (app.choice + n - 1) % n;
            None
        }
        (KeyCode::Char(c), 0) if !ctrl => {
            app.input.push(c);
            None
        }
        (KeyCode::Backspace, 0) => {
            app.input.pop();
            None
        }
        _ => None,
    };

    if let Some(reply) = reply {
        if let Some((_, tx)) = app.prompt.take() {
            let _ = tx.send(reply);
        }
        app.input.clear();
        app.choice = 0;
    }
}
