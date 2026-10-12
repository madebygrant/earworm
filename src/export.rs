use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};

use crate::app::{Asker, Msg};
use crate::config::{self, composed};
use crate::lookup;
use crate::playlists::{self, Playlist};
use crate::portable::{self, Form};
use crate::worker::{self, Others};

/// What the export is of: a library folder by path, or a custom playlist by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Folder(PathBuf),
    List(String),
}

/// How a custom playlist lands on the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// One folder holding copies of every track and a playlist beside them.
    Copies,
    /// Each track in a folder of its own name, with the playlist at the root pointing into them.
    Folders,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub copies: Vec<(PathBuf, PathBuf)>,
    /// Destinations that are re-encoded on the way, and the format they become.
    pub convert: std::collections::HashMap<PathBuf, &'static str>,
    /// Who wrote what is on the device, which is the only claim a mirror may act on. See `RECORD`.
    pub owner: String,
    pub playlists: Vec<(PathBuf, String)>,
    /// Entries of a custom playlist that no folder holds, so there was nothing to copy.
    pub missing: usize,
}

// Hidden, so a copy killed part-way is swept and never read as a track.
const SCRATCH: &str = ".earworm-export-";
// Hidden, one per folder on the device: `owner<TAB>name` for every file earworm wrote there. A mirror offers
// only what its own owner recorded, because nothing else says a file in a folder of the same name is ours.
const RECORD: &str = ".earworm-export";
// FAT keeps modification times to 2 seconds.
const SLACK: Duration = Duration::from_secs(2);

fn spelled(name: &str, form: Form) -> String {
    portable::name(name, form)
}

// The name a file goes under on the device, with the extension of the format it is being re-encoded to.
fn device_name(file: &Path, format: Option<&str>, form: Form) -> (String, Option<&'static str>) {
    let raw = file.file_name().unwrap_or_default().to_string_lossy().to_string();
    let target = format
        .and_then(|f| config::FORMATS.iter().find(|(name, _, _)| *name == f))
        .filter(|(name, _, _)| worker::format_on_disk(file) != Some(*name));
    match target {
        Some((name, ext, _)) => (spelled(&Path::new(&raw).with_extension(ext).to_string_lossy(), form), Some(*name)),
        None => (spelled(&raw, form), None),
    }
}

fn audio_names(folder: &Path) -> Vec<String> {
    worker::audio_files(folder)
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect()
}

// The order earworm last wrote, when the folder has one playlist file, and then anything it does not name.
fn folder_order(folder: &Path) -> Vec<String> {
    let have = audio_names(folder);
    let by_form: std::collections::HashMap<String, &String> = have.iter().map(|n| (composed(n), n)).collect();
    let mut order: Vec<String> = Vec::new();
    let lists: Vec<PathBuf> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "m3u8") && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .collect();
    if let [only] = lists.as_slice() {
        let text = std::fs::read_to_string(only).unwrap_or_default();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            if let Some(name) = by_form.get(&composed(line)) {
                if !order.contains(name) {
                    order.push((*name).clone());
                }
            }
        }
    }
    let mut rest: Vec<String> = have.into_iter().filter(|n| !order.contains(n)).collect();
    rest.sort();
    order.extend(rest);
    order
}

fn m3u8(names: &[String]) -> String {
    let mut text = String::from("#EXTM3U\n");
    for name in names {
        text.push_str(name);
        text.push('\n');
    }
    text
}

