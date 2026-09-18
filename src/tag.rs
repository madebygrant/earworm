use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::items::Timestamp;
use lofty::tag::{Accessor, Tag, TagExt};

use std::time::Duration;

use crate::app::{Asker, Status};
use crate::config::Config;
use crate::lookup::{self, Match};

/// One image through ffmpeg, which is a decode and an encode of a few
/// megabytes. Generous, since the alternative is leaving the art oversized.
const COVER_RESIZE: Duration = Duration::from_secs(20);

/* A whole track decoded and re-encoded, where the cover is a few megabytes of
   image. Long enough for a flac of a twenty-minute live set on a slow disk,
   because the cost of being wrong is a conversion that fails for no reason
   the user can see. */
const TRANSCODE: Duration = Duration::from_secs(300);

static WRITING: AtomicBool = AtomicBool::new(false);

/// True while a track's tags are being rewritten in place. Quitting during one
/// truncates the file, so the shutdown path waits this out.
pub fn writing() -> bool {
    WRITING.load(Ordering::SeqCst)
}

struct WriteGuard;

impl WriteGuard {
    fn new() -> Self {
        WRITING.store(true, Ordering::SeqCst);
        WriteGuard
    }
}

impl Drop for WriteGuard {
    fn drop(&mut self) {
        WRITING.store(false, Ordering::SeqCst);
    }
}

pub struct Info {
    pub artist: String,
    pub title: String,
    pub album: String,
    /// `None` is a file with no year tag, which is not the same as a year of
    /// zero: one is unknown and the other is a claim.
    pub year: Option<u16>,
    pub duration: u64,
}

/* What an edit writes. Album and year travel beside artist and title because
   `set_fields` writes the whole set: a caller changing one has to carry the
   others, or a swap would silently drop the album off every track it touched. */
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fields {
    pub artist: String,
    pub title: String,
    pub album: String,
    pub year: Option<u16>,
}

fn open(path: &Path) -> Result<TaggedFile> {
    Probe::open(path)
        .context("cannot open file")?
        .read()
        .context("cannot read audio")
}

pub fn read(path: &Path) -> Result<Info> {
    let file = open(path)?;
    let tag = file.primary_tag().or_else(|| file.first_tag());
    Ok(Info {
        artist: tag.and_then(|t| t.artist()).unwrap_or_default().to_string(),
        title: tag.and_then(|t| t.title()).unwrap_or_default().to_string(),
        album: tag.and_then(|t| t.album()).unwrap_or_default().to_string(),
        /* `date` reads the full recording date and falls back to a bare
           year, which is the only one of the two earworm ever writes. */
        year: tag.and_then(|t| t.date()).map(|d| d.year),
        duration: file.properties().duration().as_secs(),
    })
}

pub struct Outcome {
    pub status: Status,
    pub source: String,
    pub note: String,
    pub artist: String,
    pub title: String,
    pub mbid: Option<String>,
}

fn with_tag<F: FnOnce(&mut Tag)>(path: &Path, edit: F) -> Result<()> {
    let mut file = open(path)?;
    /* Taken from the container, never assumed: inserting Vorbis comments
       into an mp3 is refused outright by lofty, and the write then fails on
       any untagged file in a format other than opus or flac. */
    if file.primary_tag().is_none() {
        let kind = file.file_type().primary_tag_type();
        file.insert_tag(Tag::new(kind));
    }
    let _writing = WriteGuard::new();
    let tag = file.primary_tag_mut().context("no writable tag")?;
    edit(tag);
    tag.save_to_path(path, WriteOptions::default())
        .context("cannot write tags")
}

pub fn set_album(path: &Path, album: &str) -> Result<()> {
    with_tag(path, |tag| tag.set_album(album.to_string()))
}

pub fn set_fields(path: &Path, fields: &Fields) -> Result<()> {
    with_tag(path, |tag| {
        tag.set_artist(fields.artist.clone());
        tag.set_title(fields.title.clone());
        /* Removed rather than set to nothing: lofty writes an empty album as
           a present-but-blank frame, which reads back as a tagged file whose
           album happens to be "" and stops `tag_tracks` filling it in. */
        if fields.album.is_empty() {
            tag.remove_album();
        } else {
            tag.set_album(fields.album.clone());
        }
        /* A date with only a year in it, which is what the box asks for and
           what every source earworm has gives. Written through `set_date`
           because lofty has no year of its own: it keys the bare `Year` field
           and the full recording date to the same accessor. */
        match fields.year {
            Some(year) => tag.set_date(Timestamp {
                year,
                ..Timestamp::default()
            }),
            None => {
                tag.remove_date();
            }
        }
    })
}

