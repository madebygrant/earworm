use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::items::Timestamp;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt, TagType};

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

const MEASURE: Duration = Duration::from_secs(120);

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

/// A tag beyond the identity fields, mapped to each container's native frame by lofty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extra {
    TrackGain,
    TrackPeak,
    AlbumGain,
    AlbumPeak,
    // Opus players read these instead of the ReplayGain pair.
    R128TrackGain,
    R128AlbumGain,
    Lyrics,
}

impl Extra {
    fn key(self, kind: TagType) -> ItemKey {
        match self {
            Extra::TrackGain => ItemKey::ReplayGainTrackGain,
            Extra::TrackPeak => ItemKey::ReplayGainTrackPeak,
            Extra::AlbumGain => ItemKey::ReplayGainAlbumGain,
            Extra::AlbumPeak => ItemKey::ReplayGainAlbumPeak,
            Extra::R128TrackGain => ItemKey::R128TrackGain,
            Extra::R128AlbumGain => ItemKey::R128AlbumGain,
            // ID3v2 has no frame for `Lyrics`, only the unsynchronised one.
            Extra::Lyrics if kind == TagType::Id3v2 => ItemKey::UnsyncLyrics,
            Extra::Lyrics => ItemKey::Lyrics,
        }
    }
}

pub fn extra(path: &Path, which: Extra) -> Result<Option<String>> {
    let file = open(path)?;
    let tag = file.primary_tag().or_else(|| file.first_tag());
    Ok(tag.and_then(|t| {
        let found = t.get_string(which.key(t.tag_type()));
        // Another tool may have written the Vorbis `UNSYNCEDLYRICS` spelling.
        let found = found.or_else(|| {
            (which == Extra::Lyrics).then(|| t.get_string(ItemKey::UnsyncLyrics)).flatten()
        });
        found.filter(|text| !text.is_empty()).map(str::to_string)
    }))
}

/// An empty value removes the tag, for the reason `set_fields` removes an empty album.
pub fn set_extra(path: &Path, which: Extra, value: &str) -> Result<()> {
    set_extras(path, &[(which, value.to_string())])
}

// One save for the lot, since each write rewrites the file.
pub fn set_extras(path: &Path, values: &[(Extra, String)]) -> Result<()> {
    let mut refused = None;
    with_tag(path, |tag| {
        for (which, value) in values {
            let key = which.key(tag.tag_type());
            if value.is_empty() {
                tag.remove_key(key);
            } else if !tag.insert_text(key, value.clone()) {
                refused = Some(*which);
            }
        }
    })?;
    // lofty drops a key the container has no frame for without saying so.
    if let Some(which) = refused {
        anyhow::bail!("this file's tag cannot hold {which:?}");
    }
    Ok(())
}

/// What ffmpeg's `ebur128` filter reports for a whole file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loudness {
    pub lufs: f64,
    pub peak_db: f64,
}

// Opus players compare against -23 LUFS and everything else against -18.
const R128_TARGET: f64 = -23.0;
const REPLAYGAIN_TARGET: f64 = -18.0;

// Reads the two numbers out of ffmpeg's closing summary, ignoring the per-100ms lines before it.
fn parse_loudness(log: &str) -> Option<Loudness> {
    // The last one: a tag in the file's own metadata dump can say "Summary:" too.
    let summary = log.rsplit("Summary:").next().filter(|_| log.contains("Summary:"))?;
    let number = |label: &str, unit: &str| -> Option<f64> {
        let line = summary.lines().map(str::trim).find(|l| l.starts_with(label))?;
        line.strip_prefix(label)?.trim().strip_suffix(unit)?.trim().parse().ok()
    };
    let (lufs, peak_db) = (number("I:", "LUFS")?, number("Peak:", "dBFS")?);
    // Silence reads "-inf", which parses as a number and means there is nothing to level.
    (lufs.is_finite() && peak_db.is_finite()).then_some(Loudness { lufs, peak_db })
}