// How a folder's files are named on the device, in order. Shared with the folder layout of a list, so a
// track gets one device name whichever export puts it there.
fn folder_names(folder: &Path, form: Form, format: Option<&str>) -> (Vec<(PathBuf, String, Option<&'static str>)>, HashSet<String>) {
    let mut taken = HashSet::new();
    let named = folder_order(folder)
        .into_iter()
        .map(|file| {
            let from = folder.join(&file);
            let (name, convert) = device_name(&from, format, form);
            let safe = portable::unique(&mut taken, &name);
            (from, safe, convert)
        })
        .collect();
    (named, taken)
}

/// A library folder, as a folder of the same name on the device with its own playlist inside.
pub fn plan_folder(folder: &Path, dest: &Path, form: Form, format: Option<&str>) -> Plan {
    let raw = folder.file_name().unwrap_or_default().to_string_lossy().to_string();
    let home = dest.join(spelled(&raw, form));
    let mut plan = Plan { owner: format!("folder:{raw}"), ..Plan::default() };
    let (named, mut taken) = folder_names(folder, form, format);
    let mut names = Vec::new();
    for (from, safe, convert) in named {
        let to = home.join(&safe);
        if let Some(f) = convert {
            plan.convert.insert(to.clone(), f);
        }
        plan.copies.push((from, to));
        names.push(safe);
    }
    let list = portable::unique(&mut taken, &spelled(&format!("{raw}.m3u8"), form));
    plan.playlists.push((home.join(list), m3u8(&names)));
    plan
}

/// A custom playlist, from the files its entries resolved to (`None` where no folder holds one).
pub fn plan_list(
    dir: &Path,
    list: &Playlist,
    files: &[Option<PathBuf>],
    dest: &Path,
    form: Form,
    layout: Layout,
    format: Option<&str>,
) -> Plan {
    let mut plan = Plan { owner: format!("list:{}", list.name), ..Plan::default() };
    let name = spelled(&format!("{}.m3u8", list.name), form);
    let mut names = Vec::new();
    match layout {
        Layout::Copies => {
            let home = dest.join(spelled(&list.name, form));
            let mut taken = HashSet::new();
            for file in files.iter().flatten() {
                let (name, convert) = device_name(file, format, form);
                let safe = portable::unique(&mut taken, &name);
                let to = home.join(&safe);
                if let Some(f) = convert {
                    plan.convert.insert(to.clone(), f);
                }
                plan.copies.push((file.clone(), to));
                names.push(safe);
            }
            plan.playlists.push((home.join(&name), m3u8(&names)));
        }
        Layout::Folders => {
            // Named as the folder's own export names them, so the two never give one device name to two tracks.
            let mut known: std::collections::HashMap<PathBuf, (std::collections::HashMap<PathBuf, (String, Option<&'static str>)>, HashSet<String>)> =
                std::collections::HashMap::new();
            for file in files.iter().flatten() {
                if plan.copies.iter().any(|(from, _)| from == file) {
                    let to = &plan.copies.iter().find(|(from, _)| from == file).expect("just found").1;
                    names.push(to.strip_prefix(dest).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default());
                    continue;
                }
                let parent = file.parent().unwrap_or(dir).to_path_buf();
                let (map, taken) = known.entry(parent.clone()).or_insert_with(|| {
                    let (named, taken) = folder_names(&parent, form, format);
                    (named.into_iter().map(|(from, name, convert)| (from, (name, convert))).collect(), taken)
                });
                let (safe, convert) = match map.get(file) {
                    Some((name, convert)) => (name.clone(), *convert),
                    None => {
                        let (name, convert) = device_name(file, format, form);
                        (portable::unique(taken, &name), convert)
                    }
                };
                let home = file.parent().and_then(|p| p.strip_prefix(dir).ok()).and_then(|r| r.components().next());
                let home = home.map_or_else(String::new, |c| c.as_os_str().to_string_lossy().to_string());
                let to = dest.join(spelled(&home, form)).join(safe);
                if let Some(f) = convert {
                    plan.convert.insert(to.clone(), f);
                }
                names.push(to.strip_prefix(dest).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default());
                plan.copies.push((file.clone(), to));
            }
            plan.playlists.push((dest.join(&name), m3u8(&names)));
        }
    }
    plan.missing = files.iter().filter(|f| f.is_none()).count();
    plan
}

/// Mounted volumes worth offering as a destination, which is a macOS listing and empty elsewhere.
fn mounts() -> Vec<String> {
    let Ok(read) = std::fs::read_dir("/Volumes") else {
        return Vec::new();
    };
    let mut found: Vec<String> = read.flatten().map(|e| e.path().display().to_string()).collect();
    found.sort();
    found
}

fn free_bytes(dest: &Path) -> Option<u64> {
    let out = lookup::run_bounded(
        Command::new("df").args(["-Pk"]).arg(dest).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()),
        Duration::from_secs(5),
    )?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let kib: u64 = text.lines().nth(1)?.split_whitespace().nth(3)?.parse().ok()?;
    Some(kib * 1024)
}

fn close_enough(a: SystemTime, b: SystemTime) -> bool {
    let gap = a.duration_since(b).unwrap_or_else(|e| e.duration());
    gap <= SLACK
}

// Same modification time to the device's resolution, and for a straight copy the same size: it is already there.
// A re-encoded file is a different size by design, so the time says it came from this source and the codec
// says it is the format asked for, which matters because alac and m4a share an extension.
fn already_there(from: &Path, to: &Path, format: Option<&str>) -> bool {
    let (Ok(a), Ok(b)) = (std::fs::metadata(from), std::fs::metadata(to)) else {
        return false;
    };
    let same = match format {
        Some(f) => worker::format_on_disk(to) == Some(f),
        None => a.len() == b.len(),
    };
    match (a.modified(), b.modified()) {
        (Ok(x), Ok(y)) => same && close_enough(x, y),
        _ => false,
    }
}

// Names are the same file to a case- or accent-insensitive disk, which is every one a device is likely to have.
fn key(name: &str) -> String {
    composed(name).to_lowercase()
}

fn read_record(home: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(home.join(RECORD))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .map(|(owner, name)| (owner.to_string(), name.to_string()))
        .collect()
}

fn write_record(home: &Path, lines: &[(String, String)]) -> Result<()> {
    let path = home.join(RECORD);
    if lines.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let mut text = String::from("#earworm export record: owner, name\n");
    for (owner, name) in lines {
        text.push_str(&format!("{owner}\t{name}\n"));
    }
    write_atomic(&path, &text)
}

fn owns(owner: &str, path: &Path) -> bool {
    let (Some(home), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let want = key(&name.to_string_lossy());
    read_record(home).iter().any(|(o, n)| o == owner && key(n) == want)
}

// The folders on the device this plan writes into.
fn touched(plan: &Plan) -> Vec<PathBuf> {
    let mut homes: Vec<PathBuf> = plan
        .copies
        .iter()
        .map(|(_, to)| to)
        .chain(plan.playlists.iter().map(|(p, _)| p))
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .collect();
    homes.sort();
    homes.dedup();
    homes
}

// Estimated, since a lossless copy of a lossy file is several times the size.
fn size_after(from: &Path, format: Option<&str>) -> u64 {
    let len = std::fs::metadata(from).map(|m| m.len()).unwrap_or(0);
    let widens = format.is_some_and(config::lossless) && worker::format_on_disk(from).is_none_or(|s| !config::lossless(s));
    if widens { len * 6 } else { len }
}

// The tags, lyrics and picture go across by hand, since ffmpeg drops the picture and maps tags unevenly between containers.
fn reencode(from: &Path, scratch: &Path, format: &str) -> Result<()> {
    let source = worker::format_on_disk(from);
    let kept = crate::tag::Kept::read(from);
    crate::tag::transcode(from, scratch, format, source)?;
    if std::fs::metadata(scratch).map(|m| m.len()).unwrap_or(0) == 0 {
        bail!("the converted file is empty");
    }
    // A picture that will not go on is not worth failing the copy for.
    kept.write(scratch)?;
    Ok(())
}

// macOS leaves a `._name` beside what it writes to exFAT. Only one this call caused is removed.
fn apple_double(path: &Path) -> Option<PathBuf> {
    Some(path.with_file_name(format!("._{}", path.file_name()?.to_string_lossy())))
}

fn make_dirs(dir: &Path) -> Result<()> {
    let mut missing: Vec<&Path> = dir.ancestors().take_while(|d| !d.exists()).collect();
    missing.reverse();
    for made in missing {
        let cleanup = apple_double(made).filter(|p| !p.exists());
        std::fs::create_dir(made).with_context(|| format!("making {}", made.display()))?;
        if let Some(extra) = cleanup {
            let _ = std::fs::remove_file(extra);
        }
    }
    Ok(())
}

fn put(from: &Path, to: &Path, format: Option<&str>) -> Result<()> {
    let dir = to.parent().context("a destination with no folder")?;
    make_dirs(dir)?;
    let scratch = dir.join(format!("{SCRATCH}{}", to.file_name().unwrap_or_default().to_string_lossy()));
    let cleanup = apple_double(to).filter(|p| !p.exists());
    match format {
        Some(format) => reencode(from, &scratch, format)
            .inspect_err(|_| {
                let _ = std::fs::remove_file(&scratch);
            })
            .with_context(|| format!("converting {}", from.display()))?,
        None => {
            std::fs::copy(from, &scratch).with_context(|| format!("copying {}", from.display()))?;
        }
    }
    if let Ok(modified) = std::fs::metadata(from).and_then(|m| m.modified()) {
        let _ = std::fs::OpenOptions::new().write(true).open(&scratch).and_then(|f| f.set_modified(modified));
    }
    let moved = std::fs::rename(&scratch, to);
    if moved.is_err() {
        let _ = std::fs::remove_file(&scratch);
    }
    moved.with_context(|| format!("placing {}", to.display()))?;
    if let Some(extra) = cleanup {
        let _ = std::fs::remove_file(extra);
    }
    // And the scratch name's own, which the copy caused too.
    if let Some(extra) = apple_double(&scratch) {
        let _ = std::fs::remove_file(extra);
    }
    Ok(())
}

fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let dir = path.parent().context("a destination with no folder")?;
    make_dirs(dir)?;
    let scratch = dir.join(format!("{SCRATCH}{}", path.file_name().unwrap_or_default().to_string_lossy()));
    let cleanup = apple_double(path).filter(|p| !p.exists());
    std::fs::write(&scratch, text)?;
    std::fs::rename(&scratch, path).with_context(|| format!("placing {}", path.display()))?;
    if let Some(extra) = cleanup {
        let _ = std::fs::remove_file(extra);
    }
    if let Some(extra) = apple_double(&scratch) {
        let _ = std::fs::remove_file(extra);
    }
    Ok(())
}

// A playlist file already there that earworm did not write is somebody's, and is left alone.
fn write_text(path: &Path, text: &str, owner: &str) -> Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|held| held == text) {
        return Ok(());
    }
    if path.exists() && !owns(owner, path) {
        bail!("{} is already on the device and not from this export  ·  left alone", path.display());
    }
    write_atomic(path, text)
}