/* `alac` and `m4a` both write `.m4a`, so the extension cannot say which one
   a file holds, and "already in the target format" is wrong in both
   directions without this: an AAC file sits untouched in a folder being
   brought to alac, and an alac file does the same in one going to m4a.

   Returns the `FORMATS` name, so callers compare it against `cfg.format`
   like any other. `None` is a file that is not MPEG-4 at all, or one whose
   codec lofty could not read, and the caller falls back to the extension:
   guessing here would rewrite a file on no evidence. */
pub fn mp4_format(path: &Path) -> Option<&'static str> {
    use lofty::config::ParseOptions;
    use lofty::mp4::{Mp4Codec, Mp4File};

    let mut file = std::fs::File::open(path).ok()?;
    // Parsed for the codec alone, which lives in the properties.
    let parsed = Mp4File::read_from(&mut file, ParseOptions::new()).ok()?;
    match parsed.properties().codec()? {
        Mp4Codec::ALAC => Some("alac"),
        Mp4Codec::AAC => Some("m4a"),
        // MP3 or FLAC in an MPEG-4 container is neither of the two formats
        // earworm writes here, and nothing should claim otherwise.
        _ => None,
    }
}

/// Embedding a PNG as image/jpeg produces a file players silently refuse to
/// show art for, so the bytes decide the mime type.
pub fn image_extension(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0xFF, 0xD8]) {
        Some("jpg")
    } else if data.starts_with(b"\x89PNG") {
        Some("png")
    } else {
        None
    }
}

/* No bigger than Deezer's `cover_xl`, which is the largest earworm would
   ever choose for itself: the cap is "not larger than our own best source"
   rather than a number to argue about. YouTube serves an Art Track's
   thumbnail at 2048 square, and `--embed-thumbnail` puts that in every file
   the lookup could not place, where it is several times the size of the
   audio it is attached to. */
pub const COVER_MAX: u16 = 1000;

/* A decode and a downscale to a few hundred pixels, with no encode on the end
   of it. Measured at about 40ms on a real track, so this is the bound for a
   machine under load rather than an expectation. Far tighter than
   `COVER_RESIZE`, because nothing is lost by giving up: the pane draws the
   metadata it already has and the row simply carries no picture. */
const COVER_READ: Duration = Duration::from_secs(3);

/// An embedded cover, decoded to exactly the pixels a pane has room for.
/* `rows` is pixel rows, never terminal rows: the pane draws two of them per
   cell with a half-block, so the caller asks for twice the height it has and
   `cell_rows` is what converts back. */
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Art {
    pub cols: u16,
    pub rows: u16,
    /// RGB triples, row-major, `cols * rows` of them.
    pub pixels: Vec<(u8, u8, u8)>,
}

impl Art {
    /* How many terminal rows this covers, which is half its pixel rows: a
       half-block carries the one above and the one below in a single cell.
       A method rather than a second field, because two numbers that have to
       agree is how they come to disagree — which they did, once, between the
       thread that asks for a picture and the pane that draws it. */
    pub fn cell_rows(&self) -> u16 {
        self.rows / 2
    }

    pub fn at(&self, col: u16, row: u16) -> (u8, u8, u8) {
        let at = usize::from(row) * usize::from(self.cols) + usize::from(col);
        self.pixels.get(at).copied().unwrap_or((0, 0, 0))
    }
}

/* The embedded cover as raw pixels, straight out of ffmpeg at the size asked
   for. One call rather than a lofty read and a decode: ffmpeg already knows
   how to find the picture in a tagged file, it is a required dependency, and
   this way nothing large is ever held in memory or written to disk.

   Cropped rather than padded, so a square pane is filled: cover art is
   square often enough that the crop is usually a no-op, and letterboxing a
   picture into a pane this small wastes the rows that make it legible. */