pub fn measure(path: &Path) -> Result<Loudness> {
    // A file, not a pipe, for the reason `transcode` gives.
    let log = std::env::temp_dir().join(format!(
        "earworm-loudness-{}-{:?}.log",
        std::process::id(),
        std::thread::current().id()
    ));
    let errors = std::fs::File::create(&log).context("cannot write a scratch log")?;
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-nostats", "-hide_banner", "-i"])
        .arg(path)
        .args(["-vn", "-af", "ebur128=peak=true:framelog=quiet", "-f", "null", "-"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::from(errors));
    let outcome = lookup::run_bounded(&mut cmd, MEASURE);
    let text = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_file(&log);
    match outcome {
        Some(out) if out.status.success() => {
            parse_loudness(&text).context("no loudness in the file (silent, or too short to measure)")
        }
        Some(_) => anyhow::bail!("ffmpeg could not read the audio"),
        None => anyhow::bail!("measuring did not finish within {}s", MEASURE.as_secs()),
    }
}

// Opus keeps its gain in the R128 tags as a Q7.8 integer; every other format uses ReplayGain text.
fn gain_tags(loud: Loudness, opus: bool) -> Vec<(Extra, String)> {
    if opus {
        let q78 = ((R128_TARGET - loud.lufs) * 256.0).round() as i32;
        return vec![(Extra::R128TrackGain, q78.to_string())];
    }
    let peak = 10f64.powf(loud.peak_db / 20.0);
    vec![
        (Extra::TrackGain, format!("{:.2} dB", REPLAYGAIN_TARGET - loud.lufs)),
        (Extra::TrackPeak, format!("{peak:.6}")),
    ]
}

// Energy mean, weighted by length, of per-track integrated loudness: the closest an album can get without decoding it whole.
fn album_lufs(tracks: &[(f64, f64)]) -> Option<f64> {
    let total: f64 = tracks.iter().map(|(_, weight)| weight).sum();
    if tracks.is_empty() || total <= 0.0 {
        return None;
    }
    let energy: f64 = tracks.iter().map(|(lufs, weight)| weight * 10f64.powf(lufs / 10.0)).sum();
    Some(10.0 * (energy / total).log10())
}

// A track's measurement read back out of its gain tags, so the album pass decodes nothing.
fn stored_loudness(path: &Path) -> Result<Option<(f64, Option<f64>)>> {
    if let Some(q78) = extra(path, Extra::R128TrackGain)? {
        let q78: f64 = q78.parse().context("unreadable R128_TRACK_GAIN")?;
        return Ok(Some((R128_TARGET - q78 / 256.0, None)));
    }
    let Some(gain) = extra(path, Extra::TrackGain)? else {
        return Ok(None);
    };
    let gain: f64 = gain.trim_end_matches("dB").trim().parse().context("unreadable REPLAYGAIN_TRACK_GAIN")?;
    let peak = extra(path, Extra::TrackPeak)?.and_then(|p| p.parse().ok());
    Ok(Some((REPLAYGAIN_TARGET - gain, peak)))
}

/// Writes one album gain to every file that has a track gain. Returns how many
/// files changed and how many were skipped as unreadable; a file already
/// carrying the right value is left alone, so a rerun is tag reads only and a
/// track added later moves the whole album.
pub fn write_album_loudness(paths: &[&Path]) -> (usize, usize) {
    let mut skipped = 0;
    let mut measured = Vec::new();
    for path in paths {
        let found = stored_loudness(path).and_then(|found| match found {
            // A zero-length file would carry no weight, and one second keeps it in the mean.
            Some((lufs, peak)) => Ok(Some((*path, lufs, peak, (read(path)?.duration as f64).max(1.0)))),
            None => Ok(None),
        });
        match found {
            Ok(Some(track)) => measured.push(track),
            Ok(None) => {}
            Err(_) => skipped += 1,
        }
    }
    let weighted: Vec<(f64, f64)> = measured.iter().map(|(_, l, _, w)| (*l, *w)).collect();
    let Some(album) = album_lufs(&weighted) else {
        return (0, skipped);
    };
    let peak = measured.iter().filter_map(|(_, _, p, _)| *p).fold(None, |a: Option<f64>, p| Some(a.map_or(p, |a| a.max(p))));
    let mut changed = 0;
    for (path, _, _, _) in measured {
        let written = (|| -> Result<bool> {
            let opus = extra(path, Extra::R128TrackGain)?.is_some();
            let mut want = if opus {
                vec![(Extra::R128AlbumGain, (((R128_TARGET - album) * 256.0).round() as i32).to_string())]
            } else {
                vec![(Extra::AlbumGain, format!("{:.2} dB", REPLAYGAIN_TARGET - album))]
            };
            if let (false, Some(peak)) = (opus, peak) {
                want.push((Extra::AlbumPeak, format!("{peak:.6}")));
            }
            if want.iter().all(|(which, value)| extra(path, *which).ok().flatten().as_deref() == Some(value)) {
                return Ok(false);
            }
            set_extras(path, &want)?;
            Ok(true)
        })();
        match written {
            Ok(true) => changed += 1,
            Ok(false) => {}
            Err(_) => skipped += 1,
        }
    }
    (changed, skipped)
}

/// Measures the file and writes its track gain. `false` means it already had
/// one, so a rerun costs a tag read and no ffmpeg, and another tool's value stays.
pub fn write_loudness(path: &Path) -> Result<bool> {
    use lofty::file::FileType;
    let opus = open(path)?.file_type() == FileType::Opus;
    let marker = if opus { Extra::R128TrackGain } else { Extra::TrackGain };
    if extra(path, marker)?.is_some() {
        return Ok(false);
    }
    set_extras(path, &gain_tags(measure(path)?, opus))?;
    Ok(true)
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

/* A decode and a downscale with no encode on the end of it, measured at about
   40ms on a real track. The bound is for a machine under load rather than an
   expectation, and giving up costs nothing: the pane draws the words it
   already has and the row carries no picture. */
const COVER_READ: Duration = Duration::from_secs(3);

/* The embedded cover as raw RGB, at exactly the pixels the pane covers.
   One ffmpeg call rather than a lofty read and an image-crate decode: ffmpeg
   already knows how to find the picture in any container earworm writes, it is
   a required dependency, and this way no codec feature has to be switched on
   in `image` — the pixels go straight into an `RgbImage`.

   Cropped rather than padded, and asked for in the pane's own proportions
   rather than as a square: a cell is about twice as tall as it is wide, so a
   square request would be letterboxed back down by the resize and the pane
   would carry fewer pixels than it has room for. */
pub fn cover_rgb(path: &Path, width: u32, height: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 {
        return None;
    }
    /* Through a file, like `shrink`: `run_bounded` polls `try_wait` and drains
       nothing, so a pipe that fills its buffer hangs ffmpeg until the deadline
       kills it. */
    let dir = std::env::temp_dir().join(format!(
        "earworm-art-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let out = dir.join("art.raw");
    let _ = std::fs::remove_file(&out);

    let scale = format!(
        "scale={width}:{height}:force_original_aspect_ratio=increase:flags=lanczos,crop={width}:{height}"
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
    let raw = ran
        .filter(|o| o.status.success())
        .and_then(|_| std::fs::read(&out).ok())
        .filter(|raw| raw.len() as u32 == width * height * 3);
    let _ = std::fs::remove_dir_all(&dir);
    raw
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
        let said = || last_line(&log);
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

// What ffmpeg last wrote to a scratch log, which is the line that names the failure.
fn last_line(log: &Path) -> String {
    std::fs::read_to_string(log)
        .ok()
        .and_then(|text| {
            text.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(|l| l.trim().to_string())
        })
        .unwrap_or_else(|| "ffmpeg said nothing".into())
}

// One chapter is seconds to a few minutes of audio, so well inside what a whole transcode is allowed.
const CUT: Duration = Duration::from_secs(120);

/* One chapter's span of a downloaded video, in its own file and the same
   format. A copy for everything lossy, which is exact to a frame and adds no
   generation. Not for flac or alac: a file with no seek table ignores the
   range under a copy and writes the whole thing out, so those re-encode, which
   costs nothing there. The length is checked afterwards for that reason, since
   ffmpeg exits 0 on the bad case. */
pub fn cut(from: &Path, to: &Path, start: f64, end: f64, format: &str) -> Result<()> {
    let log = std::env::temp_dir().join(format!(
        "earworm-cut-{}-{:?}.log",
        std::process::id(),
        std::thread::current().id()
    ));
    let encoders: Vec<Option<&str>> = if crate::config::lossless(format) {
        crate::config::encoders(format).iter().map(|e| Some(*e)).collect()
    } else {
        vec![None]
    };
    let mut last = String::from("no encoder ran");
    for encoder in encoders {
        let _ = std::fs::remove_file(to);
        let errors = std::fs::File::create(&log).ok();
        let mut cmd = std::process::Command::new("ffmpeg");
        cmd.args(["-v", "error", "-y"])
            .args(["-ss", &format!("{start:.3}"), "-to", &format!("{end:.3}")])
            .arg("-i")
            .arg(from)
            // The video's own tags and chapter list are not this track's: lofty writes the real tags afterwards.
            .args(["-vn", "-map_metadata", "-1", "-map_chapters", "-1"])
            .args(["-c:a", encoder.unwrap_or("copy")])
            .arg(to)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(errors.map_or_else(std::process::Stdio::null, std::process::Stdio::from));
        match lookup::run_bounded(&mut cmd, CUT) {
            Some(out) if out.status.success() => {
                let got = read(to).map(|info| info.duration as f64).unwrap_or(0.0);
                // The length reads in whole seconds, so a second is rounding; the rest is the codec's frame.
                if (got - (end - start)).abs() <= 1.0 + (end - start) * 0.02 {
                    let _ = std::fs::remove_file(&log);
                    return Ok(());
                }
                last = format!("cut came out {got:.0}s long, not {:.0}s", end - start);
            }
            Some(_) => last = last_line(&log),
            None => last = format!("cutting did not finish within {}s", CUT.as_secs()),
        }
    }
    let _ = std::fs::remove_file(&log);
    let _ = std::fs::remove_file(to);
    anyhow::bail!("{last}")
}

#[derive(Default)]
pub struct CoverState {
    pub written: bool,
    /// Whether this folder's tracks share one sleeve, which an album's do.
    pub one_sleeve: bool,
    /// That sleeve, once a track has resolved one.
    pub shared: Option<Vec<u8>>,
    /// The album name the folder settled on, under the same rule.
    pub album: Option<String>,
}

/// The art for one track, which in an album is the folder's one sleeve.
/* An album's tracks share a sleeve, so looking each one up separately returns
   whatever release that track happened to match: a folder of one record ends
   up holding three covers, at one download per track for an answer that
   cannot legitimately differ. The first answer is kept and reused. A track
   that finds nothing stores nothing, so the next one still tries and one miss
   does not cost the folder its art.

   Takes the fetch rather than doing it, so the sharing is testable without a
   network: the test counts how many times the closure runs. */
pub fn sleeve(cover: &mut CoverState, fetch: impl FnOnce() -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    if let Some(held) = &cover.shared {
        return Some(held.clone());
    }
    let found = fetch();
    if cover.one_sleeve {
        // A miss stores nothing, so the next track still tries.
        cover.shared.clone_from(&found);
    }
    found
}

/// The album name for one track, under the same rule as the sleeve.
/* The two are read together, so sharing one and not the other is what the
   sharing was meant to stop: track five matching a compilation would wear
   that compilation's name beside the record's sleeve. Kept apart from
   `sleeve` because this answer costs no request, so it is first-found rather
   than first-fetched and a track whose art failed still settles the name. */
pub fn album_name(cover: &mut CoverState, found: Option<String>) -> Option<String> {
    if let Some(held) = &cover.album {
        return Some(held.clone());
    }
    if cover.one_sleeve {
        cover.album.clone_from(&found);
    }
    found
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

    let jpeg = sleeve(cover, || {
        cfg.cover
            .then(|| lookup::cover_bytes(m, cfg.apple))
            .flatten()
    });
    let album = album_name(cover, m.album.clone());
    with_tag(path, |tag| {
        tag.set_title(m.title.clone());
        tag.set_artist(artist.clone());
        if let Some(album) = &album {
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

    /* An album's tracks share a sleeve, so one lookup is both the right
       answer and twelve fewer downloads. A playlist's do not: its tracks come
       from a dozen records, and reusing the first one's art is the bug the
       `c` menu's restore row exists to undo. The closure counts the requests,
       which is the half a "the files all match" assertion cannot see. */
    #[test]
    fn an_album_resolves_one_sleeve_and_a_playlist_resolves_each_track() {
        let art = |n: u8| Some(vec![n; 4]);

        let mut album = CoverState {
            one_sleeve: true,
            ..CoverState::default()
        };
        let mut calls = 0;
        let got: Vec<Option<Vec<u8>>> = (1..=3)
            .map(|n| {
                sleeve(&mut album, || {
                    calls += 1;
                    art(n)
                })
            })
            .collect();
        assert_eq!(calls, 1, "an album looked its sleeve up once per track");
        assert_eq!(
            got,
            vec![art(1), art(1), art(1)],
            "the tracks of one record were given different covers"
        );

        let mut playlist = CoverState::default();
        let mut calls = 0;
        let got: Vec<Option<Vec<u8>>> = (1..=3)
            .map(|n| {
                sleeve(&mut playlist, || {
                    calls += 1;
                    art(n)
                })
            })
            .collect();
        assert_eq!(calls, 3, "a playlist reused one track's art for the rest");
        assert_eq!(got, vec![art(1), art(2), art(3)]);
    }

    /* The album tag and the sleeve are read together, so sharing one without
       the other is the bug wearing different clothes: a track that matched a
       compilation would wear that compilation's name beside the record's
       own sleeve, which is worse than the twelve-covers case it replaced
       because the file now contradicts itself. */
    #[test]
    fn an_album_settles_on_one_name_as_well_as_one_sleeve() {
        let mut album = CoverState {
            one_sleeve: true,
            ..CoverState::default()
        };
        let got: Vec<Option<String>> = ["Autobahn", "Now 47", "Autobahn"]
            .into_iter()
            .map(|name| album_name(&mut album, Some(name.into())))
            .collect();
        assert_eq!(
            got.iter().filter(|n| n.as_deref() == Some("Autobahn")).count(),
            3,
            "one record's tracks were given different album names: {got:?}"
        );

        /* First found, not first fetched: this answer costs no request, so a
           track whose art failed still settles the name for the folder. */
        let mut late = CoverState {
            one_sleeve: true,
            ..CoverState::default()
        };
        assert_eq!(album_name(&mut late, None), None);
        assert_eq!(album_name(&mut late, Some("Autobahn".into())).as_deref(), Some("Autobahn"));
        assert_eq!(album_name(&mut late, Some("Now 47".into())).as_deref(), Some("Autobahn"));

        // A playlist's tracks are from different records and keep their own.
        let mut playlist = CoverState::default();
        let got: Vec<Option<String>> = ["Autobahn", "Now 47"]
            .into_iter()
            .map(|name| album_name(&mut playlist, Some(name.into())))
            .collect();
        assert_eq!(got, vec![Some("Autobahn".into()), Some("Now 47".into())]);
    }

    /* One track finding nothing must not cost the folder its art: the next
       one still tries. Otherwise an album whose first track is untagged or
       obscure ends up with no cover at all. */
    #[test]
    fn an_album_keeps_looking_until_a_track_finds_a_sleeve() {
        let mut cover = CoverState {
            one_sleeve: true,
            ..CoverState::default()
        };
        let mut calls = 0;
        let mut ask = |found: Option<Vec<u8>>| {
            sleeve(&mut cover, || {
                calls += 1;
                found
            })
        };
        assert_eq!(ask(None), None);
        assert_eq!(ask(None), None);
        assert_eq!(ask(Some(vec![7; 4])), Some(vec![7; 4]));
        assert_eq!(ask(None), Some(vec![7; 4]), "the sleeve was not kept");
        assert_eq!(calls, 3, "it asked again after finding one");
    }

    use crate::lookup;
    use std::path::{Path, PathBuf};

    #[test]
    fn extra_tags_round_trip_in_every_container_earworm_downloads() {
        let dir = std::env::temp_dir().join(format!("earworm-extras-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lyrics = "Fahren fahren fahren\nauf der Autobahn\n\n\u{1f697} caf\u{e9}";
        let mut ran = 0;
        for (format, ext, codecs) in crate::config::FORMATS {
            let file = dir.join(format!("{format}.{ext}"));
            if bare(&file, codecs).is_none() {
                eprintln!("skipped {ext}: ffmpeg could not write it");
                continue;
            }
            ran += 1;
            let wrote = [
                (Extra::TrackGain, "-6.20 dB"),
                (Extra::TrackPeak, "0.977"),
                (Extra::AlbumGain, "-5.10 dB"),
                (Extra::AlbumPeak, "0.991"),
                (Extra::Lyrics, lyrics),
            ];
            for (which, value) in wrote {
                set_extra(&file, which, value).unwrap_or_else(|e| panic!("{ext} {which:?}: {e}"));
            }
            for (which, value) in wrote {
                assert_eq!(extra(&file, which).unwrap().as_deref(), Some(value), "{ext} {which:?}");
            }
            // Writing one must not have disturbed the identity fields.
            set_fields(&file, &Fields { artist: "A".into(), title: "T".into(), ..Fields::default() }).unwrap();
            assert_eq!(extra(&file, Extra::Lyrics).unwrap().as_deref(), Some(lyrics), "{ext} lost lyrics to set_fields");

            set_extra(&file, Extra::Lyrics, "").unwrap();
            assert_eq!(extra(&file, Extra::Lyrics).unwrap(), None, "{ext} kept removed lyrics");
            assert_eq!(extra(&file, Extra::TrackGain).unwrap().as_deref(), Some("-6.20 dB"), "{ext}");

            if ext == "opus" {
                set_extra(&file, Extra::R128TrackGain, "-1536").unwrap();
                assert_eq!(extra(&file, Extra::R128TrackGain).unwrap().as_deref(), Some("-1536"));
            }
        }
        assert!(ran > 0, "no encoder was available for any format");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Break it by cutting flac with a stream copy: that writes the whole file and exits 0.
    #[test]
    fn a_chapter_cut_is_the_right_length_in_every_container_earworm_downloads() {
        let dir = std::env::temp_dir().join(format!("earworm-cut-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut ran = 0;
        for (format, ext, codecs) in crate::config::FORMATS {
            let whole = dir.join(format!("whole.{ext}"));
            if !tone(&whole, codecs) {
                eprintln!("skipped {ext}: ffmpeg could not write it");
                continue;
            }
            ran += 1;
            let part = dir.join(format!("part.{ext}"));
            cut(&whole, &part, 1.0, 2.5, format).unwrap_or_else(|e| panic!("{ext}: {e}"));
            let got = read(&part).unwrap().duration;
            // `tone` is three seconds, so the whole file reads 3 and the cut reads 1 or 2.
            assert!((1..=2).contains(&got), "{ext}: the cut is {got}s");
            // Tags are lofty's to write, not the video's to carry over.
            assert_eq!(read(&part).unwrap().title, "", "{ext} carried the video's title");
        }
        assert!(ran > 0, "no encoder was available for any format");
        // A span past the end is an error and leaves no file, not a short one.
        let whole = dir.join("whole.opus");
        if whole.is_file() {
            let past = dir.join("past.opus");
            assert!(cut(&whole, &past, 100.0, 104.0, "opus").is_err());
            assert!(!past.exists());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // Break it by taking `-map_chapters -1` out: the cut then carries the video's chapter list.
    #[test]
    fn a_cut_carries_none_of_the_videos_chapters() {
        let dir = std::env::temp_dir().join(format!("earworm-nochap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("plain.opus");
        if !tone(&plain, &["libopus"]) {
            eprintln!("skipped: ffmpeg could not write opus");
            return;
        }
        let meta = dir.join("meta.txt");
        std::fs::write(
            &meta,
            ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=1500\ntitle=One\n\
             [CHAPTER]\nTIMEBASE=1/1000\nSTART=1500\nEND=3000\ntitle=Two\n",
        )
        .unwrap();
        let whole = dir.join("whole.opus");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(&plain)
            .arg("-i")
            .arg(&meta)
            .args(["-map", "0", "-map_chapters", "1", "-c", "copy"])
            .arg(&whole)
            .status()
            .is_ok_and(|s| s.success());
        let chapters = |file: &Path| {
            let out = std::process::Command::new("ffprobe")
                .args(["-v", "error", "-show_chapters", "-of", "csv"])
                .arg(file)
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).lines().count()
        };
        if !made || chapters(&whole) != 2 {
            eprintln!("skipped: could not build a file with chapters");
            return;
        }
        let part = dir.join("part.opus");
        cut(&whole, &part, 0.0, 1.5, "opus").unwrap();
        assert_eq!(chapters(&part), 0, "the cut kept the video's chapters");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn loudness_is_read_from_the_summary_and_not_the_running_lines() {
        let log = "[Parsed_ebur128_0 @ 0x1] t: 2.9  I: -99.0 LUFS  TPK: -1.0 dBFS\n\
                   [Parsed_ebur128_0 @ 0x1] Summary:\n\n  Integrated loudness:\n    I:         -21.8 LUFS\n\
                   \n  True peak:\n    Peak:      -17.9 dBFS\n";
        assert_eq!(parse_loudness(log), Some(Loudness { lufs: -21.8, peak_db: -17.9 }));
        // Silence prints -inf, and a file shorter than a block prints no summary at all.
        let silent = "Summary:\n    I:         -inf LUFS\n    Peak:      -inf dBFS\n";
        assert_eq!(parse_loudness(silent), None);
        assert_eq!(parse_loudness("no summary here"), None);
    }

    // Opus is against -23 LUFS in Q7.8 and the rest against -18 in dB. Swapping them is the likely mistake.
    #[test]
    fn the_two_reference_levels_are_not_swapped() {
        let at = |lufs| Loudness { lufs, peak_db: -6.0206 };
        assert_eq!(gain_tags(at(-23.0), true), vec![(Extra::R128TrackGain, "0".into())]);
        assert_eq!(gain_tags(at(-29.0), true), vec![(Extra::R128TrackGain, "1536".into())]);
        assert_eq!(gain_tags(at(-18.0), true), vec![(Extra::R128TrackGain, "-1280".into())]);
        let plain = gain_tags(at(-18.0), false);
        assert_eq!(plain[0], (Extra::TrackGain, "0.00 dB".into()));
        assert_eq!(plain[1], (Extra::TrackPeak, "0.500000".into()));
        assert_eq!(gain_tags(at(-23.0), false)[0].1, "5.00 dB");
        assert_eq!(gain_tags(at(-14.5), false)[0].1, "-3.50 dB");
    }

    // A tone, so there is something to measure; the silent fixtures read -inf.
    fn tone(at: &Path, codecs: &[&str]) -> bool {
        quiet_tone(at, codecs, 0)
    }

    fn quiet_tone(at: &Path, codecs: &[&str], db: i32) -> bool {
        codecs.iter().any(|codec| {
            std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-f", "lavfi", "-i"])
                .arg(format!("sine=f=440:d=3,volume={db}dB"))
                .args(["-ac", "2", "-map_metadata", "-1", "-c:a", codec, "-strict", "-2", "-y"])
                .arg(at)
                .status()
                .is_ok_and(|s| s.success())
        })
    }

    #[test]
    fn loudness_is_written_once_in_every_container_earworm_downloads() {
        let dir = std::env::temp_dir().join(format!("earworm-loud-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut ran = 0;
        for (format, ext, codecs) in crate::config::FORMATS {
            let file = dir.join(format!("{format}.{ext}"));
            if !tone(&file, codecs) {
                eprintln!("skipped {ext}: ffmpeg could not write it");
                continue;
            }
            ran += 1;
            let measured = measure(&file).unwrap_or_else(|e| panic!("{ext}: {e}"));
            // A 440 Hz sine at ffmpeg's default amplitude, give or take the codec.
            assert!((measured.lufs + 21.8).abs() < 1.0, "{ext}: {measured:?}");
            assert!(write_loudness(&file).unwrap(), "{ext}: nothing written");
            let (marker, want) = if format == "opus" {
                (Extra::R128TrackGain, ((R128_TARGET - measured.lufs) * 256.0).round())
            } else {
                (Extra::TrackGain, REPLAYGAIN_TARGET - measured.lufs)
            };
            let got: f64 = extra(&file, marker).unwrap().unwrap_or_else(|| panic!("{ext}: no tag"))
                .trim_end_matches(" dB").parse().unwrap();
            let tolerance = if format == "opus" { 26.0 } else { 0.1 };
            assert!((got - want).abs() <= tolerance, "{ext}: wrote {got}, measured {want}");
            assert!(!write_loudness(&file).unwrap(), "{ext}: measured twice");
        }
        assert!(ran > 0, "no encoder was available for any format");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_album_is_the_energy_mean_weighted_by_length() {
        assert_eq!(album_lufs(&[]), None);
        let same = album_lufs(&[(-20.0, 200.0), (-20.0, 100.0)]).unwrap();
        assert!((same + 20.0).abs() < 1e-9, "{same}");
        // The louder track dominates an energy mean: -22.6, not the -25 an arithmetic mean gives.
        let mixed = album_lufs(&[(-20.0, 100.0), (-30.0, 100.0)]).unwrap();
        assert!((mixed + 22.596).abs() < 0.001, "{mixed}");
        // Length counts: nine quiet minutes against one loud one.
        let long = album_lufs(&[(-20.0, 60.0), (-30.0, 540.0)]).unwrap();
        assert!(long < mixed - 3.0, "{long}");
    }

    #[test]
    fn a_metadata_line_saying_summary_does_not_hide_the_real_one() {
        let log = "    comment         : Summary: a note\n\
                   [Parsed_ebur128_0 @ 0x1] Summary:\n    I:         -21.8 LUFS\n    Peak:      -17.9 dBFS\n";
        assert_eq!(parse_loudness(log), Some(Loudness { lufs: -21.8, peak_db: -17.9 }));
    }

    #[test]
    fn one_unreadable_track_does_not_cost_the_album_its_gain() {
        let dir = std::env::temp_dir().join(format!("earworm-albumskip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.flac"), dir.join("b.flac"));
        let codecs = crate::config::encoders("flac");
        if !tone(&a, codecs) || !tone(&b, codecs) {
            eprintln!("skipped: ffmpeg could not write flac");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        write_loudness(&a).unwrap();
        // What another tool might leave behind: a gain no parser reads.
        set_extra(&b, Extra::TrackGain, "-3,2 dB").unwrap();
        assert_eq!(write_album_loudness(&[a.as_path(), b.as_path()]), (1, 1));
        assert!(extra(&a, Extra::AlbumGain).unwrap().is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_album_gain_is_shared_by_every_track_and_written_once() {
        let dir = std::env::temp_dir().join(format!("earworm-album-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut ran = 0;
        for (format, ext, codecs) in crate::config::FORMATS {
            let (loud, quiet) = (dir.join(format!("loud.{ext}")), dir.join(format!("quiet.{ext}")));
            if !quiet_tone(&loud, codecs, 0) || !quiet_tone(&quiet, codecs, -12) {
                eprintln!("skipped {ext}: ffmpeg could not write it");
                continue;
            }
            ran += 1;
            let (a, b) = (loud.as_path(), quiet.as_path());
            write_loudness(a).unwrap();
            write_loudness(b).unwrap();
            assert_eq!(write_album_loudness(&[a, b]), (2, 0), "{ext}");
            let key = if format == "opus" { Extra::R128AlbumGain } else { Extra::AlbumGain };
            let (one, two) = (extra(a, key).unwrap(), extra(b, key).unwrap());
            assert!(one.is_some() && one == two, "{ext}: {one:?} vs {two:?}");
            if format != "opus" {
                // Both tracks share one peak: the louder one's.
                assert_eq!(extra(a, Extra::AlbumPeak).unwrap(), extra(a, Extra::TrackPeak).unwrap());
                assert_eq!(extra(b, Extra::AlbumPeak).unwrap(), extra(a, Extra::TrackPeak).unwrap());
            }
            assert_eq!(write_album_loudness(&[a, b]), (0, 0), "{ext}: rewrote an unchanged album");
        }
        assert!(ran > 0, "no encoder was available for any format");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // The numbers by hand: -20 and -30 LUFS at equal length average to -22.60 in energy, not -25.
    #[test]
    fn an_opus_album_gain_is_the_energy_mean_in_q78_and_follows_a_changed_track() {
        let dir = std::env::temp_dir().join(format!("earworm-r128-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.opus"), dir.join("b.opus"));
        if !tone(&a, &["libopus"]) || !tone(&b, &["libopus"]) {
            eprintln!("skipped: ffmpeg could not write opus");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        set_extra(&a, Extra::R128TrackGain, "-768").unwrap();
        set_extra(&b, Extra::R128TrackGain, "1792").unwrap();
        assert_eq!(write_album_loudness(&[a.as_path(), b.as_path()]), (2, 0));
        for file in [&a, &b] {
            assert_eq!(extra(file, Extra::R128AlbumGain).unwrap().as_deref(), Some("-103"));
        }
        // A track that changes moves the album for every file, which is why this one is rewritten.
        set_extra(&b, Extra::R128TrackGain, "-768").unwrap();
        assert_eq!(write_album_loudness(&[a.as_path(), b.as_path()]), (2, 0));
        for file in [&a, &b] {
            assert_eq!(extra(file, Extra::R128AlbumGain).unwrap().as_deref(), Some("-768"));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

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