// A copy killed part-way leaves a hidden scratch file, and nothing else reads that prefix.
fn sweep(dir: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for path in read.flatten().map(|e| e.path()) {
        if path.is_file() && path.file_name().is_some_and(|n| n.to_string_lossy().starts_with(SCRATCH)) {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Done {
    pub copied: usize,
    pub skipped: usize,
    pub failed: Vec<String>,
    pub stopped: bool,
}

/// Copies what is not already on the device. A file there that this export did not write is never replaced.
pub fn execute(plan: &Plan, tx: &Sender<Msg>, cancel: &AtomicBool) -> Result<Done> {
    let reencoded = |to: &PathBuf| plan.convert.get(to).copied();
    let pending: Vec<&(PathBuf, PathBuf)> = plan.copies.iter().filter(|(from, to)| !already_there(from, to, reencoded(to))).collect();
    let need: u64 = pending.iter().map(|(from, to)| size_after(from, reencoded(to))).sum();
    if let Some(root) = plan.copies.first().and_then(|(_, to)| to.ancestors().find(|a| a.exists()).map(Path::to_path_buf)) {
        if let Some(free) = free_bytes(&root).filter(|free| *free < need) {
            bail!("needs about {} MB and the device has {} MB free", need / 1_000_000 + 1, free / 1_000_000);
        }
    }
    let mut done = Done { skipped: plan.copies.len() - pending.len(), ..Done::default() };
    let swept: HashSet<&Path> = pending.iter().filter_map(|(_, to)| to.parent()).collect();
    for dir in swept {
        sweep(dir);
    }
    let mut written: Vec<PathBuf> = Vec::new();
    for (n, (from, to)) in pending.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            done.stopped = true;
            break;
        }
        let name = from.file_name().unwrap_or_default().to_string_lossy().to_string();
        if to.exists() && !owns(&plan.owner, to) {
            let _ = tx.send(Msg::Log(format!("export: {} is already on the device and not from this export  ·  left alone", to.display())));
            done.failed.push(name);
            continue;
        }
        let _ = tx.send(Msg::Stage(format!("exporting  {}/{}", n + 1, pending.len())));
        match put(from, to, reencoded(to)) {
            Ok(()) => {
                done.copied += 1;
                written.push(to.clone());
            }
            Err(err) => {
                let _ = tx.send(Msg::Log(format!("export: {err:#}")));
                done.failed.push(name);
            }
        }
    }
    if !done.stopped {
        for (path, text) in &plan.playlists {
            match write_text(path, text, &plan.owner) {
                Ok(()) => written.push(path.clone()),
                Err(err) => {
                    let _ = tx.send(Msg::Log(format!("export: {err:#}")));
                    done.failed.push(path.file_name().unwrap_or_default().to_string_lossy().to_string());
                }
            }
        }
    }
    claim(plan, &written, tx);
    Ok(done)
}

// Records what was written, replacing this owner's older line for the same name and dropping its lines
// for files that have gone. Other owners' lines are left exactly as they were.
fn claim(plan: &Plan, written: &[PathBuf], tx: &Sender<Msg>) {
    for home in touched(plan) {
        let mine: Vec<String> = written
            .iter()
            .filter(|p| p.parent() == Some(home.as_path()))
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .collect();
        let mut lines = read_record(&home);
        let before = lines.len();
        lines.retain(|(o, n)| *o != plan.owner || (home.join(n).exists() && !mine.iter().any(|m| key(m) == key(n))));
        let changed = lines.len() != before || !mine.is_empty();
        lines.extend(mine.into_iter().map(|n| (plan.owner.clone(), n)));
        if changed {
            if let Err(err) = write_record(&home, &lines) {
                let _ = tx.send(Msg::Log(format!("export: could not record what was written in {}: {err:#}", home.display())));
            }
        }
    }
}

// What a mirror would remove: files this export wrote on an earlier run and no longer plans. Compared as
// the device compares names, or a rename that only changes case would offer the file it just wrote.
pub fn leftovers(plan: &Plan) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for home in touched(plan) {
        let planned: HashSet<String> = plan
            .copies
            .iter()
            .map(|(_, to)| to)
            .chain(plan.playlists.iter().map(|(p, _)| p))
            .filter(|p| p.parent() == Some(home.as_path()))
            .filter_map(|p| p.file_name().map(|n| key(&n.to_string_lossy())))
            .collect();
        let record = read_record(&home);
        let theirs: HashSet<String> = record.iter().filter(|(o, _)| *o != plan.owner).map(|(_, n)| key(n)).collect();
        for (owner, name) in &record {
            let path = home.join(name);
            if *owner == plan.owner && !planned.contains(&key(name)) && !theirs.contains(&key(name)) && path.is_file() {
                found.push(path);
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Removes `gone` from the device, after the caller has asked. Each must be recorded as this export's own.
pub fn remove(plan: &Plan, dest: &Path, gone: &[PathBuf], tx: &Sender<Msg>) -> usize {
    let root = dest.canonicalize().unwrap_or_else(|_| dest.to_path_buf());
    let mut removed = 0;
    for path in gone {
        let inside = path.parent().is_some_and(|p| touched(plan).iter().any(|h| h == p))
            && owns(&plan.owner, path)
            && path.canonicalize().is_ok_and(|real| real.starts_with(&root));
        if !inside {
            let _ = tx.send(Msg::Log(format!("export: {} is not recorded as this export's  ·  left alone", path.display())));
            continue;
        }
        match std::fs::remove_file(path) {
            Ok(()) => {
                removed += 1;
                if let (Some(home), Some(name)) = (path.parent(), path.file_name()) {
                    let want = key(&name.to_string_lossy());
                    let mut lines = read_record(home);
                    lines.retain(|(o, n)| *o != plan.owner || key(n) != want);
                    let _ = write_record(home, &lines);
                }
            }
            Err(err) => {
                let _ = tx.send(Msg::Log(format!("export: {}: {err}", path.display())));
            }
        }
    }
    removed
}

fn said(done: &Done, plan: &Plan, to: &Path) -> String {
    let mut parts = vec![format!("{} copied", done.copied)];
    if done.skipped > 0 {
        parts.push(format!("{} already there", done.skipped));
    }
    if plan.missing > 0 {
        parts.push(format!("{} not on disk", plan.missing));
    }
    if !done.failed.is_empty() {
        parts.push(format!("{} failed  ·  l has why", done.failed.len()));
    }
    let head = if done.stopped { "export stopped" } else { "exported" };
    format!("{head} to {}  ·  {}", to.display(), parts.join("  ·  "))
}

fn destination(asker: &Asker, dir: &Path) -> Result<Option<PathBuf>> {
    let found = mounts();
    let note = if found.is_empty() {
        "a folder on a stick, card or synced drive  ·  only files earworm put there are ever offered for removal".to_string()
    } else {
        format!("a folder on a stick, card or synced drive  ·  mounted: {}", found.join("  "))
    };
    let Some(answer) = asker.input_noted("Export to", &note, "", crate::app::Escape::Keep) else {
        return Ok(None);
    };
    let answer = answer.trim();
    if answer.is_empty() {
        return Ok(None);
    }
    let path = match answer.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest),
        None => PathBuf::from(answer),
    };
    check(&path, dir)?;
    Ok(Some(path))
}

fn check(path: &Path, dir: &Path) -> Result<()> {
    if !path.is_absolute() {
        bail!("{} is not a full path  ·  start it with / or ~/", path.display());
    }
    if !path.is_dir() {
        bail!("{} is not a folder  ·  make it first, so a typo does not make a new one", path.display());
    }
    let library = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let real = path.canonicalize()?;
    if real.starts_with(&library) {
        bail!("that is inside the library  ·  pick somewhere else");
    }
    if library.starts_with(&real) {
        bail!("the library is inside that  ·  pick a folder of its own");
    }
    Ok(())
}

// Every name goes in the note, which wraps: the header clips, and nobody can answer a question about
// deleting files without being told which.
fn asked_to_remove(asker: &Asker, gone: &[PathBuf]) -> bool {
    let names: Vec<String> = gone.iter().map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string()).collect();
    let count = names.len();
    let files = if count == 1 { "file" } else { "files" };
    let note = format!("{}  ·  no longer in this export, and this cannot be undone", names.join(", "));
    let options = vec!["keep them".to_string(), format!("remove {count} {files}")];
    asker.choose_noted(&format!("Remove {count} {files} from the device?"), &note, options) == Some(1)
}

pub fn run(dir: &Path, tx: &Sender<Msg>, asker: &Asker, cancel: &AtomicBool, target: &Target) -> Result<()> {
    let list = match target {
        Target::List(name) => Some(
            playlists::load(dir, name).with_context(|| format!("no playlist called \"{name}\"  ·  it may have been deleted"))?,
        ),
        Target::Folder(folder) => {
            if !folder.is_dir() {
                bail!("{} is not a folder", folder.display());
            }
            None
        }
    };
    let Some(dest) = destination(asker, dir)? else {
        return Ok(());
    };
    let Some(form) = asker.choose_noted(
        "Spell accented names",
        "iPhone compares bytes and needs decomposed  ·  most other players want composed",
        vec!["Composed  ·  Android, Windows, most players".into(), "Decomposed  ·  iPhone".into()],
    ) else {
        return Ok(());
    };
    let form = if form == 0 { Form::Composed } else { Form::Decomposed };
    let mut options = vec!["As they are".to_string()];
    options.extend(config::FORMATS.iter().map(|(name, ext, _)| format!("{name}  ·  .{ext}")));
    let Some(pick) = asker.choose_noted(
        "Audio format on the device",
        "for a player that cannot read what is on disk  ·  a converted copy is a generation further from the source",
        options,
    ) else {
        return Ok(());
    };
    let format = pick.checked_sub(1).and_then(|i| config::FORMATS.get(i)).map(|(name, _, _)| *name);
    let plan = match (&list, target) {
        (Some(list), _) => {
            let Some(layout) = asker.choose_noted(
                "Lay the playlist out",
                "copies repeat audio the folders already hold",
                vec![
                    "One folder of copies  ·  plays anywhere".into(),
                    "Each track in its own folder  ·  no repeats".into(),
                ],
            ) else {
                return Ok(());
            };
            let layout = if layout == 0 { Layout::Copies } else { Layout::Folders };
            let files = worker::resolve_entries(&Others::everything(dir), dir, &list.entries);
            plan_list(dir, list, &files, &dest, form, layout, format)
        }
        (None, Target::Folder(folder)) => plan_folder(folder, &dest, form, format),
        (None, Target::List(_)) => unreachable!("a list target always loads"),
    };
    if plan.copies.is_empty() {
        bail!("nothing on disk to export");
    }
    let gone = leftovers(&plan);
    let mirror = !gone.is_empty() && asked_to_remove(asker, &gone);
    let done = execute(&plan, tx, cancel)?;
    let removed = if mirror && !done.stopped { remove(&plan, &dest, &gone, tx) } else { 0 };
    let mut flash = said(&done, &plan, &dest);
    if removed > 0 {
        flash.push_str(&format!("  ·  {removed} removed"));
    }
    let _ = tx.send(Msg::Flash(flash));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playlists::Entry;
    use std::sync::mpsc::channel;

    fn root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("earworm-export-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn folder(dir: &Path, name: &str, files: &[&str]) -> PathBuf {
        let folder = dir.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        for f in files {
            std::fs::write(folder.join(f), format!("audio {f}")).unwrap();
        }
        let list: Vec<String> = files.iter().map(|f| f.to_string()).collect();
        std::fs::write(folder.join(format!("{name}.m3u8")), m3u8(&list)).unwrap();
        folder
    }

    fn visible(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort();
        names
    }

    fn run_plan(plan: &Plan) -> Done {
        let (tx, _rx) = channel();
        execute(plan, &tx, &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn two_folders_and_a_list_land_on_the_device_and_a_second_export_copies_nothing() {
        let lib = root("lib");
        let a = folder(&lib, "A", &["01 - One.opus", "02 - Two.opus"]);
        let b = folder(&lib, "B", &["01 - Three.opus"]);
        let dest = root("dev");
        let list = Playlist {
            name: "Mix".into(),
            entries: vec![Entry { id: "x".into(), hint: PathBuf::new() }, Entry { id: "y".into(), hint: PathBuf::new() }],
        };
        let files = vec![Some(b.join("01 - Three.opus")), Some(a.join("02 - Two.opus"))];
        let plans = [
            plan_folder(&a, &dest, Form::Composed, None),
            plan_folder(&b, &dest, Form::Composed, None),
            plan_list(&lib, &list, &files, &dest, Form::Composed, Layout::Copies, None),
        ];
        let copied: usize = plans.iter().map(|p| run_plan(p).copied).sum();
        assert_eq!(copied, 5);
        assert!(dest.join("A/01 - One.opus").is_file() && dest.join("B/01 - Three.opus").is_file());
        assert_eq!(std::fs::read_to_string(dest.join("Mix/Mix.m3u8")).unwrap(), "#EXTM3U\n01 - Three.opus\n02 - Two.opus\n");
        assert_eq!(std::fs::read_to_string(dest.join("A/A.m3u8")).unwrap(), "#EXTM3U\n01 - One.opus\n02 - Two.opus\n");

        let again: Vec<Done> = plans.iter().map(run_plan).collect();
        assert!(again.iter().all(|d| d.copied == 0), "a second export copied {again:?}");
        assert_eq!(again.iter().map(|d| d.skipped).sum::<usize>(), 5);
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn a_track_named_with_a_question_mark_lands_under_a_safe_name_the_playlist_uses() {
        let lib = root("safe");
        let a = folder(&lib, "A", &["01 - What?.opus"]);
        let dest = root("safe-dev");
        let plan = plan_folder(&a, &dest, Form::Composed, None);
        run_plan(&plan);
        assert!(dest.join("A/01 - What_.opus").is_file());
        assert_eq!(std::fs::read_to_string(dest.join("A/A.m3u8")).unwrap(), "#EXTM3U\n01 - What_.opus\n");
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn names_that_collapse_to_one_stay_two_files() {
        let lib = root("clash");
        let a = folder(&lib, "A", &["a?b.opus", "a*b.opus"]);
        let dest = root("clash-dev");
        let plan = plan_folder(&a, &dest, Form::Composed, None);
        assert_eq!(run_plan(&plan).copied, 2);
        assert_eq!(visible(&dest.join("A")).len(), 3, "one track overwrote the other");
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn a_playlist_kept_in_its_folders_points_into_them_from_the_root() {
        let lib = root("folders");
        let a = folder(&lib, "A", &["01.opus"]);
        let dest = root("folders-dev");
        let list = Playlist { name: "Mix".into(), entries: vec![Entry { id: "x".into(), hint: PathBuf::new() }] };
        let plan = plan_list(&lib, &list, &[Some(a.join("01.opus")), None], &dest, Form::Composed, Layout::Folders, None);
        assert_eq!(plan.missing, 1);
        run_plan(&plan);
        assert_eq!(std::fs::read_to_string(dest.join("Mix.m3u8")).unwrap(), "#EXTM3U\nA/01.opus\n");
        assert!(dest.join("A/01.opus").is_file());
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn a_stopped_export_writes_no_playlist_and_leaves_no_scratch_file() {
        let lib = root("stop");
        let a = folder(&lib, "A", &["01.opus", "02.opus"]);
        let dest = root("stop-dev");
        let plan = plan_folder(&a, &dest, Form::Composed, None);
        let (tx, _rx) = channel();
        let done = execute(&plan, &tx, &AtomicBool::new(true)).unwrap();
        assert!(done.stopped && done.copied == 0);
        assert!(!dest.join("A/A.m3u8").exists());
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn a_leftover_scratch_file_is_swept_and_a_real_track_is_not() {
        let dest = root("sweep");
        std::fs::write(dest.join(format!("{SCRATCH}01.opus")), "half").unwrap();
        std::fs::write(dest.join("01.opus"), "whole").unwrap();
        sweep(&dest);
        assert!(!dest.join(format!("{SCRATCH}01.opus")).exists());
        assert!(dest.join("01.opus").exists());
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn a_converted_export_keeps_the_lyrics_and_the_isrc() {
        let lib = root("extras");
        let dest = root("extras-dev");
        let src = lib.join("A");
        std::fs::create_dir_all(&src).unwrap();
        let opus = src.join("01 - Song.opus");
        assert!(crate::worker::tests::tiny_opus(&opus).is_some(), "this test needs ffmpeg with libopus");
        crate::tag::set_fields(&opus, &crate::tag::Fields { artist: "A".into(), title: "Song".into(), album: String::new(), year: None }).unwrap();
        crate::tag::set_extra(&opus, crate::tag::Extra::Lyrics, "the words").unwrap();
        crate::tag::set_isrc(&opus, "GBAYE9200070").unwrap();
        let plan = plan_folder(&src, &dest, Form::Composed, Some("flac"));
        assert_eq!(run_plan(&plan).copied, 1);
        let landed = dest.join("A/01 - Song.flac");
        assert_eq!(crate::tag::extra(&landed, crate::tag::Extra::Lyrics).unwrap().as_deref(), Some("the words"));
        assert_eq!(crate::tag::read(&landed).unwrap().isrc.as_deref(), Some("GBAYE9200070"));
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn a_second_export_in_another_codec_under_one_extension_is_redone() {
        let lib = root("codec");
        let dest = root("codec-dev");
        let src = lib.join("A");
        std::fs::create_dir_all(&src).unwrap();
        let wav = src.join("01 - Tone.wav");
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=duration=1"])
            .arg(&wav)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(made, "this test needs ffmpeg, like the transcode tests");
        let list = Playlist { name: "Mix".into(), entries: vec![Entry { id: "x".into(), hint: PathBuf::new() }] };
        let export = |format| plan_list(&lib, &list, &[Some(wav.clone())], &dest, Form::Composed, Layout::Copies, Some(format));
        assert_eq!(run_plan(&export("alac")).copied, 1);
        // Both land on `.m4a`, so only the codec can say the file there is the wrong one.
        let again = run_plan(&export("m4a"));
        assert_eq!((again.copied, again.skipped), (1, 0), "alac was kept as m4a: {again:?}");
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn a_destination_must_be_an_existing_folder_outside_the_library() {
        let lib = root("inside");
        let out = root("outside");
        std::fs::create_dir_all(lib.join("A")).unwrap();
        assert!(check(&lib.join("A"), &lib).is_err(), "inside the library");
        assert!(check(&lib.join("nope"), &lib).is_err(), "not a folder");
        assert!(check(Path::new("relative"), &lib).is_err(), "not a full path");
        assert!(check(&out, &lib).is_ok());
        assert!(check(lib.parent().unwrap(), &lib).is_err(), "a folder that holds the library");
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&out);
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        paths.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect()
    }

    #[test]
    fn a_mirror_offers_only_what_this_export_recorded_and_no_longer_plans() {
        let lib = root("mirror");
        let a = folder(&lib, "A", &["01.opus", "02.opus"]);
        let dest = root("mirror-dev");
        run_plan(&plan_folder(&a, &dest, Form::Composed, None));
        let home = dest.join("A");
        // None of these is in the record, so none is the export's to remove.
        for mine in ["notes.txt", "09 - Gone.opus", "Old.m3u8", "._01.opus"] {
            std::fs::write(home.join(mine), "not earworm's").unwrap();
        }
        std::fs::write(dest.join("Other.opus"), "not ours").unwrap();

        std::fs::remove_file(a.join("01.opus")).unwrap();
        let plan = plan_folder(&a, &dest, Form::Composed, None);
        let gone = leftovers(&plan);
        assert_eq!(names(&gone), ["01.opus"], "{gone:?}");

        let (tx, _rx) = channel();
        assert_eq!(remove(&plan, &dest, &gone, &tx), 1);
        assert!(!home.join("01.opus").exists());
        for kept in ["notes.txt", "09 - Gone.opus", "Old.m3u8"] {
            assert!(home.join(kept).exists(), "{kept} was removed");
        }
        assert!(dest.join("Other.opus").exists());
        // A path the record never named is refused even when it is passed in.
        assert_eq!(remove(&plan, &dest, &[home.join("09 - Gone.opus")], &tx), 0);
        assert!(leftovers(&plan).is_empty(), "the removed file is still recorded");
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&dest);
    }

    // The blocker from the review: a name that differs only in case is the file just written to the device.
    #[test]
    fn a_rename_that_changes_only_case_or_accent_form_is_never_offered_for_removal() {
        let lib = root("case");
        let a = folder(&lib, "A", &["a song.opus", "Cafe\u{301}.opus"]);
        let dest = root("case-dev");
        run_plan(&plan_folder(&a, &dest, Form::Composed, None));

        std::fs::rename(a.join("a song.opus"), a.join("A Song.opus")).unwrap();
        // The same tracks again, spelled the other way: what the review reproduced on APFS.
        let plan = plan_folder(&a, &dest, Form::Decomposed, None);
        let gone = leftovers(&plan);
        assert!(gone.is_empty(), "offered a file the plan still writes: {gone:?}");
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn a_device_folder_earworm_did_not_write_is_neither_offered_nor_overwritten() {
        let lib = root("unrelated");
        let a = folder(&lib, "Music", &["01.opus"]);
        let dest = root("unrelated-dev");
        std::fs::create_dir_all(dest.join("Music")).unwrap();
        std::fs::write(dest.join("Music/01.opus"), "somebody else's track").unwrap();
        std::fs::write(dest.join("Music/Mine.opus"), "and another").unwrap();

        let plan = plan_folder(&a, &dest, Form::Composed, None);
        assert!(leftovers(&plan).is_empty(), "offered a file nothing recorded");
        let done = run_plan(&plan);
        assert!(done.failed.contains(&"01.opus".to_string()), "{done:?}");
        assert_eq!(std::fs::read_to_string(dest.join("Music/01.opus")).unwrap(), "somebody else's track");
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn a_list_and_a_folder_of_one_name_never_offer_each_others_files() {
        let lib = root("shared");
        let a = folder(&lib, "Focus", &["01.opus"]);
        let b = folder(&lib, "Other", &["02.opus"]);
        let dest = root("shared-dev");
        let list = Playlist { name: "Focus".into(), entries: vec![Entry { id: "x".into(), hint: PathBuf::new() }] };
        let folder_plan = plan_folder(&a, &dest, Form::Composed, None);
        let list_plan = plan_list(&lib, &list, &[Some(b.join("02.opus"))], &dest, Form::Composed, Layout::Copies, None);
        run_plan(&folder_plan);
        run_plan(&list_plan);
        assert!(leftovers(&folder_plan).is_empty() && leftovers(&list_plan).is_empty());

        // The list drops its track: only the list's own file is ever its to offer.
        let empty = plan_list(&lib, &list, &[], &dest, Form::Composed, Layout::Copies, None);
        assert_eq!(names(&leftovers(&empty)), ["02.opus"]);
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn a_list_laid_out_in_folders_names_tracks_as_the_folders_own_export_does() {
        let lib = root("same-name");
        let a = folder(&lib, "A", &["a?b.opus", "a*b.opus"]);
        let dest = root("same-name-dev");
        run_plan(&plan_folder(&a, &dest, Form::Composed, None));
        let before = std::fs::read(dest.join("A/a_b.opus")).unwrap();

        let list = Playlist { name: "Mix".into(), entries: vec![Entry { id: "x".into(), hint: PathBuf::new() }] };
        let plan = plan_list(&lib, &list, &[Some(a.join("a*b.opus"))], &dest, Form::Composed, Layout::Folders, None);
        assert_eq!(plan.copies[0].1, dest.join("A/a_b (2).opus"), "the folder's own name for it");
        run_plan(&plan);
        assert_eq!(std::fs::read(dest.join("A/a_b.opus")).unwrap(), before, "the other track was overwritten");
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn a_folder_layout_list_can_be_mirrored_without_touching_the_folders_own_files() {
        let lib = root("mirror-folders");
        let a = folder(&lib, "A", &["01.opus", "02.opus"]);
        let dest = root("mirror-folders-dev");
        run_plan(&plan_folder(&a, &dest, Form::Composed, None));
        let list = Playlist { name: "Mix".into(), entries: vec![Entry { id: "x".into(), hint: PathBuf::new() }, Entry { id: "y".into(), hint: PathBuf::new() }] };
        let both = [Some(a.join("01.opus")), Some(a.join("02.opus"))];
        run_plan(&plan_list(&lib, &list, &both, &dest, Form::Composed, Layout::Folders, None));

        let fewer = plan_list(&lib, &list, &both[..1], &dest, Form::Composed, Layout::Folders, None);
        // 02.opus is on the device from the folder's own export as well, so the list cannot claim it.
        assert!(leftovers(&fewer).is_empty(), "{:?}", leftovers(&fewer));
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn a_converted_track_takes_the_new_extension_and_a_second_export_leaves_it() {
        let lib = root("convert");
        let dest = root("convert-dev");
        let src = lib.join("A");
        std::fs::create_dir_all(&src).unwrap();
        let wav = src.join("01 - Tone.wav");
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=duration=1"])
            .arg(&wav)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(made, "this test needs ffmpeg, like the transcode tests");
        let list = Playlist { name: "Mix".into(), entries: vec![Entry { id: "x".into(), hint: PathBuf::new() }] };
        let plan = plan_list(&lib, &list, &[Some(wav.clone())], &dest, Form::Composed, Layout::Copies, Some("flac"));
        assert_eq!(run_plan(&plan).copied, 1);
        assert!(dest.join("Mix/01 - Tone.flac").is_file());
        assert_eq!(std::fs::read_to_string(dest.join("Mix/Mix.m3u8")).unwrap(), "#EXTM3U\n01 - Tone.flac\n");
        let again = run_plan(&plan);
        assert_eq!((again.copied, again.skipped), (0, 1), "a converted file is a different size and was redone");
        let _ = std::fs::remove_dir_all(&lib);
        let _ = std::fs::remove_dir_all(&dest);
    }
}