pub fn cover_art(path: &Path, cols: u16, rows: u16) -> Option<Art> {
    if cols == 0 || rows == 0 {
        return None;
    }
    /* Through a file, like `shrink`: `run_bounded` polls `try_wait` and drains
       nothing, so a pipe that fills its buffer hangs ffmpeg until the deadline
       kills it. A few kilobytes of rawvideo would fit today, which is exactly
       how that trap gets re-set by a later change to the size. */
    let dir = std::env::temp_dir().join(format!(
        "earworm-art-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let out = dir.join("art.raw");
    let _ = std::fs::remove_file(&out);

    let scale = format!(
        "scale={cols}:{rows}:force_original_aspect_ratio=increase,crop={cols}:{rows}"
    );
    let ran = lookup::run_bounded(
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(path)
            // The audio stream is not wanted and is the expensive half.
            .args(["-an", "-vf", &scale])
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24"])
            .arg(&out)
            // Same reason as `shrink`: an inherited stdin lets ffmpeg eat the
            // keys meant for the UI.
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()),
        COVER_READ,
    );
    let art = ran
        .filter(|o| o.status.success())
        .and_then(|_| std::fs::read(&out).ok())
        .filter(|raw| raw.len() == usize::from(cols) * usize::from(rows) * 3)
        .map(|raw| Art {
            cols,
            rows,
            pixels: raw.chunks_exact(3).map(|p| (p[0], p[1], p[2])).collect(),
        });
    let _ = std::fs::remove_dir_all(&dir);
    art
}

/// The embedded front cover, for carrying art across a rewrite or measuring
/// what is already there. `None` when the file has no picture.
pub fn read_cover(path: &Path) -> Option<Vec<u8>> {
    let file = open(path).ok()?;
    let tag = file.primary_tag().or_else(|| file.first_tag())?;
    let front = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first())?;
    Some(front.data().to_vec())
}

/* Shrinks an oversized embedded cover in place, and says whether it had to.
   Measured with `jpeg_size`, so a picture that is not a JPEG is left alone:
   the only ones that are not come from the `c` picker, where somebody chose
   the file on purpose. */
pub fn cap_cover(path: &Path) -> Result<bool> {
    let Some(image) = read_cover(path) else {
        return Ok(false);
    };
    let (w, h) = lookup::jpeg_size(&image);
    if w == 0 || (w <= COVER_MAX && h <= COVER_MAX) {
        return Ok(false);
    }
    let smaller = shrink(&image).context("could not resize the cover")?;
    set_cover(path, &smaller)?;
    Ok(true)
}

/* Through files rather than pipes: `run_bounded` polls `try_wait` and never
   writes to the child, so a piped stdin nobody closes would leave ffmpeg
   waiting for input that is not coming. */
fn shrink(image: &[u8]) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!(
        "earworm-cover-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let (from, to) = (dir.join("in.jpg"), dir.join("out.jpg"));
    let _ = std::fs::remove_file(&to);
    std::fs::write(&from, image).ok()?;

    let out = lookup::run_bounded(
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(&from)
            // Never upscales, and keeps whatever aspect the art came with.
            .args([
                "-vf",
                &format!("scale={COVER_MAX}:{COVER_MAX}:force_original_aspect_ratio=decrease"),
            ])
            .args(["-q:v", "3"])
            .arg(&to)
            // Same reason as `transcode`: an inherited stdin lets ffmpeg read
            // the terminal earworm holds in raw mode.
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()),
        COVER_RESIZE,
    );
    let smaller = out
        .filter(|o| o.status.success())
        .and_then(|_| std::fs::read(&to).ok())
        // A truncated write and a wrong format both fail here rather than
        // replacing good art with something no player will show.
        .filter(|bytes| image_extension(bytes) == Some("jpg"));
    let _ = std::fs::remove_dir_all(&dir);
    smaller
}

pub fn set_cover(path: &Path, image: &[u8]) -> Result<()> {
    let mime = match image_extension(image) {
        Some("png") => MimeType::Png,
        _ => MimeType::Jpeg,
    };
    with_tag(path, |tag| {
        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(
            Picture::unchecked(image.to_vec())
                .pic_type(PictureType::CoverFront)
                .mime_type(mime)
                .build(),
        );
    })
}

/* Re-encodes one track into `format`, leaving the source untouched: the
   caller is what decides the old file is safe to remove, and only once this
   output has been verified. Here rather than in a module of its own because
   ffmpeg-on-an-audio-file already lives beside `shrink`.

   `-vn` drops the embedded picture rather than letting ffmpeg carry it, and
   `-map_metadata` is left alone so anything earworm does not model survives.
   The tags that matter are written afterwards through lofty, which knows
   which tag type each container takes, where ffmpeg's field mapping across
   containers is inconsistent in exactly the way that has already cost a day. */
pub fn transcode(from: &Path, to: &Path, format: &str, source: Option<&str>) -> Result<()> {
    let lossless = crate::config::lossless(format);
    /* Not a pipe: `run_bounded` polls `try_wait` and drains nothing until the
       child has exited, so a stderr pipe that fills its buffer would hang
       ffmpeg until the deadline killed it. A file never fills. */
    let log = std::env::temp_dir().join(format!(
        "earworm-convert-{}-{:?}.log",
        std::process::id(),
        std::thread::current().id()
    ));
    let mut last = String::from("no encoder ran");
    /* The encoder's name is not the same on every ffmpeg build, so the list
       is tried in order. A build with neither is the error worth reporting. */
    for encoder in crate::config::encoders(format) {
        let _ = std::fs::remove_file(to);
        let errors = std::fs::File::create(&log).ok();
        let mut cmd = std::process::Command::new("ffmpeg");
        cmd.args(["-v", "error", "-y", "-i"])
            .arg(from)
            .args(["-vn", "-c:a", encoder]);
        /* Generous on purpose for a lossy target. The source is already lossy
           and whatever this throws away cannot be got back, where the cost of
           being too generous is only disk. */
        if !lossless {
            cmd.args(["-b:a", "192k"]);
        }
        /* A lossy source decodes to float, and ffmpeg then writes 24-bit
           flac or alac: twice the size to store the decoder's own rounding,
           since there was never more than 16 bits of anything in it. Only
           when the source is known and lossy, so a genuinely deeper file is
           never quietly flattened. The two encoders disagree about the
           spelling and each refuses the other's. */
        if lossless && source.is_some_and(|from| !crate::config::lossless(from)) {
            cmd.args(["-sample_fmt", if format == "alac" { "s16p" } else { "s16" }]);
        }
        cmd.arg(to)
            // Inherited, ffmpeg reads the terminal earworm has in raw mode and
            // eats the keys meant for the UI.
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(errors.map_or_else(std::process::Stdio::null, std::process::Stdio::from));

        let outcome = lookup::run_bounded(&mut cmd, TRANSCODE);
        let said = || {
            std::fs::read_to_string(&log)
                .ok()
                .and_then(|text| {
                    text.lines()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .map(|l| l.trim().to_string())
                })
                .unwrap_or_else(|| "ffmpeg said nothing".into())
        };
        match outcome {
            Some(out) if out.status.success() => {
                let _ = std::fs::remove_file(&log);
                return Ok(());
            }
            Some(_) => last = said(),
            None => last = format!("{encoder} did not finish within {}s", TRANSCODE.as_secs()),
        }
    }
    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(to);
    anyhow::bail!("{last}")
}

pub struct CoverState {
    pub written: bool,
}

pub fn identify(
    path: &Path,
    cfg: &Config,
    asker: &Asker,
    cover: &mut CoverState,
    index: usize,
) -> Result<Outcome> {
    let info = read(path)?;
    let mut out = Outcome {
        status: Status::NoMatch,
        source: String::new(),
        note: String::new(),
        artist: info.artist.clone(),
        title: info.title.clone(),
        mbid: None,
    };

    let mut found: Option<Match> = None;
    if cfg.lookup {
        out.status = Status::NoMatch;
        if let Some(key) = &cfg.acoustid_key {
            found = lookup::acoustid(path, key);
        }
        if found.is_none() {
            match lookup::deezer(&info.artist, &info.title) {
                Some(m) if lookup::plausible(&m, &info.artist, &info.title) => found = Some(m),
                Some(m) => {
                    out.status = Status::Weak;
                    out.source = m.source.into();
                    out.note = format!("rejected \"{}\"", m.title);
                }
                None => {}
            }
        }
        /* Apple is the fallback, not a second opinion: Deezer's plausible hit
        already won, and an implausible Deezer hit already owns the rejection
        note. */
        if found.is_none() && cfg.apple {
            match lookup::itunes(&info.artist, &info.title) {
                Some(m) if lookup::plausible(&m, &info.artist, &info.title) => found = Some(m),
                Some(m) if out.status == Status::NoMatch => {
                    out.status = Status::Weak;
                    out.source = m.source.into();
                    out.note = format!("rejected \"{}\"", m.title);
                }
                _ => {}
            }
        }
    } else {
        out.status = Status::Kept;
    }

    if let Some(m) = &found {
        out.status = Status::Ok;
        out.source = m.source.into();
        out.mbid = m.mbid.clone();
        if let Some(score) = m.score {
            out.note = format!("score {score:.2}");
        }
        let (artist, title, note) = apply(path, m, cfg, cover, asker, index, &info.artist)?;
        out.artist = artist;
        out.title = title;
        if !note.is_empty() {
            out.note = note;
        }
    }

    /* Unresolved tracks are the only ones worth asking about, and with
    --no-lookup that is all of them. */
    let unresolved = matches!(
        out.status,
        Status::NoMatch | Status::Weak | Status::Kept
    );
    if cfg.fix
        && asker.enabled
        && unresolved
        && let Some(typed) = manual(path, asker, &out.artist, &out.title, index, cfg.apple)
    {
        out.status = Status::Manual;
        out.source = "typed".into();
        out.note.clear();
        let (artist, title, _) = apply(path, &typed, cfg, cover, asker, index, "")?;
        out.artist = artist;
        out.title = title;
    }

    Ok(out)
}

/// The text-search half of `identify`, for words already worth searching
/// with: Deezer first, Apple fallback, the plausibility gate on both. No
/// fingerprint and no prompts; a miss is just `None`.
pub fn search(artist: &str, title: &str, apple: bool) -> Option<Match> {
    lookup::deezer(artist, title)
        .filter(|m| lookup::plausible(m, artist, title))
        .or_else(|| {
            apple
                .then(|| lookup::itunes(artist, title))
                .flatten()
                .filter(|m| lookup::plausible(m, artist, title))
        })
}

fn manual(
    path: &Path,
    asker: &Asker,
    artist: &str,
    title: &str,
    index: usize,
    apple: bool,
) -> Option<Match> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let answers = asker.form(
        &format!("track {index}"),
        &name,
        vec![
            ("artist".into(), artist.to_string()),
            ("title".into(), title.to_string()),
        ],
        crate::app::Escape::Skip,
    )?;
    let [new_artist, new_title] = answers.as_slice() else {
        return None;
    };
    let (new_artist, new_title) = (new_artist.clone(), new_title.clone());
    if new_artist.is_empty() || new_title.is_empty() {
        return None;
    }
    if new_artist == artist && new_title == title {
        return None;
    }
    /* A hand-typed artist and title make a far better search key than the video
    title did, so retry for album and artwork through the same search a `T`
    re-run would use. */
    let extra = search(&new_artist, &new_title, apple);
    Some(Match {
        title: new_title,
        artist: new_artist,
        album: extra.as_ref().and_then(|m| m.album.clone()),
        cover_url: extra.as_ref().and_then(|m| m.cover_url.clone()),
        mbid: None,
        score: None,
        source: "manual",
    })
}

/// Writes a lookup hit into the file: tags, embedded art, and the folder
/// image unless the caller already wrote one. Returns the artist and title
/// as written, plus why the artist differs when it does.
pub fn apply(
    path: &Path,
    m: &Match,
    cfg: &Config,
    cover: &mut CoverState,
    asker: &Asker,
    index: usize,
    old_artist: &str,
) -> Result<(String, String, String)> {
    let mut artist = m.artist.clone();
    let mut note = String::new();

    // Title and album are always taken; only the artist can lose information.
    if !old_artist.is_empty() && lookup::downgrades(&artist, old_artist) {
        let (chosen, why) = choose_artist(asker, index, old_artist, &m.artist);
        artist = chosen;
        note = why;
    }

    let jpeg = cfg
        .cover
        .then(|| lookup::cover_bytes(m, cfg.apple))
        .flatten();
    with_tag(path, |tag| {
        tag.set_title(m.title.clone());
        tag.set_artist(artist.clone());
        if let Some(album) = &m.album {
            tag.set_album(album.clone());
        }
        if let Some(data) = &jpeg {
            tag.remove_picture_type(PictureType::CoverFront);
            tag.push_picture(
                Picture::unchecked(data.clone())
                    .pic_type(PictureType::CoverFront)
                    .mime_type(MimeType::Jpeg)
                    .build(),
            );
        }
    })?;

    /* Folder image is written once per run, by whichever track resolves art
    first; every track still gets the image embedded. */
    if let (Some(data), false) = (&jpeg, cover.written)
        && let Some(dir) = path.parent()
    {
        std::fs::write(dir.join("cover.jpg"), data)?;
        cover.written = true;
    }

    Ok((artist, m.title.clone(), note))
}

/// Only reached on a real conflict, which keeps the prompt rare enough to be
/// worth answering. Unattended runs keep the fuller existing credit.
fn choose_artist(asker: &Asker, index: usize, old: &str, new: &str) -> (String, String) {
    let options = vec![
        format!("{old}  (existing)"),
        format!("{new}  (lookup)"),
        "type it myself".to_string(),
    ];
    match asker.choose(&format!("Track {index}: artist conflict"), options) {
        Some(1) => (new.to_string(), "chose lookup".into()),
        Some(2) => match asker.input("Artist", old) {
            Some(typed) if !typed.is_empty() => (typed, "typed artist".into()),
            _ => (old.to_string(), "kept artist".into()),
        },
        _ => (old.to_string(), "kept artist".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lookup;
    use std::path::{Path, PathBuf};

    /* Silent, untagged and in whatever container the codec implies, which is
       the case `with_tag` has to insert a tag for. Several candidates per
       format because the encoder's name is a property of the ffmpeg build:
       this one has no `libvorbis` and ships the native `vorbis` instead. */
    fn bare(at: &Path, codecs: &[&str]) -> Option<()> {
        codecs.iter().any(|codec| encode(at, codec)).then_some(())
    }

    fn encode(at: &Path, codec: &str) -> bool {
        let mut cmd = std::process::Command::new("ffmpeg");
        // Stereo: ffmpeg's own vorbis encoder refuses anything else.
        cmd.args(["-v", "error", "-f", "lavfi", "-i", "anullsrc=r=44100:cl=stereo"])
            .args(["-t", "0.2", "-map_metadata", "-1", "-c:a", codec])
            // That encoder is also marked experimental in some builds.
            .args(["-strict", "-2"])
            /* Otherwise ffmpeg stamps an encoder tag of its own and the file
               arrives already tagged, which is not the case under test. */
            .args(["-fflags", "+bitexact", "-flags:a", "+bitexact"]);
        // The only one of them that can be written with no tag at all.
        if codec == "libmp3lame" {
            cmd.args(["-id3v2_version", "0", "-write_id3v1", "0"]);
        }
        cmd.arg("-y")
            .arg(at)
            .status()
            .is_ok_and(|status| status.success())
    }

    /* A tag type the container does not take is refused outright, and a file
       with no tag at all is where the type has to be guessed: assuming Vorbis
       comments left an untagged mp3 or m4a unwritable while the download that
       produced it reported success. */
    #[test]
    fn writes_tags_into_every_container_earworm_downloads() {
        let dir = std::env::temp_dir().join(format!("earworm-tagtypes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Named rather than counted: every container but mp3 carries a tag
        // block by definition, so this says which fixture is doing the work.
        let mut untagged: Vec<&str> = Vec::new();
        /* Driven off FORMATS, so a format added to the table without a
           container earworm can tag fails here rather than at the end of
           somebody's download. */
        for (format, ext, codecs) in crate::config::FORMATS {
            let file: PathBuf = dir.join(format!("{format}.{ext}"));
            if bare(&file, codecs).is_none() {
                eprintln!("skipped {ext}: ffmpeg could not write it");
                continue;
            }
            use lofty::file::TaggedFileExt;
            if open(&file).unwrap().primary_tag().is_none() {
                untagged.push(format);
            }
            let fields = Fields {
                artist: "Kraftwerk".into(),
                title: "Autobahn".into(),
                album: "Autobahn".into(),
                year: Some(1974),
            };
            set_fields(&file, &fields).unwrap_or_else(|e| panic!("{ext}: {e}"));
            let back = read(&file).unwrap();
            assert_eq!((back.artist.as_str(), back.title.as_str()), ("Kraftwerk", "Autobahn"), "{ext}");
            /* Album and year go into the same tag block, so every container
               that takes the first two has to take these as well. */
            assert_eq!(back.album, "Autobahn", "{ext}");
            assert_eq!(back.year, Some(1974), "{ext}");
        }
        /* The branch under test only runs for a file with no tag, so a
           fixture that arrives tagged would pass while saying nothing. */
        assert!(!untagged.is_empty(), "every fixture came with a tag already");
        std::fs::remove_dir_all(&dir).unwrap();
    }


    /* `alac` and `m4a` both write `.m4a`, so without reading the codec a
       folder of one counts as already being the other and the conversion
       skips every file in it. */
    #[test]
    fn the_codec_tells_alac_and_m4a_apart_behind_one_extension() {
        let dir = std::env::temp_dir().join(format!("earworm-mp4codec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let aac = dir.join("aac.m4a");
        let alac = dir.join("alac.m4a");
        if bare(&aac, crate::config::encoders("m4a")).is_none()
            || bare(&alac, crate::config::encoders("alac")).is_none()
        {
            eprintln!("skipped: ffmpeg could not write both m4a codecs");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        // Same extension on both, so only the codec can be answering.
        assert_eq!(aac.extension(), alac.extension());
        assert_eq!(mp4_format(&aac), Some("m4a"));
        assert_eq!(mp4_format(&alac), Some("alac"));
        // Not an MPEG-4 file at all, where a guess would re-encode on nothing.
        let opus = dir.join("not.opus");
        if bare(&opus, crate::config::encoders("opus")).is_some() {
            assert_eq!(mp4_format(&opus), None);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* Driven off FORMATS so a format added to the table without a working
       encoder fails here rather than part-way through somebody's library. The
       tags are what the conversion exists to carry, and the container change
       is exactly where lofty's tag-type choice has to hold. */
    #[test]
    fn a_transcode_lands_in_the_target_container_and_keeps_its_tags() {
        let dir = std::env::temp_dir().join(format!("earworm-transcode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let source = dir.join("source.flac");
        if bare(&source, crate::config::encoders("flac")).is_none() {
            eprintln!("skipped: ffmpeg could not write the source");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let fields = Fields {
            artist: "Kraftwerk".into(),
            title: "Autobahn".into(),
            album: "Autobahn".into(),
            year: Some(1974),
        };
        set_fields(&source, &fields).unwrap();

        let mut ran = 0;
        for (format, ext, _) in crate::config::FORMATS {
            let out = dir.join(format!("{format}.{ext}"));
            if let Err(err) = transcode(&source, &out, format, Some("flac")) {
                eprintln!("skipped {format}: {err}");
                continue;
            }
            ran += 1;
            assert!(std::fs::metadata(&out).unwrap().len() > 0, "{format} is empty");
            /* ffmpeg carries the tags of its own accord for most containers,
               but not all of them and not the same fields, which is why the
               swap writes them again through lofty. Written here the same way
               so the assertion is about the container taking them. */
            set_fields(&out, &fields).unwrap_or_else(|e| panic!("{format}: {e}"));
            let back = read(&out).unwrap();
            assert_eq!(back.artist, "Kraftwerk", "{format}");
            assert_eq!(back.title, "Autobahn", "{format}");
            assert_eq!(back.album, "Autobahn", "{format}");
            assert_eq!(back.year, Some(1974), "{format}");
            // The container is the point: an .m4a that is still flac inside
            // would pass every tag assertion above.
            if ext == "m4a" {
                assert_eq!(mp4_format(&out), Some(format), "{format}");
            }
        }
        assert!(ran > 0, "no format transcoded, so this asserted nothing");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /* A lossy source decodes to float and ffmpeg then writes 24-bit flac,
       which is twice the size to store the decoder's own rounding. A file
       that really does carry more depth must still keep it, which is the
       half that makes this a rule rather than a blanket downgrade. */
    #[test]
    fn a_lossy_source_does_not_become_a_24_bit_lossless_file() {
        let dir = std::env::temp_dir().join(format!("earworm-depth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let depth = |path: &Path| -> Option<u32> {
            let out = std::process::Command::new("ffprobe")
                .args(["-v", "error", "-select_streams", "a:0"])
                .args(["-show_entries", "stream=bits_per_raw_sample"])
                .args(["-of", "default=noprint_wrappers=1:nokey=1"])
                .arg(path)
                .output()
                .ok()?;
            String::from_utf8_lossy(&out.stdout).trim().parse().ok()
        };

        // A 24-bit source, which is the case that must not be flattened.
        let deep = dir.join("deep.flac");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo"])
            .args(["-t", "0.3", "-c:a", "flac", "-sample_fmt", "s32", "-y"])
            .arg(&deep)
            .status();
        if !made.is_ok_and(|s| s.success()) || depth(&deep) != Some(24) {
            eprintln!("skipped: ffmpeg or ffprobe could not make a 24-bit fixture");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }

        // Lossless in, so the depth is real and stays.
        let kept = dir.join("kept.flac");
        transcode(&deep, &kept, "flac", Some("flac")).unwrap();
        assert_eq!(depth(&kept), Some(24), "a real 24-bit source was flattened");

        /* The same bytes described as having come from a lossy format, which
           is the only thing that changes: whatever depth ffmpeg reads off the
           decoder there is the decoder's, not the recording's. */
        let flat = dir.join("flat.flac");
        transcode(&deep, &flat, "flac", Some("opus")).unwrap();
        assert_eq!(depth(&flat), Some(16), "a lossy source still wrote 24-bit");

        // A target that is not lossless has no depth to argue about.
        let lossy = dir.join("lossy.opus");
        if transcode(&deep, &lossy, "opus", Some("flac")).is_ok() {
            assert!(lossy.is_file());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A failed transcode must leave nothing behind for the swap to adopt.
    #[test]
    fn a_transcode_that_fails_writes_no_output() {
        let dir = std::env::temp_dir().join(format!("earworm-badconv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("not-audio.flac");
        std::fs::write(&source, b"this is not a flac file").unwrap();
        let out = dir.join("out.opus");
        assert!(transcode(&source, &out, "opus", Some("flac")).is_err());
        assert!(!out.exists(), "a failed transcode left a file behind");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A square JPEG of `px`, the shape YouTube serves an Art Track's
    /// thumbnail in. `None` if ffmpeg is not installed.
    fn square_jpeg(at: &Path, px: u32) -> Option<()> {
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .arg(format!("testsrc=size={px}x{px}:duration=1:rate=1"))
            .args(["-frames:v", "1", "-y"])
            .arg(at)
            .status()
            .ok()
            .filter(|s| s.success())
            .map(|_| ())
    }

    /* YouTube serves an Art Track's thumbnail at 2048 square, and
       `--embed-thumbnail` puts it in every file the lookup could not place:
       on a real folder that was 2.9MB of art on 3.5MB of audio, more than a
       third of every file and the same image in all twelve. */
    #[test]
    fn an_oversized_cover_is_shrunk_and_a_small_one_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("earworm-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let track = dir.join("01 - A.opus");
        let big = dir.join("big.jpg");
        if crate::worker::tests::tiny_opus(&track).is_none() || square_jpeg(&big, 2048).is_none() {
            eprintln!("no ffmpeg, skipping");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let art = std::fs::read(&big).unwrap();
        assert_eq!(lookup::jpeg_size(&art).0, 2048, "the fixture is not 2048 wide");
        set_cover(&track, &art).unwrap();
        let before = std::fs::metadata(&track).unwrap().len();

        assert!(cap_cover(&track).unwrap(), "an oversized cover was left alone");

        let capped = read_cover(&track).expect("the cover went entirely");
        let (w, h) = lookup::jpeg_size(&capped);
        assert!(w <= COVER_MAX && h <= COVER_MAX, "still {w}x{h}");
        assert_eq!(w, COVER_MAX, "it shrank past the cap rather than to it");
        // Still a picture a player will show, not a truncated write.
        assert_eq!(image_extension(&capped), Some("jpg"));
        assert!(
            std::fs::metadata(&track).unwrap().len() < before,
            "the file did not get smaller"
        );
        /* Idempotent: a second pass has nothing to do, which is what keeps
           this off the cost of every later sync. */
        assert!(!cap_cover(&track).unwrap(), "it resized an already-capped cover");

        // Under the cap is left exactly as it was, bytes included.
        let small = dir.join("small.jpg");
        square_jpeg(&small, 600).unwrap();
        let art = std::fs::read(&small).unwrap();
        set_cover(&track, &art).unwrap();
        assert!(!cap_cover(&track).unwrap());
        assert_eq!(read_cover(&track).unwrap(), art, "a small cover was re-encoded");

        // And a file with no picture at all is not an error.
        let bare = dir.join("02 - B.opus");
        crate::worker::tests::tiny_opus(&bare).unwrap();
        assert!(read_cover(&bare).is_none());
        assert!(!cap_cover(&bare).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_image_types_by_magic_bytes() {
        assert_eq!(image_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\n"), Some("png"));
        assert_eq!(image_extension(b"GIF89a"), None);
        assert_eq!(image_extension(b"<html>"), None);
        assert_eq!(image_extension(b""), None);
    }
}


