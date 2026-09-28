//! Window presentation for FVid's own video reader.
//!
//! The chrome follows the minimalist player design: a near-black window, the
//! picture in a rounded dark frame, a title block top-left, one large play
//! button in the middle while paused, and a thin progress line with round
//! controls along the bottom. Controls fade out while the video plays and
//! the pointer rests; any movement brings them back.
use crate::color::{
    Chromaticity, ColourDescription, CubePlan, DisplayTarget, Grade, HdrMetadata, Interpolation,
    Log, Lut, MasteringDisplay, MatrixCoeff, Primaries, Settings, ToneMap, Transfer,
};
use crate::container::FileTags;
use crate::subtitles::{self, Cue};
use crate::{
    playback_native::{NativeReader, Planar8},
    playback_thread::{Event, Frame, Pixels, Playback},
    player_gpu::VideoCallback,
};
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use eframe::egui_wgpu;
use std::sync::Arc;
use std::{
    fs::File,
    future::Future,
    io::{BufReader, Read, Seek},
    path::{Path, PathBuf},
    pin::Pin,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

type Dialog = Pin<Box<dyn Future<Output = Option<rfd::FileHandle>>>>;

/// Open an empty player or a supported local Y4M, MP4/AVC or WebM/VP9/AV1 file.
/// Several inputs, or a playlist that names them, queue up: the player moves to
/// the next item when the picture runs out.
/// The bounds `--start-time` and `--stop-time` put on an item. They belong to
/// the first one only, so opening the next takes them out and leaves this.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct PlayBounds {
    start: Option<Duration>,
    stop: Option<Duration>,
}

/// The panel a session defaults to grading for: a desktop screen's diffuse
/// white, which is what an SDR player is asked to fill. `--display` replaces it
/// for a screen that reaches higher, or one that takes BT.2100 codes at all.
const PANEL_NITS: f32 = 100.0;

/// The destinations `--display` offers, each with the codes its panel is fed.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Panel {
    /// An SDR screen: BT.709 codes over whatever primaries the item states, so
    /// the destination curve stays the one [`Grading::grade_for`] picks.
    Sdr(f32),
    /// An HDR10 screen: PQ codes over BT.2020 at this peak.
    Pq(f32),
    /// A broadcast HDR screen: HLG codes over BT.2020 at this peak.
    Hlg(f32),
}

impl Panel {
    /// A desktop panel at the diffuse white the session assumes for it.
    const DEFAULT: Self = Self::Sdr(PANEL_NITS);

    /// The light this panel shows: reference white rides with the kind, since
    /// an HDR panel's grey is not the desktop's.
    fn target(self) -> DisplayTarget {
        match self {
            Self::Sdr(nits) => DisplayTarget::sdr(nits),
            Self::Pq(nits) | Self::Hlg(nits) => DisplayTarget::hdr(nits),
        }
    }

    /// The codes to write, when the panel's own kind decides them rather than
    /// the item.
    fn destination(self) -> Option<(Transfer, Primaries)> {
        match self {
            Self::Sdr(_) => None,
            Self::Pq(_) => Some((Transfer::Pq, Primaries::BT2020)),
            Self::Hlg(_) => Some((Transfer::Hlg, Primaries::BT2020)),
        }
    }
}

/// What the command line asked to be done to a picture's colour: read before
/// any item is opened, and applied to each one from the signal that item
/// states for itself. As with the crop and the aspect, the request holds for
/// the whole session.
#[derive(Clone, Debug, Default)]
struct Grading {
    /// Camera log curve the coded values carry.
    log: Option<Log>,
    /// Highlight compression to run in place of the one the plan picks for the
    /// item.
    tone_map: Option<ToneMap>,
    /// Grading LUT read out of the named file, applied after the conversion the
    /// other two ask for.
    lut: Option<Lut>,
    /// The working gamut the coded values are read in, named instead of taken
    /// from the file or the curve.
    gamut: Option<Primaries>,
    /// The panel the picture is graded for, named instead of assumed.
    panel: Option<Panel>,
    /// Edge length of the grid the conversion is baked on, named instead of the
    /// 33 nodes every session starts with. A modifier of how a grade is computed
    /// rather than a request to change a picture, so it is not in [`Self::silent`].
    grid: Option<usize>,
    /// How that grid and a `--lut` file are read between their nodes. Also a
    /// modifier: `nearest` on a coarse look keeps its steps instead of blending
    /// them away.
    interp: Option<Interpolation>,
}

impl Grading {
    /// Whether the command line named no colour change at all.
    fn silent(&self) -> bool {
        self.log.is_none()
            && self.gamut.is_none()
            && self.panel.is_none()
            && self.tone_map.is_none()
            && self.lut.is_none()
    }

    /// Bake what this session asks for against the signal `signal` and the
    /// light `hdr` describe, or `None` when the bytes the reader hands over are
    /// the picture to show.
    ///
    /// A log curve or a BT.2100 transfer has to be turned into something an SDR
    /// panel can show, so the destination is the video signal a screen is fed:
    /// BT.709 codes over BT.709 primaries. A request that names neither is a
    /// look over the item's own codes, so the destination becomes the statement
    /// the file itself makes and the LUT is the one thing that moves; a curve
    /// this module has no name for passes a code value straight through, which
    /// is the same picture. Naming nothing leaves an SDR item ungraded, while
    /// BT.2100 material still gets the compression [`Grade::new`] picks, since
    /// unmapped it is a flat grey one. A gamut the session names is a request on
    /// its own and outranks the primaries the file states, since naming one is
    /// how a wrong label gets corrected. A panel it names outranks the item too,
    /// because writing BT.2100 codes is the only way to hand a screen an HDR
    /// picture, and a caller who asks for one does not want the file's curve
    /// kept over the request. A grid and a reading of it change how finely the
    /// conversion is baked and how its nodes are joined, which says nothing about
    /// the picture and so never asks for one on its own.
    fn grade_for(&self, signal: ColourDescription, hdr: &HdrMetadata) -> Option<Grade> {
        if self.silent() && !signal.is_hdr() {
            return None;
        }
        let panel = self.panel.unwrap_or(Panel::DEFAULT);
        let mut settings = Settings::video(panel.target());
        settings.log = self.log;
        settings.gamut = self.gamut;
        settings.tone_map = self.tone_map;
        if let Some(size) = self.grid {
            settings.size = size;
        }
        if let Some(mode) = self.interp {
            settings.interpolation = mode;
        }
        if !signal.is_hdr() && self.log.is_none() {
            settings.to = signal.transfer_function();
            if let Some(primaries) = signal.primary_set() {
                settings.dest = primaries;
            }
        }
        if let Some((transfer, primaries)) = panel.destination() {
            settings.to = transfer;
            settings.dest = primaries;
        }
        Some(Grade::new(signal, hdr, settings, self.lut.clone()))
    }
}

/// What `fvid play` was asked for: the inputs to queue, and how the first of
/// them starts, stops and runs at.
struct PlayArgs {
    paths: Vec<PathBuf>,
    start: Option<Duration>,
    stop: Option<Duration>,
    rate_milli: u32,
    start_paused: bool,
    no_audio: bool,
    /// Which audio track of each item to hear, counted from zero.
    audio_track: Option<usize>,
    /// Which of each item's subtitle sources to read, counted from zero.
    subtitle_track: Option<usize>,
    /// Output gain in thousandths the session starts at.
    volume_milli: u32,
    muted: bool,
    /// What happens when the queue runs out, as the command line asked.
    repeat: Repeat,
    /// Whether the queue walks in a drawn order from the first frame, as
    /// `--random` asks for it.
    shuffle: bool,
    /// Signed shifts in milliseconds the clocks start with.
    audio_delay_ms: i64,
    subtitle_delay_ms: i64,
    /// A subtitle file named on the command line, in front of the one the
    /// player would guess from the video's own name.
    subtitle_file: Option<PathBuf>,
    /// Magnification in thousandths the session starts at, of the whole picture
    /// fitted into the frame.
    zoom_milli: u32,
    /// The shape the session starts cropped to, as VLC's crop option writes it.
    crop: (u32, u32),
    /// The shape the session starts drawn at, as VLC's aspect option writes it.
    aspect: Aspect,
    /// The picture settings the session starts with. As in VLC, naming one of
    /// them is asking for the filter, so any of the five switches the dialog's
    /// own Enable on; naming none leaves the pixels alone until a key says
    /// otherwise.
    adjust: Adjust,
    /// The colour the session starts graded with: the camera log to read the
    /// codes as, the highlight compression to run, and the LUT to apply after
    /// them. Naming any of the three asks for the grade; naming none leaves the
    /// picture as the reader hands it over, except on BT.2100 material, which an
    /// SDR panel cannot show unmapped.
    grading: Grading,
}

/// Read the options `fvid play` answers, in either `--flag VALUE` or
/// `--flag=VALUE` form. Everything that is not an option joins the queue.
fn parse_play_args(args: &[String]) -> crate::Result<PlayArgs> {
    let mut paths = Vec::new();
    let mut start = None;
    let mut stop = None;
    let mut rate_milli = 1_000;
    let mut start_paused = false;
    let mut no_audio = false;
    let mut audio_track = None;
    let mut subtitle_track = None;
    let mut volume_milli = 1_000;
    let mut muted = false;
    let mut repeat = Repeat::default();
    let mut shuffle = false;
    let mut audio_delay_ms = 0;
    let mut subtitle_delay_ms = 0;
    let mut subtitle_file = None;
    let mut zoom_milli = 1_000;
    let mut crop = NO_CROP;
    let mut aspect = Aspect::Source;
    let mut adjust = Adjust::default();
    let mut grading = Grading::default();
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (arg, None),
        };
        index += 1;
        match flag {
            "--start-time" => {
                let value = option_value(args, &mut index, flag, inline)?;
                start = Some(parse_clock(&value)?);
            }
            "--stop-time" => {
                let value = option_value(args, &mut index, flag, inline)?;
                stop = Some(parse_clock(&value)?);
            }
            "--rate" => {
                let value = option_value(args, &mut index, flag, inline)?;
                rate_milli = parse_rate(&value)?;
            }
            "--start-paused" => {
                no_value(inline, flag)?;
                start_paused = true;
            }
            "--no-audio" => {
                no_value(inline, flag)?;
                no_audio = true;
            }
            "--audio-track" => {
                let value = option_value(args, &mut index, flag, inline)?;
                audio_track = Some(parse_track(&value, flag)?);
            }
            "--subtitle-track" => {
                let value = option_value(args, &mut index, flag, inline)?;
                subtitle_track = Some(parse_track(&value, flag)?);
            }
            "--volume" => {
                let value = option_value(args, &mut index, flag, inline)?;
                volume_milli = parse_volume(&value)?;
            }
            "--mute" => {
                no_value(inline, flag)?;
                muted = true;
            }
            "--loop" => {
                no_value(inline, flag)?;
                repeat = Repeat::All;
            }
            "--repeat" => {
                no_value(inline, flag)?;
                repeat = Repeat::One;
            }
            "--random" => {
                no_value(inline, flag)?;
                shuffle = true;
            }
            "--audio-delay" => {
                let value = option_value(args, &mut index, flag, inline)?;
                audio_delay_ms = parse_delay(&value, flag)?;
            }
            "--subtitle-delay" => {
                let value = option_value(args, &mut index, flag, inline)?;
                subtitle_delay_ms = parse_delay(&value, flag)?;
            }
            "--sub-file" => {
                let value = option_value(args, &mut index, flag, inline)?;
                subtitle_file = Some(PathBuf::from(value));
            }
            "--zoom" => {
                let value = option_value(args, &mut index, flag, inline)?;
                zoom_milli = parse_zoom(&value)?;
            }
            "--crop" => {
                let value = option_value(args, &mut index, flag, inline)?;
                crop = parse_crop(&value)?;
            }
            "--aspect" => {
                let value = option_value(args, &mut index, flag, inline)?;
                aspect = parse_aspect(&value)?;
            }
            "--brightness" => {
                let value = option_value(args, &mut index, flag, inline)?;
                adjust.brightness = parse_setting(&value, "brightness", 0.0..=2.0)?;
                adjust.on = true;
            }
            "--gamma" => {
                let value = option_value(args, &mut index, flag, inline)?;
                adjust.gamma = parse_setting(&value, "gamma", 0.01..=10.0)?;
                adjust.on = true;
            }
            "--saturation" => {
                let value = option_value(args, &mut index, flag, inline)?;
                adjust.saturation = parse_setting(&value, "saturation", 0.0..=3.0)?;
                adjust.on = true;
            }
            "--contrast" => {
                let value = option_value(args, &mut index, flag, inline)?;
                adjust.contrast = parse_setting(&value, "contrast", 0.0..=2.0)?;
                adjust.on = true;
            }
            "--hue" => {
                let value = option_value(args, &mut index, flag, inline)?;
                adjust.hue = parse_setting(&value, "hue", -180.0..=180.0)?;
                adjust.on = true;
            }
            "--log" => {
                let value = option_value(args, &mut index, flag, inline)?;
                grading.log = Some(parse_log(&value)?);
            }
            "--gamut" => {
                let value = option_value(args, &mut index, flag, inline)?;
                grading.gamut = Some(parse_gamut(&value)?);
            }
            "--display" => {
                let value = option_value(args, &mut index, flag, inline)?;
                grading.panel = Some(parse_panel(&value)?);
            }
            "--tonemap" => {
                let value = option_value(args, &mut index, flag, inline)?;
                grading.tone_map = Some(parse_tone_map(&value)?);
            }
            "--lut" => {
                let value = option_value(args, &mut index, flag, inline)?;
                grading.lut = Some(read_lut(&value)?);
            }
            "--grid" => {
                let value = option_value(args, &mut index, flag, inline)?;
                grading.grid = Some(parse_grid(&value)?);
            }
            "--interp" => {
                let value = option_value(args, &mut index, flag, inline)?;
                grading.interp = Some(parse_interpolation(&value)?);
            }
            _ if arg.starts_with('-') => {
                return Err(crate::invalid(&format!("unknown play option {arg}")));
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if let (Some(start), Some(stop)) = (start, stop)
        && stop <= start
    {
        return Err(crate::invalid("--stop-time is before --start-time"));
    }
    Ok(PlayArgs {
        paths,
        start,
        stop,
        rate_milli,
        start_paused,
        no_audio,
        audio_track,
        subtitle_track,
        volume_milli,
        muted,
        repeat,
        shuffle,
        audio_delay_ms,
        subtitle_delay_ms,
        subtitle_file,
        zoom_milli,
        crop,
        aspect,
        adjust,
        grading,
    })
}

/// A switch that carries no value: `--start-paused=quick` is a mistake rather
/// than a switch with a word after it.
fn no_value(inline: Option<&str>, flag: &str) -> crate::Result<()> {
    if inline.is_some() {
        return Err(crate::invalid(&format!("{flag} takes no value")));
    }
    Ok(())
}

/// A track number as the player's own messages say it, from one, kept as the
/// index from zero the lists are walked with.
fn parse_track(text: &str, flag: &str) -> crate::Result<usize> {
    let invalid = || crate::invalid(&format!("invalid {flag} {text:?}"));
    let stated = text.trim().parse::<usize>().map_err(|_| invalid())?;
    let counted = stated.checked_sub(1).ok_or_else(invalid)?;
    Ok(counted)
}

/// The value of an option: glued to its flag with `=`, or standing after it.
fn option_value(
    args: &[String],
    index: &mut usize,
    flag: &str,
    inline: Option<&str>,
) -> crate::Result<String> {
    if let Some(value) = inline {
        return Ok(value.to_owned());
    }
    let value = args
        .get(*index)
        .ok_or_else(|| crate::invalid(&format!("{flag} needs a value")))?;
    *index += 1;
    Ok(value.clone())
}

/// A media time: seconds on their own, or hours, minutes and seconds split by
/// `:`. Only the last part carries the fraction.
fn parse_clock(text: &str) -> crate::Result<Duration> {
    let invalid = || crate::invalid(&format!("invalid time {text:?}"));
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() > 3 {
        return Err(invalid());
    }
    let mut total = 0.0;
    for part in parts {
        let value = part.trim().parse::<f64>().map_err(|_| invalid())?;
        if !value.is_finite() || value < 0.0 {
            return Err(invalid());
        }
        total = total * 60.0 + value;
    }
    Duration::try_from_secs_f64(total).map_err(|_| invalid())
}

/// `--rate` in thousandths, within the range VLC offers.
fn parse_rate(text: &str) -> crate::Result<u32> {
    let invalid = || crate::invalid(&format!("invalid rate {text:?}"));
    let rate = text.parse::<f64>().map_err(|_| invalid())?;
    if !(0.25..=4.0).contains(&rate) {
        return Err(invalid());
    }
    Ok((rate * 1_000.0).round() as u32)
}

/// `--volume` as VLC states it: a whole percent of the recorded level, up to
/// the 200% its slider reaches, kept as the thousandths the player holds.
fn parse_volume(text: &str) -> crate::Result<u32> {
    let invalid = || crate::invalid(&format!("invalid volume {text:?}"));
    let percent = text.trim().parse::<u32>().map_err(|_| invalid())?;
    if percent > VOLUME_MAX / 10 {
        return Err(invalid());
    }
    Ok(percent * 10)
}

/// A clock shift in milliseconds, either way round, held to the ±10 s the
/// delay keys stop at: a start value asks for the same shift a key would leave.
fn parse_delay(text: &str, flag: &str) -> crate::Result<i64> {
    let invalid = || crate::invalid(&format!("invalid {flag} {text:?}"));
    let millis = text.trim().parse::<i64>().map_err(|_| invalid())?;
    Ok(millis.clamp(-10_000, 10_000))
}

/// `--zoom` as a multiple of the fitted picture, within the span VLC's own
/// zoom controls cover: its lowest menu rung to the ceiling its scale keys
/// stop at. Kept as thousandths, which is also how the keys hold it.
fn parse_zoom(text: &str) -> crate::Result<u32> {
    let invalid = || crate::invalid(&format!("invalid zoom {text:?}"));
    let factor = text.parse::<f64>().map_err(|_| invalid())?;
    if !(0.25..=10.0).contains(&factor) {
        return Err(invalid());
    }
    Ok((factor * 1_000.0).round() as u32)
}

/// One of the picture settings, held to the span its slider covers: a value
/// outside it is a mistake rather than a clamp request, the way `--zoom` reads
/// its own bounds.
fn parse_setting(
    text: &str,
    flag: &str,
    span: std::ops::RangeInclusive<f32>,
) -> crate::Result<f32> {
    let invalid = || crate::invalid(&format!("invalid {flag} {text:?}"));
    let value = text.trim().parse::<f32>().map_err(|_| invalid())?;
    if !span.contains(&value) {
        return Err(invalid());
    }
    Ok(value)
}

/// `--crop` as a shape: two whole numbers split by `:`, which is how VLC's
/// crop option reads. `none` stands for the menu's Default, which crops
/// nothing.
fn parse_crop(text: &str) -> crate::Result<(u32, u32)> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("none") {
        return Ok(NO_CROP);
    }
    parse_ratio(text, "crop")
}

/// `--aspect` as one of VLC's aspect-ratio choices: `default` for the shape the
/// file declares, `fill` for the window's own shape (its `Fill Window`), and a
/// shape split by `:` for everything else.
fn parse_aspect(text: &str) -> crate::Result<Aspect> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("default") {
        return Ok(Aspect::Source);
    }
    if text.eq_ignore_ascii_case("fill") {
        return Ok(Aspect::Fill);
    }
    let (x, y) = parse_ratio(text, "aspect")?;
    Ok(Aspect::Shape(x, y))
}

/// A shape as VLC writes it on the command line: `4:3`, or `185:100` for the
/// cinema ratios its menus spell `1.85:1`. The bounds keep a mistyped ratio
/// from asking for a single pixel of picture.
fn parse_ratio(text: &str, flag: &str) -> crate::Result<(u32, u32)> {
    let invalid = || crate::invalid(&format!("invalid {flag} {text:?}"));
    let (x, y) = text.split_once(':').ok_or_else(invalid)?;
    let shape = (
        x.trim().parse::<u32>().map_err(|_| invalid())?,
        y.trim().parse::<u32>().map_err(|_| invalid())?,
    );
    if shape.0 == 0 || shape.1 == 0 || shape.0 > 1_000 || shape.1 > 1_000 {
        return Err(invalid());
    }
    Ok(shape)
}

/// `--log` as one of the camera curves fvid knows how to un-wrap. The names are
/// the vendors' own spellings, lowercased, so what a manual calls the profile is
/// what the option takes. A word that names none is a mistake at the door, and
/// the message says which ones do.
fn parse_log(text: &str) -> crate::Result<Log> {
    let label = text.trim().to_ascii_lowercase();
    let known = || {
        let names = Log::ALL
            .iter()
            .map(|log| log.label())
            .collect::<Vec<_>>()
            .join(", ");
        crate::invalid(&format!("unknown --log {text:?}; fvid unwraps {names}"))
    };
    Log::from_label(&label).ok_or_else(known)
}

/// `--gamut` as one of the working gamuts fvid converts a picture from. A
/// caller names one to correct a container that labels its bytes wrongly or
/// leaves them unlabelled, so the triangle it picks outranks both the primaries
/// the file states and the one a `--log` curve lends. Case and the separators a
/// name is written with make no difference, and the vendor's own shorthand for
/// the same triangle (`awg4`, `rec709`) reaches it too. A word that names none
/// is a mistake at the door, and the message lists the names on offer.
fn parse_gamut(text: &str) -> crate::Result<Primaries> {
    let known = || {
        let names = Primaries::NAMED
            .iter()
            .map(|p| p.label())
            .collect::<Vec<_>>()
            .join(", ");
        crate::invalid(&format!(
            "unknown --gamut {text:?}; fvid converts from {names}"
        ))
    };
    Primaries::from_label(text.trim()).ok_or_else(known)
}

/// `--display` as the panel the session grades every picture for: a kind and
/// optionally the peak luminance it reaches. The kinds are the destinations
/// `fvid` writes codes for — `sdr` for the BT.709 signal a desktop screen is
/// fed, `pq` for HDR10's ST 2084, `hlg` for broadcast's ARIB curve — and both
/// BT.2100 kinds land on BT.2020 primaries; `hdr` is taken as another spelling
/// of `pq`, since that is the format the word usually names. Omitting the
/// number takes what the kind is authored against: 100 cd/m² of diffuse white
/// for SDR, 1 000 for an HDR panel. A screen of another peak is the caller's to
/// say, since where its white sits is what decides how hard the highlights get
/// compressed.
fn parse_panel(text: &str) -> crate::Result<Panel> {
    let unknown = || {
        crate::invalid(&format!(
            "unknown --display {text:?}; fvid grades for sdr[:nits], pq[:nits] or hlg[:nits]"
        ))
    };
    let (kind, nits) = match text.split_once(':') {
        Some((kind, nits)) => (kind, Some(nits)),
        None => (text, None),
    };
    let kind = kind.trim().to_ascii_lowercase();
    let panel = match (kind.as_str(), nits) {
        ("sdr", None) => Panel::Sdr(PANEL_NITS),
        ("pq" | "hdr", None) => Panel::Pq(1_000.0),
        ("hlg", None) => Panel::Hlg(1_000.0),
        ("sdr", Some(nits)) => Panel::Sdr(parse_nits(nits)?),
        ("pq" | "hdr", Some(nits)) => Panel::Pq(parse_nits(nits)?),
        ("hlg", Some(nits)) => Panel::Hlg(parse_nits(nits)?),
        _ => return Err(unknown()),
    };
    Ok(panel)
}

/// The peak a `--display` names, in cd/m². The range covers a laptop lid and
/// the brightest reference monitor a grade is checked on; 10 000 is where PQ's
/// own scale ends, so nothing above it can be written as a code.
fn parse_nits(text: &str) -> crate::Result<f32> {
    let nits = text.trim().parse::<f32>().map_err(|_| {
        crate::invalid(&format!(
            "--display needs a peak in cd/m², {text:?} is none"
        ))
    })?;
    if !(1.0..=10_000.0).contains(&nits) {
        return Err(crate::invalid(&format!(
            "--display peak {text:?} is outside 1..=10000 cd/m²"
        )));
    }
    Ok(nits)
}

/// `--grid` as the edge length the conversion is baked on. A cube is the whole
/// colour decision of a frame, so its edge trades accuracy for build time and
/// memory: 33 nodes are what a session uses by default, 128 is the largest grid
/// `fvid` reads out of a file, and below two nodes there is nothing to join.
fn parse_grid(text: &str) -> crate::Result<usize> {
    let size = text
        .trim()
        .parse::<usize>()
        .map_err(|_| crate::invalid(&format!("--grid needs a node count, {text:?} is none")))?;
    if !(2..=128).contains(&size) {
        return Err(crate::invalid(&format!(
            "--grid {size} is outside 2..=128 nodes"
        )));
    }
    Ok(size)
}

/// `--interp` as the way a grid's nodes are joined: `nearest` for the step a
/// look was authored with, `trilinear` for the six-sided box around a point,
/// `tetrahedral` for the split that keeps an edge sharp. It reads both the baked
/// conversion and a `--lut` file, so the two are never asked for separately.
fn parse_interpolation(text: &str) -> crate::Result<Interpolation> {
    let label = text.trim().to_ascii_lowercase();
    Interpolation::from_label(&label).ok_or_else(|| {
        crate::invalid(&format!(
            "unknown --interp {text:?}; fvid reads a grid by {}",
            Interpolation::ALL
                .iter()
                .map(|mode| mode.label())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })
}

/// What the item on screen states about its colour, said only when it is worth
/// saying. BT.709 codes over BT.709 primaries with the BT.709 luma and the video
/// range are what every screen expects, so repeating them for every file would
/// tell a viewer nothing; a part the file leaves unstated is a statement of its
/// own, since it is the part the grade has to answer for.
fn colour_line(signal: ColourDescription) -> Option<String> {
    // H.273 counts 0 as unspecified, and a container that wrote no colour at all
    // leaves exactly that in all three fields. Naming a shape for the matrix code
    // of a file that claimed none would tell a viewer something the file never
    // said, so an all-zero triple with the studio range gets no line — the same
    // reading `ColourDescription::filled_with` gives it upstream.
    let stated = (signal.primaries | signal.transfer | signal.matrix) != 0;
    if !stated && !signal.full_range {
        return None;
    }
    let primaries = signal.primary_set();
    let transfer = signal.transfer_function();
    let matrix = signal.matrix_coefficients();
    let expected = primaries == Some(Primaries::BT709)
        && transfer == Transfer::Bt709
        && matrix == Some(MatrixCoeff::Bt709)
        && !signal.full_range;
    if expected {
        return None;
    }
    let named = |code: u8, label: Option<&'static str>| match label {
        Some(word) => word.to_owned(),
        // H.273 spells unspecified as 0 or 2, and a container that wrote no
        // colour at all leaves exactly those. Any other number is a part the
        // file did state and fvid cannot resolve, so the line gives the number
        // rather than the same words the silent file got.
        None if code == 0 || code == 2 => "not stated".to_owned(),
        None => format!("code {code}"),
    };
    let curve = match transfer {
        Transfer::Unknown if signal.transfer != 0 && signal.transfer != 2 => {
            format!("code {}", signal.transfer)
        }
        _ => transfer.label().to_owned(),
    };
    Some(format!(
        "Colour: {} · {curve} · {} · {}",
        named(signal.primaries, primaries.map(|set| set.label())),
        named(signal.matrix, matrix.map(|coeff| coeff.label())),
        if signal.full_range { "full" } else { "limited" }
    ))
}

/// The panel a master was graded for, from the corners the file itself states.
/// They are carried in multiples of 0.00002, so an exact compare answers
/// "custom" to a BT.2020 HDR10 master — the same rounding `MasteringDisplay::is_hdr10`
/// exists to cover. A volume no table entry rounds onto is genuinely another
/// panel, and the line says so.
fn mastered_on(display: &MasteringDisplay) -> &'static str {
    let corners = display.primaries();
    let close = |x: f64, y: f64| (x - y).abs() <= 0.002;
    let near = |a: Chromaticity, b: Chromaticity| close(a.x, b.x) && close(a.y, b.y);
    Primaries::NAMED
        .iter()
        .copied()
        .find(|panel| {
            near(panel.r, corners.r)
                && near(panel.g, corners.g)
                && near(panel.b, corners.b)
                && near(panel.white, corners.white)
        })
        .map_or("custom", |panel| panel.label())
}

/// The light the item carries: the display it was mastered for and the two
/// content limits, with the parts a file leaves unstated left out rather than
/// guessed at. A file that states neither has no line here, since the panel's
/// job is to say what the picture is made of.
fn light_line(hdr: &HdrMetadata) -> Option<String> {
    if hdr.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(display) = hdr.mastering {
        parts.push(format!(
            "{} · {}–{} cd/m²",
            mastered_on(&display),
            display.min_luminance,
            display.max_luminance
        ));
    }
    if hdr.light.max_cll > 0.0 {
        parts.push(format!("MaxCLL {} cd/m²", hdr.light.max_cll));
    }
    if hdr.light.max_fall > 0.0 {
        parts.push(format!("MaxFALL {} cd/m²", hdr.light.max_fall));
    }
    Some(format!("HDR: {}", parts.join(" · ")))
}

/// What the grade does to the picture: the curve its bytes are read as and the
/// curve they are written as, the triangle they change when the two differ, the
/// shoulder the highlights go through, the panel all of it is fitted to, the
/// look applied after it and the grid all of that is baked on with its reading.
/// Taken from the plan the grade baked, so the line says what is being done
/// rather than what was asked — the two part whenever fvid picks a shoulder for
/// material that asked for none.
fn grade_text(plan: &CubePlan, interpolation: Interpolation, lut: Option<&Lut>) -> String {
    let mut parts = vec![format!(
        "{} → {}",
        match plan.log {
            Some(profile) => profile.label(),
            None => plan.from.label(),
        },
        plan.to.label()
    )];
    if plan.source != plan.dest {
        parts.push(format!("{} → {}", plan.source.label(), plan.dest.label()));
    }
    if let Some(mode) = plan.tone_map {
        parts.push(mode.label().to_owned());
    }
    if let Some(target) = plan.target {
        parts.push(format!("{} cd/m²", target.peak_nits));
    }
    if let Some(lut) = lut {
        parts.push(match lut {
            // A look's own grid is the only thing about it a viewer can act on:
            // the file's title is not carried through the parse, and the shape
            // and node count say what was applied and how coarse it is.
            Lut::One(table) => format!("look 1D {}", table.len()),
            Lut::Three(table) => format!("look {}³", table.size),
        });
    }
    parts.push(format!("{}³ {}", plan.size, interpolation.label()));
    parts.join(" · ")
}

/// `--tonemap` as one of the highlight curves fvid can run, spelled the way
/// FFmpeg's `tonemap` filter spells them. Naming none of them is a mistake at
/// the door, and the message says which are on offer.
fn parse_tone_map(text: &str) -> crate::Result<ToneMap> {
    let label = text.trim().to_ascii_lowercase();
    let known = || {
        let names = ToneMap::ALL
            .iter()
            .map(|mode| mode.label())
            .collect::<Vec<_>>()
            .join(", ");
        crate::invalid(&format!(
            "unknown --tonemap {text:?}; fvid compresses with {names}"
        ))
    };
    ToneMap::from_label(&label).ok_or_else(known)
}

/// `--lut`: the grading look the session starts with, read out of the named file
/// before anything is opened. A look the player cannot parse stops the command
/// at startup rather than showing an ungraded picture, so the file's own name is
/// in every message about it. `.cube` and `.3dl` are told apart by their content,
/// not their suffix.
fn read_lut(path: &str) -> crate::Result<Lut> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| crate::invalid(&format!("cannot read --lut {path:?}: {error}")))?;
    Lut::from_text(&text).map_err(|error| crate::invalid(&format!("bad --lut {path:?}: {error}")))
}

pub fn run(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let parsed = parse_play_args(&args)?;
    let mut app = Player {
        queue: expand_inputs(&parsed.paths),
        bounds: PlayBounds {
            start: parsed.start,
            stop: parsed.stop,
        },
        rate_milli: parsed.rate_milli,
        start_paused: parsed.start_paused,
        no_audio: parsed.no_audio,
        preferred_audio: parsed.audio_track,
        preferred_subtitle: parsed.subtitle_track,
        volume_milli: parsed.volume_milli,
        muted: parsed.muted,
        repeat: parsed.repeat,
        shuffle: parsed.shuffle,
        audio_delay_ms: parsed.audio_delay_ms,
        subtitle_delay_ms: parsed.subtitle_delay_ms,
        subtitle_file: parsed.subtitle_file,
        zoom_milli: parsed.zoom_milli,
        crop: parsed.crop,
        aspect: parsed.aspect,
        adjust: parsed.adjust,
        grading: parsed.grading,
        ..Default::default()
    };
    if app.shuffle {
        // VLC starts shuffling with the queue already read in: the cycle is
        // dealt to the list the command line named, so the first step is a
        // draw rather than the second item.
        app.start_shuffle();
    }
    if let Some(path) = app.queue.first().cloned() {
        app.open(path)?;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([480.0, 320.0])
            .with_title("FVid")
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_fullsize_content_view(true)
            .with_drag_and_drop(true),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "FVid",
        options,
        Box::new(move |cc| {
            if let Some(state) = cc.wgpu_render_state.as_ref() {
                crate::player_gpu::install(state);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

// Palette from the design canvas. Every overlay element sits at 50% opacity
// over the picture; only the window and frame backgrounds are opaque.
const WINDOW: Color32 = Color32::from_rgb(0x0e, 0x0e, 0x10);
const FRAME: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1d);
const TEXT: Color32 = Color32::from_rgba_premultiplied(0x7a, 0x7a, 0x79, 128);
const MUTED: Color32 = Color32::from_rgba_premultiplied(0x51, 0x51, 0x53, 128);
const DIM: Color32 = Color32::from_rgba_premultiplied(0x3e, 0x3e, 0x41, 128);
const ACCENT: Color32 = Color32::from_rgba_premultiplied(0x74, 0x71, 0x6b, 128);
const ERROR: Color32 = Color32::from_rgba_premultiplied(0x70, 0x45, 0x3d, 128);
const TRACK: Color32 = Color32::from_rgba_premultiplied(23, 23, 23, 23);
const CHIP: Color32 = Color32::from_rgba_premultiplied(8, 8, 8, 8);
const CHIP_STRONG: Color32 = Color32::from_rgba_premultiplied(13, 13, 13, 13);

const BUTTON: f32 = 44.0;
const HIDE_AFTER: Duration = Duration::from_millis(2500);
/// How long a transient control message stays on screen.
const OSD_AFTER: Duration = Duration::from_millis(1500);

/// Volume ceiling, matching VLC's slider at 200% of the recorded level.
const VOLUME_MAX: u32 = 2_000;
/// One VLC volume tick, 5%.
const VOLUME_STEP: u32 = 50;
/// Rates VLC exposes, in thousandths, from 0.25x to 4x.
const RATES: [u32; 7] = [250, 500, 1_000, 1_500, 2_000, 3_000, 4_000];
/// Magnifications VLC's Zoom menu lists, in thousandths of the fitted size:
/// Quarter, Half, Original and Double.
const ZOOMS: [u32; 4] = [250, 500, 1_000, 2_000];
/// The shapes VLC's Crop menu cycles, as width to height. Its first choice is
/// the one that crops nothing, so walking the menu returns to the whole
/// picture; the cinema ratios are kept as their stored numbers (`185:100`),
/// which is what the menu labels `1.85:1`.
const CROPS: [(u32, u32); 11] = [
    NO_CROP,
    (16, 10),
    (16, 9),
    (4, 3),
    (185, 100),
    (221, 100),
    (235, 100),
    (239, 100),
    (5, 3),
    (5, 4),
    (1, 1),
];
/// The menu's Default: a picture shown whole.
const NO_CROP: (u32, u32) = (0, 0);

/// Which shape the picture is drawn at: the one its container declares, one
/// VLC's aspect-ratio menu forces on it, or the window's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Aspect {
    /// The shape the file's own pixels add up to.
    #[default]
    Source,
    /// A width-to-height shape forced on the picture.
    Shape(u32, u32),
    /// VLC's `Fill Window`: the picture takes the shape of the frame.
    Fill,
}

/// The shapes VLC's aspect-ratio menu cycles, in the order it lists them. Its
/// first choice forces nothing, so walking the menu comes back to the file's own
/// shape; the cinema ratios are kept as their stored numbers (`235:100`), which
/// is what the menu labels `2.35:1`.
const ASPECTS: [Aspect; 10] = [
    Aspect::Source,
    Aspect::Shape(16, 9),
    Aspect::Shape(4, 3),
    Aspect::Shape(1, 1),
    Aspect::Shape(16, 10),
    Aspect::Shape(221, 100),
    Aspect::Shape(235, 100),
    Aspect::Shape(239, 100),
    Aspect::Shape(5, 4),
    Aspect::Fill,
];

/// Move a volume level by one step, staying inside the slider range.
fn volume_step(current: u32, up: bool) -> u32 {
    if up {
        (current + VOLUME_STEP).min(VOLUME_MAX)
    } else {
        current.saturating_sub(VOLUME_STEP)
    }
}

/// Move to the neighbouring VLC rate preset, clamped at both ends. A level that
/// is not a preset (from `--rate`, say) snaps to the next one rather than
/// inventing an in-between speed.
fn rate_step(current: u32, up: bool) -> u32 {
    if up {
        RATES
            .iter()
            .copied()
            .find(|r| *r > current)
            .unwrap_or(*RATES.last().unwrap())
    } else {
        RATES
            .iter()
            .rev()
            .copied()
            .find(|r| *r < current)
            .unwrap_or(RATES[0])
    }
}

/// VLC's fine pair: the rate one tenth faster or slower than the current one.
/// The ladder's bounds hold it, so a fine step never asks the stretching clock
/// for a speed the preset keys cannot step back from.
fn rate_fine(current: u32, up: bool) -> u32 {
    (i64::from(current) + if up { 100 } else { -100 })
        .clamp(i64::from(RATES[0]), i64::from(*RATES.last().unwrap())) as u32
}

/// Move to the neighbouring VLC zoom rung, clamped at both ends. A
/// magnification that is not one of the menu's four (from `--zoom`, say) snaps
/// to the next rung rather than inventing an in-between one.
fn zoom_step(current: u32, up: bool) -> u32 {
    if up {
        ZOOMS
            .iter()
            .copied()
            .find(|z| *z > current)
            .unwrap_or(*ZOOMS.last().unwrap())
    } else {
        ZOOMS
            .iter()
            .rev()
            .copied()
            .find(|z| *z < current)
            .unwrap_or(ZOOMS[0])
    }
}

/// Walk one step along VLC's Crop menu, wrapping at both ends: the menu starts
/// with the whole picture, so the key that crops also brings it back. A shape
/// the menu does not list (from `--crop`, say) has no place in it, so the walk
/// starts from the top rather than inventing one.
fn crop_step(current: (u32, u32), forward: bool) -> (u32, u32) {
    let at = CROPS
        .iter()
        .position(|shape| *shape == current)
        .unwrap_or(0);
    let next = if forward {
        (at + 1) % CROPS.len()
    } else {
        (at + CROPS.len() - 1) % CROPS.len()
    };
    CROPS[next]
}

/// Walk one step along VLC's aspect-ratio menu, wrapping at both ends: the menu
/// starts at the file's own shape, so the key that forces one gives it back. A
/// shape the menu does not list (from `--aspect`, say) has no place in it, so
/// the walk starts from the top rather than inventing one.
fn aspect_step(current: Aspect, forward: bool) -> Aspect {
    let at = ASPECTS
        .iter()
        .position(|shape| *shape == current)
        .unwrap_or(0);
    let next = if forward {
        (at + 1) % ASPECTS.len()
    } else {
        (at + ASPECTS.len() - 1) % ASPECTS.len()
    };
    ASPECTS[next]
}

/// Frame cadence at a playback rate: showing each picture for shorter means the
/// timeline advances `milli`/1000 times faster.
fn paced_period(period: Duration, milli: u32) -> Duration {
    if milli == 1_000 {
        return period;
    }
    Duration::try_from_secs_f64(period.as_secs_f64() * 1_000.0 / f64::from(milli)).unwrap_or(period)
}

/// The control line for a volume change, muted or not.
fn volume_osd(milli: u32, muted: bool) -> String {
    if muted {
        return "Muted".to_owned();
    }
    format!("Volume {}%", milli.div_euclid(10))
}

/// A thousandths level spoken as a multiple with no trailing zeros: `2000` is
/// `2×`, `250` is `0.25×`.
fn times(milli: u32) -> String {
    let text = format!("{:.2}", f64::from(milli) / 1000.0);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text}\u{d7}")
}

/// The control line for a rate change, as a multiple rather than a percent.
fn rate_osd(milli: u32) -> String {
    times(milli)
}

/// The control line for a magnification change, which VLC's menu states the
/// same way: a multiple of the picture as fitted.
fn zoom_osd(milli: u32) -> String {
    format!("Zoom {}", times(milli))
}

/// A crop shape the way VLC's menu writes it: `4:3`, and a decimal for the
/// cinema ratios it stores as hundredths (`185:100` is `1.85:1`).
fn crop_label(shape: (u32, u32)) -> String {
    if shape == NO_CROP {
        return "Default".to_owned();
    }
    if shape.1 == 100 {
        return format!("{}.{:02}:1", shape.0 / 100, shape.0 % 100);
    }
    format!("{}:{}", shape.0, shape.1)
}

/// The control line for a crop change, naming the shape the picture keeps.
fn crop_osd(shape: (u32, u32)) -> String {
    format!("Crop {}", crop_label(shape))
}

/// A forced shape the way VLC's menu writes it: `4:3`, a decimal for the cinema
/// ratios it stores as hundredths (`235:100` is `2.35:1`), and one word for
/// each of the two choices that name no shape.
fn aspect_label(shape: Aspect) -> String {
    match shape {
        Aspect::Source => "Default".to_owned(),
        Aspect::Fill => "Fill".to_owned(),
        Aspect::Shape(x, 100) => format!("{}.{:02}:1", x / 100, x % 100),
        Aspect::Shape(x, y) => format!("{x}:{y}"),
    }
}

/// The control line for an aspect change, naming the shape the picture is drawn
/// at.
fn aspect_osd(shape: Aspect) -> String {
    format!("Aspect {}", aspect_label(shape))
}

/// VLC's absolute position keys: `0` starts the file, `9` jumps to 90%.
fn position_from_digit(digit: u8) -> Option<f32> {
    if digit > 9 {
        return None;
    }
    Some(f32::from(digit) / 10.0)
}

/// The size a picture of these coded pixels takes on screen: its height stands,
/// and its width follows the shape the container gives each pixel.
fn display_size(coded: Vec2, pixel_aspect: (u32, u32)) -> Vec2 {
    Vec2::new(
        coded.x * pixel_aspect.0 as f32 / pixel_aspect.1 as f32,
        coded.y,
    )
}

/// The size the picture on screen takes: the shape its container declares for
/// its pixels, the shape VLC's aspect-ratio menu forces on it, or the shape of
/// the frame it is shown in. As in `display_size`, the height stands and the
/// width follows, and `video_rect` fits the result into the frame either way, so
/// what reaches the frame is the proportion.
fn shown_size(coded: Vec2, pixel_aspect: (u32, u32), aspect: Aspect, frame: Vec2) -> Vec2 {
    match aspect {
        Aspect::Source => display_size(coded, pixel_aspect),
        Aspect::Shape(x, y) => Vec2::new(coded.y * x as f32 / y as f32, coded.y),
        Aspect::Fill => frame,
    }
}

/// Where the picture of a display size lands in the frame: the whole of it
/// fitted inside at 1×, and a magnification grown out of that centre, slid by
/// the drag that follows it. The slide only counts while the picture is bigger
/// than the frame, and never far enough to bare an edge the picture could have
/// filled — so a fitted picture keeps the letterbox the fitting left it with,
/// and a magnified one keeps the frame covered.
fn video_rect(frame: Rect, size: Vec2, zoom_milli: u32, pan: Vec2) -> Rect {
    let fit = (frame.width() / size.x)
        .min(frame.height() / size.y)
        .max(0.0);
    let drawn = size * (fit * zoom_milli as f32 / 1_000.0);
    let room = (drawn - frame.size()).max(Vec2::ZERO) / 2.0;
    let offset = Vec2::new(pan.x.clamp(-room.x, room.x), pan.y.clamp(-room.y, room.y));
    Rect::from_center_size(frame.center() + offset, drawn)
}

/// The part of a picture VLC's Crop menu keeps: the largest centred region
/// whose displayed shape is the ratio, returned as the pixels cut from each
/// edge — left, top, right, bottom. The shape is the one you see, so a file
/// whose container stretches its pixels has the stretch counted first, and a
/// wider picture than the ratio loses its sides while a taller one loses its
/// top and bottom. The menu's Default cuts nothing.
fn crop_insets(source: Vec2, pixel_aspect: (u32, u32), shape: (u32, u32)) -> [u32; 4] {
    if shape == NO_CROP {
        return [0, 0, 0, 0];
    }
    // The same shape measured in stored pixels: a pixel stretched wider needs
    // fewer of them to span the ratio.
    let wanted = f64::from(shape.0) / f64::from(shape.1) * f64::from(pixel_aspect.1)
        / f64::from(pixel_aspect.0);
    let stored = f64::from(source.x) / f64::from(source.y);
    let (keep_x, keep_y) = if stored > wanted {
        let width = f64::from(source.y) * wanted;
        (
            width.round().min(f64::from(source.x)).max(1.0) as u32,
            source.y as u32,
        )
    } else {
        let height = f64::from(source.x) / wanted;
        (
            source.x as u32,
            height.round().min(f64::from(source.y)).max(1.0) as u32,
        )
    };
    let (across, down) = (
        source.x as u32 - keep_x.min(source.x as u32),
        source.y as u32 - keep_y.min(source.y as u32),
    );
    // An odd remainder leaves the extra pixel on the far edge, so the region
    // stays centred as closely as whole pixels allow.
    [across / 2, down / 2, across - across / 2, down - down / 2]
}

/// The size a picture has left after cropping, in the pixels it is stored in.
fn cropped_size(source: Vec2, insets: [u32; 4]) -> Vec2 {
    Vec2::new(
        source.x - insets[0] as f32 - insets[2] as f32,
        source.y - insets[1] as f32 - insets[3] as f32,
    )
}

/// Where the cropped part of a picture sits inside the whole of it, as texture
/// coordinates: the first two numbers are the corner the region starts at, the
/// last two the corner it ends at. Both planes of a frame are addressed by the
/// same fractions, however differently they are sampled.
fn uv_window(source: Vec2, insets: [u32; 4]) -> [f32; 4] {
    let (x, y) = (source.x.max(1.0), source.y.max(1.0));
    [
        insets[0] as f32 / x,
        insets[1] as f32 / y,
        (x - insets[2] as f32) / x,
        (y - insets[3] as f32) / y,
    ]
}

/// The pixels to cut from each edge of a frame: the borders its container
/// itself states, and then the Crop menu's shape measured against the region
/// those borders leave, so the two cuts stack from the coded edges inward.
/// A frame the container's own borders cannot fit inside — one smaller than
/// the file described — is shown whole rather than cut to nothing.
fn shown_insets(
    source: Vec2,
    base: [u32; 4],
    pixel_aspect: (u32, u32),
    shape: (u32, u32),
) -> [u32; 4] {
    let base = if f64::from(base[0]) + f64::from(base[2]) < f64::from(source.x.max(1.0))
        && f64::from(base[1]) + f64::from(base[3]) < f64::from(source.y.max(1.0))
    {
        base
    } else {
        [0; 4]
    };
    let inner = crop_insets(cropped_size(source, base), pixel_aspect, shape);
    [
        base[0] + inner[0],
        base[1] + inner[1],
        base[2] + inner[2],
        base[3] + inner[3],
    ]
}

/// The picture settings of VLC's Adjustments tab: a switch and five sliders
/// whose ranges and defaults are VLC's own (`adjust-hue` −180..180°,
/// `adjust-brightness` and `adjust-contrast` 0..2, `adjust-saturation` 0..3,
/// `adjust-gamma` 0.01..10, all of the last four defaulting to 1 and the hue
/// to 0). The values survive a disabled switch and a new item, as they
/// survive in VLC until its dialog turns them off.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Adjust {
    on: bool,
    hue: f32,
    brightness: f32,
    contrast: f32,
    saturation: f32,
    gamma: f32,
}

impl Default for Adjust {
    fn default() -> Self {
        Self {
            on: false,
            hue: 0.0,
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            gamma: 1.0,
        }
    }
}

/// The bundle that leaves every pixel as it stands: the sliders' own defaults
/// with the switch off, one of each multiplier and no turn. Shared with the
/// GPU path, whose uniform carries the same five numbers.
const ADJUST_IDENTITY: [f32; 5] = crate::player_gpu::IDENTITY_ADJUST;

/// The five numbers VLC's adjust filter precomputes from the sliders, as one
/// bundle the draw paths consume: the luma becomes
/// `gamma⁻¹(clip(lum + contrast·y))` on stored samples and the chroma pair
/// `sat·(cos·u + sin·v)` and `sat·(cos·v − sin·u)` around grey, which is what
/// the filter's tables and its fixed-point chroma math evaluate to. Disabled,
/// or at the dialog's defaults, the bundle is the identity and both paths
/// leave the pixels alone.
fn adjust_scalars(adjust: &Adjust) -> [f32; 5] {
    if !adjust.on {
        return ADJUST_IDENTITY;
    }
    let hue = adjust.hue.to_radians();
    let sat = adjust.saturation;
    [
        adjust.contrast,
        (adjust.brightness - 1.0) * 255.0 + 128.0 * (1.0 - adjust.contrast),
        1.0 / adjust.gamma,
        sat * hue.cos(),
        sat * hue.sin(),
    ]
}

/// One pixel through the luma half of the bundle: the contrast and brightness
/// line, clamped to the sample range, then the gamma curve on top.
fn adjust_luma(y: f32, s: &[f32; 5]) -> f32 {
    let t = (s[1] + s[0] * y).clamp(0.0, 255.0);
    (t / 255.0).powf(s[2]) * 255.0
}

/// The picture of an RGB frame with the bundle applied — the software draw
/// path, whose pixels have already left the planes behind. They are stepped
/// back through BT.601 into luma and centred chroma, the two halves of the
/// bundle run over them, and the result returns as RGB, clamped like the
/// filter's tables clamp. Grey (chroma at zero and luma at either anchor)
/// behaves as it does in VLC: grey stays grey through a hue turn.
fn adjust_rgb(source: &[u8], out: &mut Vec<u8>, s: &[f32; 5]) {
    out.clear();
    out.reserve(source.len());
    for pixel in source.as_chunks::<3>().0.iter() {
        let (r, g, b) = (
            f32::from(pixel[0]),
            f32::from(pixel[1]),
            f32::from(pixel[2]),
        );
        // The converter's fixed-point 601 matrix solved back for the samples
        // it read: the luma row divided by the 255/219 stretch it rode, the
        // chroma rows scaled back to the 224-wide chroma range and recentred.
        let y = (0.299 * r + 0.587 * g + 0.114 * b) / 1.164 + 16.0;
        let cb = (-0.168_736 * r - 0.331_264 * g + 0.5 * b) * (224.0 / 255.0);
        let cr = (0.5 * r - 0.418_688 * g - 0.081_312 * b) * (224.0 / 255.0);
        let y = adjust_luma(y, s);
        let cb = s[3] * cb + s[4] * cr;
        let cr = s[3] * cr - s[4] * cb;
        let y = 1.164 * (y - 16.0);
        let r = y + 1.596 * cr;
        let g = y - 0.391 * cb - 0.813 * cr;
        let b = y + 2.013 * cb;
        out.extend([r, g, b].map(|v| v.round().clamp(0.0, 255.0) as u8));
    }
}

/// How far a frame may lead the audio clock before presentation waits. The
/// audio clock only advances in whole device buffers, and ITU-R BT.1359 puts
/// the perceptibility of audio lagging picture at roughly ten frames, so this
/// stays above the stutter a tighter gate would cause at every rate.
pub const AUDIO_SYNC_SLACK: f64 = 0.05;

/// One list of cues the player can show: a sidecar file or a text track the
/// container carries. `label` is what the track list and the OSD show.
struct SubtitleSource {
    label: String,
    cues: Vec<Cue>,
}

/// A named part of the open file, from the chapter list its container carries.
struct ChapterMark {
    start: Duration,
    title: String,
}

/// Which file the picker standing in front of the viewer was opened for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    /// A file to play, appended to the queue.
    Item,
    /// A file of cues to caption the item on screen.
    Subtitles,
}

struct Player {
    /// The decoding thread for the open file.
    playback: Option<Playback>,
    /// Audio playback thread for A/V sync (when audio track is present).
    audio: Option<crate::audio_thread::AudioPlayback>,
    /// Audio reached its end or failed, so it no longer gates presentation.
    audio_ended: bool,
    playback_activity: Option<fvid_platform::PlaybackActivity>,
    duration: Option<Duration>,
    seekable: bool,
    hardware: bool,
    /// Which codec the picture on screen is decoded from, spelled the way a
    /// viewer spells it; empty for an item with no picture at all.
    video_codec: &'static str,
    /// How many bytes the item on screen occupies in the file system, read once
    /// as it opens. `None` when the file was already unreadable by then.
    bytes: Option<u64>,
    /// What the open item's container says about the file as a whole. The panel
    /// leads with these facts and skips the ones the file leaves unstated, since
    /// the title above already says what the file is called where it lies.
    file_tags: FileTags,
    /// Whether the panel saying what the item on screen is stays on it. Like the
    /// crop and the magnification, it belongs to the session and survives a
    /// change of item, so the next file is described in the same place.
    info: bool,
    /// Whether VLC's Ctrl+E dialog of picture settings stands over the item.
    /// Like the settings themselves it belongs to the session; opening it also
    /// keeps the controls from fading, as the information panel does.
    effects: bool,
    /// The five sliders and their switch, kept across items the way VLC keeps
    /// the filter's settings running until its dialog turns them off.
    adjust: Adjust,
    /// The colour the command line asked for: the camera log to read the coded
    /// values as, the highlight compression to run, and the LUT to apply after
    /// them. Held for the session like the sliders, but baked again for every
    /// item from the signal that item states, so one request means a different
    /// conversion on a BT.709 file and a BT.2100 one.
    grading: Grading,
    /// The last RGB frame as the reader handed it over, with the bundle its
    /// texture was built from, so a slider moved while that same frame stands
    /// on screen can rebuild it. The GPU path keeps no copy: its bundle rides
    /// the uniform, and a repaint picks it up the way it picks up a crop.
    rgb_frame: Option<(Vec<u8>, [usize; 2], [f32; 5])>,
    /// Geometry, period and timeline interval of the frame on screen.
    dimensions: [usize; 2],
    /// How much wider a coded pixel of the item on screen is than it is tall,
    /// as that item's container states it.
    pixel_aspect: (u32, u32),
    /// The borders the item's container asks to be kept off screen, as pixel
    /// insets into the coded frame: what Matroska's `PixelCrop*` elements
    /// state, `[0; 4]` for a file that states none.
    container_insets: [u32; 4],
    /// The colour signal the item on screen states for itself and the light it
    /// carries, kept from the open so the panel can say what the picture is
    /// before anything is done to it.
    signal: ColourDescription,
    hdr: HdrMetadata,
    /// The panel's line for what is being done to the item on screen: the plan
    /// the grade baked, the reading it applies to that grid and the look that
    /// follows it, said while the grade is still in hand. `None` when the item's
    /// bytes are the picture. The line says what is being done, not what was
    /// asked, because the two part whenever fvid picks a shoulder on its own.
    grade_line: Option<String>,
    /// Which rung of VLC's Zoom menu the picture is drawn at, kept across files
    /// like the level and the rate.
    zoom_milli: u32,
    /// How far a magnified picture has been dragged off the centre of the frame,
    /// in points. A picture that fits the frame ignores it.
    pan: Vec2,
    /// Which shape of VLC's Crop menu the picture is cut to, kept across files
    /// like the level and the magnification. The menu's Default shows all of it.
    crop: (u32, u32),
    /// Which shape of VLC's aspect-ratio menu the picture is drawn at, kept
    /// across files like the crop and the magnification. The menu's Default
    /// forces nothing and leaves the shape the container declared.
    aspect: Aspect,
    period: Duration,
    interval: Option<(u128, u128, u32)>,
    /// The next decoded frame, waiting for its presentation deadline.
    queued: Option<Frame>,
    buffering: bool,
    seek_preview: bool,
    seek_target: Option<Duration>,
    /// Frame on screen when it is drawn by the GPU shader (planar).
    video: Option<(Arc<Planar8>, u64)>,
    /// Frame on screen when it arrived as packed RGB (Y4M, WebM).
    texture: Option<egui::TextureHandle>,
    /// The frame last shown, in whichever layout it came. The snapshot key
    /// writes this, so nothing is copied to keep it: the frame is moved here
    /// once its pixels have reached the screen.
    presented: Option<Frame>,
    name: String,
    error: Option<String>,
    paused: bool,
    ended: bool,
    deadline: Instant,
    presentation_stats: Option<(Instant, u32)>,
    starved_polls: u32,
    /// Last pointer movement or click; drives the controls fade-out.
    activity: Instant,
    /// An open file picker, polled once per frame so the event loop never nests.
    /// A file picker standing between the viewer and an answer, along with the
    /// kind of answer it was opened for.
    dialog: Option<(Pick, Dialog)>,
    /// Fraction of the timeline under a drag on the progress line, shown until release.
    scrub: Option<f32>,
    /// Output gain in thousandths, kept across files.
    volume_milli: u32,
    muted: bool,
    /// Playback rate in thousandths, kept across files.
    rate_milli: u32,
    /// Frames still to show while paused, asked for by the next-frame key.
    step: u32,
    /// Transient control message (volume, rate, jump) and when it appeared.
    osd: Option<(String, Instant)>,
    /// Cues from the subtitle source on screen.
    cues: Vec<Cue>,
    /// Label of that source: a sidecar file name or an embedded track name.
    subtitle_name: String,
    /// Every subtitle source the open file offers, sidecar first.
    subtitle_sources: Vec<SubtitleSource>,
    /// Which of `subtitle_sources` is on screen.
    subtitle_source: usize,
    /// Named parts of the open file, in order of their start; empty for a file
    /// that names none, which is most of them.
    chapters: Vec<ChapterMark>,
    subtitle_shown: bool,
    /// Signed shift of the cue timeline in milliseconds; positive shows later.
    subtitle_delay_ms: i64,
    /// Text size and the gap under the picture, both driven by keys.
    subtitle_font: f32,
    subtitle_margin: f32,
    /// Files queued for playback; the one on screen sits at `index`.
    queue: Vec<PathBuf>,
    index: usize,
    repeat: Repeat,
    /// VLC's Random (`s`): whether the list walks in a drawn order, every item
    /// of the queue playing once per cycle before the list ends — or before a
    /// repeating one deals the next cycle.
    shuffle: bool,
    /// Items the current cycle still has to draw, the next draw last.
    upcoming: Vec<usize>,
    /// Items the cycle has played, the one on screen last; the previous key
    /// walks back along these, the way VLC's history does.
    walked: Vec<usize>,
    /// The generator state the cycles are dealt from; mixed anew on every
    /// toggle, so the same list on another day walks another way.
    shuffle_seed: u64,
    /// A–B loop marks in media time; the picture rewinds to `loop_a` once it
    /// reaches `loop_b`.
    loop_a: Option<Duration>,
    loop_b: Option<Duration>,
    /// Signed shift of the audio clock in milliseconds; positive lets the
    /// picture run ahead, so sound arrives later.
    audio_delay_ms: i64,
    /// Path of the item on screen; the audio track key reopens it.
    opened: Option<PathBuf>,
    /// Audio tracks of the open file that a decoder exists for.
    audio_tracks: Vec<crate::audio::AudioTrack>,
    /// Which of `audio_tracks` the sound is reading.
    audio_track: usize,
    /// Which codec the sound on screen is decoded from, spelled the way a
    /// viewer spells it; empty while no sound is open. The panel names the
    /// coding VLC's information window names, and the tag a container carries
    /// is not that spelling: every container files the same sound under its
    /// own name — AAC is `mp4a` in MP4 and PCM `sowt` in QuickTime — so the
    /// answer comes from the stream that opened, folded by the same kind of
    /// table the picture's `mp4_codec` is.
    sound_codec: String,
    /// Where `--start-time` and `--stop-time` bound the item being opened; the
    /// queue's later items are left alone.
    bounds: PlayBounds,
    /// Media time the current item ends at, from `--stop-time`.
    stop_time: Option<Duration>,
    /// `--start-paused`: every item the command line opens waits for a key.
    start_paused: bool,
    /// `--no-audio`: the session has no sound, whatever the files carry.
    no_audio: bool,
    /// `--audio-track` and `--subtitle-track`: the tracks the command line
    /// asked for, held for every item rather than the queue's first one, as
    /// they name a preference rather than a place on the timeline.
    preferred_audio: Option<usize>,
    preferred_subtitle: Option<usize>,
    /// `--sub-file`: cues from this file lead every item's own list, since a
    /// named file is a likelier choice than the one guessed from its name.
    subtitle_file: Option<PathBuf>,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            playback: None,
            audio: None,
            audio_ended: false,
            playback_activity: None,
            duration: None,
            seekable: false,
            hardware: false,
            video_codec: "",
            bytes: None,
            file_tags: FileTags::default(),
            info: false,
            effects: false,
            adjust: Adjust::default(),
            grading: Grading::default(),
            rgb_frame: None,
            dimensions: [0; 2],
            pixel_aspect: (1, 1),
            container_insets: [0; 4],
            signal: ColourDescription::default(),
            hdr: HdrMetadata::default(),
            grade_line: None,
            zoom_milli: 1_000,
            pan: Vec2::ZERO,
            crop: NO_CROP,
            aspect: Aspect::Source,
            period: Duration::ZERO,
            interval: None,
            queued: None,
            buffering: true,
            seek_preview: false,
            seek_target: None,
            video: None,
            texture: None,
            presented: None,
            name: String::new(),
            error: None,
            paused: false,
            ended: false,
            deadline: Instant::now(),
            presentation_stats: std::env::var_os("FVID_PLAYER_STATS").map(|_| (Instant::now(), 0)),
            starved_polls: 0,
            activity: Instant::now(),
            dialog: None,
            scrub: None,
            volume_milli: 1_000,
            muted: false,
            rate_milli: 1_000,
            step: 0,
            osd: None,
            cues: Vec::new(),
            subtitle_name: String::new(),
            subtitle_sources: Vec::new(),
            chapters: Vec::new(),
            subtitle_source: 0,
            subtitle_shown: true,
            subtitle_delay_ms: 0,
            subtitle_font: 22.0,
            subtitle_margin: 48.0,
            queue: Vec::new(),
            index: 0,
            repeat: Repeat::default(),
            shuffle: false,
            upcoming: Vec::new(),
            walked: Vec::new(),
            shuffle_seed: 0,
            loop_a: None,
            loop_b: None,
            audio_delay_ms: 0,
            opened: None,
            audio_tracks: Vec::new(),
            audio_track: 0,
            sound_codec: String::new(),
            bounds: PlayBounds::default(),
            stop_time: None,
            start_paused: false,
            no_audio: false,
            preferred_audio: None,
            preferred_subtitle: None,
            subtitle_file: None,
        }
    }
}

impl Player {
    fn open(&mut self, path: PathBuf) -> crate::Result<()> {
        let video = NativeReader::without_memory_limit(BufReader::new(File::open(&path)?))
            .and_then(|mut reader| {
                if reader.read_frame()? {
                    Ok(reader)
                } else {
                    Err(crate::invalid("video has no frames"))
                }
            });
        let mut reader = match video {
            Ok(reader) => reader,
            // A file with nothing to show can still be all sound, and a listener
            // with only ears still wants it played. The reason the picture
            // refused stays on screen when there is no track to hear either.
            Err(video) => {
                if self.open_sound_only(&path) {
                    return Ok(());
                }
                if self.no_audio {
                    return Err(crate::invalid(
                        "--no-audio leaves a file with no picture with nothing to play",
                    ));
                }
                return Err(video);
            }
        };
        // `--start-time` and `--stop-time` belong to the first item of the queue.
        let bounds = std::mem::take(&mut self.bounds);
        if let Some(start) = bounds.start {
            reader.seek(start)?;
        }
        self.stop_time = bounds.stop;
        self.duration = reader.duration();
        self.report_duration(&path);
        self.seekable = reader.seekable();
        self.hardware = reader.hardware_accelerated();
        self.video_codec = reader.video_codec();
        self.bytes = file_size(&path);
        self.dimensions = reader.dimensions();
        self.pixel_aspect = reader.pixel_aspect();
        self.container_insets = reader.insets();
        self.period = reader.frame_period();
        self.interval = None;
        // Drop the old thread before starting the new one.
        self.playback = None;
        self.audio = None;
        self.audio_ended = false;
        self.queued = None;
        self.buffering = true;
        self.seek_preview = false;
        self.seek_target = None;
        // A–B marks belong to the item that was on screen before this one.
        self.loop_a = None;
        self.loop_b = None;
        // What the colour request means depends on the signal this item states
        // for itself, so the grade is baked from it here, before the reader
        // moves into the thread that will carry the answer.
        let (signal, hdr) = (reader.colour(), reader.hdr());
        let grade = self.grading.grade_for(signal, &hdr);
        self.grade_line = grade.as_ref().map(|grade| {
            format!(
                "Grade: {}",
                grade_text(&grade.plan(), grade.interpolation(), grade.lut())
            )
        });
        self.signal = signal;
        self.hdr = hdr;
        self.playback = Some(Playback::start(reader, grade));
        // The picture of the item before this one is no longer on screen.
        self.presented = None;
        // Try to start audio playback if the file has an audio track.
        self.opened = Some(path.clone());
        // The track list belongs to the file, so it starts empty again here; a
        // failed audio open leaves it empty rather than stale.
        self.audio_tracks = Vec::new();
        self.audio_track = 0;
        self.sound_codec = String::new();
        self.open_item_audio(&path);
        self.apply_audio_controls();
        self.load_subtitles(&path);
        let facts = container_facts(&path);
        self.chapters = facts.chapters;
        self.file_tags = facts.tags;
        self.deadline = Instant::now();
        if let Some((_, count)) = &mut self.presentation_stats {
            *count = 0;
        }
        self.name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.error = None;
        self.paused = false;
        self.ended = false;
        self.hold_start();
        Ok(())
    }

    /// Open an item that has no playable picture as a listener's item: the
    /// audio thread carries the whole timeline, and the stage stays empty.
    /// False when the file has no decodable audio track either, so the caller
    /// can report the reason the picture refused instead.
    fn open_sound_only(&mut self, path: &Path) -> bool {
        // The bounds belong to the first item of the queue, as they do for a
        // picture: the sound starts where the command said and stops there too.
        let bounds = std::mem::take(&mut self.bounds);
        self.stop_time = bounds.stop;
        self.playback = None;
        self.audio = None;
        self.seekable = false;
        self.audio_ended = false;
        self.queued = None;
        self.buffering = false;
        self.seek_preview = false;
        self.seek_target = None;
        self.presented = None;
        self.video = None;
        self.texture = None;
        self.rgb_frame = None;
        self.interval = None;
        self.dimensions = [0; 2];
        self.pixel_aspect = (1, 1);
        self.container_insets = [0; 4];
        // An item with no picture states no colour and gets no grade.
        self.signal = ColourDescription::default();
        self.hdr = HdrMetadata::default();
        self.grade_line = None;
        self.period = Duration::ZERO;
        self.hardware = false;
        // An item that refused the picture has no codec to name for it.
        self.video_codec = "";
        self.bytes = file_size(path);
        self.loop_a = None;
        self.loop_b = None;
        self.opened = Some(path.to_path_buf());
        self.audio_tracks = Vec::new();
        self.audio_track = 0;
        self.sound_codec = String::new();
        if !self.open_item_audio(path) {
            return false;
        }
        // Every audio stream the player can decode carries a seek table.
        self.seekable = true;
        self.duration = self.audio.as_ref().and_then(|audio| audio.duration());
        self.report_duration(path);
        self.apply_audio_controls();
        if let Some(start) = bounds.start
            && let Some(audio) = &mut self.audio
        {
            audio.seek(start);
        }
        // With no picture there is no preroll to hold the clock for, so the
        // sound starts as soon as the item is open.
        if let Some(audio) = &self.audio {
            audio.play();
        }
        self.load_subtitles(path);
        let facts = container_facts(path);
        self.chapters = facts.chapters;
        self.file_tags = facts.tags;
        self.name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.error = None;
        self.paused = false;
        self.ended = false;
        self.hold_start();
        self.deadline = Instant::now();
        if let Some((_, count)) = &mut self.presentation_stats {
            *count = 0;
        }
        true
    }

    /// An item is on screen even when it is all sound and has nothing to show.
    fn item_open(&self) -> bool {
        self.playback.is_some() || self.audio.is_some()
    }

    /// With the timing switch on, say what the item's timeline turned out to be.
    fn report_duration(&self, path: &Path) {
        if self.presentation_stats.is_some() {
            let shown = path.file_name().unwrap_or_default().to_string_lossy();
            eprintln!("player timeline: {shown} duration={:?}", self.duration);
        }
    }

    /// Open one of the file's audio tracks and start decoding it on its own
    /// thread. The file is offered to every audio reader the player has, in the
    /// order it holds them, until one reads a track out of it: the video reader
    /// accepts either main container and reports no codec information of its own.
    /// False when the file has no such track.
    fn try_start_audio(&mut self, path: &Path, nth: usize) -> bool {
        let mp4 = File::open(path).ok().and_then(|file| {
            crate::playback_mp4_audio::Mp4AudioReader::open_at(
                BufReader::new(file),
                crate::container::mp4::Limits::default(),
                nth,
            )
            .ok()
            .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
        });
        let stream = mp4
            .or_else(|| {
                File::open(path)
                    .ok()
                    .and_then(|file| {
                        crate::playback_webm_audio::WebmAudioReader::open_at(
                            BufReader::new(file),
                            crate::container::webm::Limits::default(),
                            nth,
                        )
                        .ok()
                    })
                    .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
            })
            // An AVI file keeps one program's audio in a stream of its own among the
            // file's several, so the track key counts those; its reader frames records
            // for the codings whose record counting it has measured, of which the
            // Microsoft-spelled ADPCM blocks are the ones this build decodes.
            .or_else(|| {
                File::open(path)
                    .ok()
                    .and_then(|file| {
                        crate::playback_avi_audio::AviAudioReader::open_at(
                            BufReader::new(file),
                            crate::container::avi::Limits::default(),
                            nth,
                        )
                        .ok()
                    })
                    .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
            })
            // A MIDI performance is one track of its own, so the second and later
            // track keys have nothing to land on.
            .or_else(move || {
                if nth > 0 {
                    return None;
                }
                File::open(path)
                    .ok()
                    .and_then(|file| {
                        crate::playback_smf::SmfAudioReader::open(
                            BufReader::new(file),
                            crate::container::smf::Limits::default(),
                        )
                        .ok()
                    })
                    .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
            })
            // A module is one performance too, and one track of it: every channel
            // is mixed into what is heard rather than offered separately.
            .or_else(move || {
                if nth > 0 {
                    return None;
                }
                File::open(path)
                    .ok()
                    .and_then(|file| {
                        crate::playback_xm::XmAudioReader::open(
                            BufReader::new(file),
                            crate::container::xm::Limits::default(),
                        )
                        .ok()
                    })
                    .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
            })
            // An Ogg file names its codec in the first bytes of its first packet
            // rather than in a field, and one program can be written as several
            // chains after each other, so it is still one track to choose.
            .or_else(move || {
                if nth > 0 {
                    return None;
                }
                File::open(path)
                    .ok()
                    .and_then(|file| {
                        crate::playback_ogg_audio::OggAudioReader::open(
                            BufReader::new(file),
                            crate::container::ogg::Limits::default(),
                        )
                        .ok()
                    })
                    .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
            })
            // A Wave file holds one run of samples, so as with the two performances
            // above there is no second track key to land on.
            .or_else(move || {
                if nth > 0 {
                    return None;
                }
                File::open(path)
                    .ok()
                    .and_then(|file| {
                        crate::playback_wav::WavAudioReader::open(
                            BufReader::new(file),
                            crate::playback_wav::Limits::default(),
                        )
                        .ok()
                    })
                    .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
            })
            // An AIFF file holds one run too, with its geometry stated once and in
            // big-endian, and its rate as an 80-bit extended number.
            .or_else(move || {
                if nth > 0 {
                    return None;
                }
                File::open(path)
                    .ok()
                    .and_then(|file| {
                        crate::playback_aiff::AiffAudioReader::open(
                            BufReader::new(file),
                            crate::playback_aiff::Limits::default(),
                        )
                        .ok()
                    })
                    .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
            })
            // A Sun `.snd` header states only a geometry too, and reads even its
            // width through a table its consumers disagree about.
            .or_else(move || {
                if nth > 0 {
                    return None;
                }
                File::open(path)
                    .ok()
                    .and_then(|file| {
                        crate::playback_au::AuAudioReader::open(
                            BufReader::new(file),
                            crate::playback_au::Limits::default(),
                        )
                        .ok()
                    })
                    .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
            })
            // A `.flac` file names itself with `fLaC` and states its geometry once,
            // but its frames carry no length, so the walk that lists them proves
            // each end by the checksum the frame holds.
            .or_else(move || {
                if nth > 0 {
                    return None;
                }
                File::open(path)
                    .ok()
                    .and_then(|file| {
                        crate::playback_flac::FlacAudioReader::open(
                            BufReader::new(file),
                            crate::playback_flac::Limits::default(),
                        )
                        .ok()
                    })
                    .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
            })
            // A bare MPEG audio file states nothing before its frames at all, so it
            // opens last: its first bytes are free to look like anything, and every
            // other reader here begins with a magic that says what it is.
            .or_else(move || {
                if nth > 0 {
                    return None;
                }
                File::open(path)
                    .ok()
                    .and_then(|file| {
                        crate::playback_mp3::Mp3AudioReader::open(
                            BufReader::new(file),
                            crate::playback_mp3::Limits::default(),
                        )
                        .ok()
                    })
                    .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
            })
            // A bare Dolby Digital file states its geometry in every frame too, but
            // its frames open on a syncword no other reader here claims, so it is
            // offered alongside the MPEG one and only takes a file whose whole run of
            // frame lengths is measured out of Table 5.18.
            .or_else(move || {
                if nth > 0 {
                    return None;
                }
                File::open(path)
                    .ok()
                    .and_then(|file| {
                        crate::playback_ac3::Ac3AudioReader::open(
                            BufReader::new(file),
                            crate::playback_ac3::Limits::default(),
                        )
                        .ok()
                    })
                    .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
            })
            // An ADTS file names its geometry in its frame headers as the two
            // elementary readers above do, but it keeps no setup block, so its
            // reader synthesizes the one the MP4 decoder asks for. It is offered
            // last because its syncword is the loosest of the three: only a file
            // whose whole run of self-stated lengths fills it is taken.
            .or_else(move || {
                if nth > 0 {
                    return None;
                }
                File::open(path)
                    .ok()
                    .and_then(|file| {
                        crate::playback_aac::AacAudioReader::open(
                            BufReader::new(file),
                            crate::playback_aac::Limits::default(),
                        )
                        .ok()
                    })
                    .map(|reader| Box::new(reader) as Box<dyn crate::audio::AudioStream>)
            });
        let Some(stream) = stream else {
            return false;
        };
        // The container lists every track it can hand to a decoder, so the track
        // key knows how far to walk without opening the file again.
        self.audio_tracks = stream.audio_tracks();
        // The tag is read from the stream itself rather than from the track
        // list: the decoder was built for it, so this is the coding actually
        // sounding, which is what the panel is asked to name.
        self.sound_codec = sound_codec(stream.codec());
        self.audio_track = nth;
        self.audio = Some(crate::audio_thread::AudioPlayback::start(stream, || {
            Box::new(crate::audio::PlatformBackend::new())
        }));
        true
    }

    /// Start the sound of the item being opened, at the track the command line
    /// asked for. `--no-audio` starts nothing at all, and a number the file's
    /// own list does not reach leaves the first track, because that is the one
    /// there is to hear.
    fn open_item_audio(&mut self, path: &Path) -> bool {
        if self.no_audio {
            return false;
        }
        if !self.try_start_audio(path, 0) {
            return false;
        }
        let Some(nth) = self
            .preferred_audio
            .filter(|nth| *nth < self.audio_tracks.len())
        else {
            return true;
        };
        if nth == 0 {
            return true;
        }
        // Dropping the old handle stops its thread before the new one starts.
        self.audio = None;
        self.try_start_audio(path, nth)
    }

    /// Hold the item the command line asked to hold. Space lets it go again; the
    /// one-frame step shows its first picture, so a paused start is not a blank
    /// window.
    fn hold_start(&mut self) {
        if !self.start_paused {
            return;
        }
        self.paused = true;
        if let Some(audio) = &self.audio {
            audio.pause();
        }
        if let Some(playback) = &self.playback {
            playback.play();
            self.step = 1;
            self.buffering = true;
        }
    }

    /// Move the sound to the next or previous audio track of the open file. The
    /// new thread starts from where the picture stands, so switching tracks does
    /// not rewind the video or restart the item.
    fn cycle_audio_track(&mut self, forward: bool) {
        let len = self.audio_tracks.len();
        if len < 2 {
            self.show_osd(if len == 0 { "No audio" } else { "Audio 1/1" });
            return;
        }
        let Some(path) = self.opened.clone() else {
            return;
        };
        let nth = track_step(self.audio_track, len, forward);
        let at = self.timeline().0;
        let playing = !self.paused && !self.ended;
        // Dropping the old handle stops its thread before the new one starts.
        self.audio = None;
        if !self.try_start_audio(&path, nth) {
            self.show_osd("Audio track unavailable");
            return;
        }
        self.audio_ended = false;
        if let (Some(at), Some(audio)) = (at, &mut self.audio) {
            audio.seek(at);
        }
        self.apply_audio_controls();
        if playing && let Some(audio) = &self.audio {
            audio.play();
        }
        self.show_osd(format!(
            "Audio {}/{} {}",
            nth + 1,
            len,
            self.audio_tracks[nth].label()
        ));
    }

    /// Push the stored level and rate to the audio thread of the open track.
    fn apply_audio_controls(&self) {
        let Some(audio) = &self.audio else {
            return;
        };
        audio.set_volume(if self.muted { 0 } else { self.volume_milli });
        audio.set_rate(self.rate_milli);
    }

    /// Show a control message over the picture for `OSD_AFTER`.
    fn show_osd(&mut self, text: impl Into<String>) {
        self.osd = Some((text.into(), Instant::now()));
    }

    fn set_volume(&mut self, milli: u32) {
        self.volume_milli = milli.min(VOLUME_MAX);
        // Turning the dial up is how a muted player comes back on in VLC.
        self.muted = false;
        self.apply_audio_controls();
        self.show_osd(volume_osd(self.volume_milli, self.muted));
    }

    fn change_volume(&mut self, up: bool) {
        let next = volume_step(self.volume_milli, up);
        self.set_volume(next);
    }

    fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        self.apply_audio_controls();
        self.show_osd(volume_osd(self.volume_milli, self.muted));
    }

    fn set_rate(&mut self, milli: u32) {
        self.rate_milli = milli;
        self.apply_audio_controls();
        self.show_osd(rate_osd(self.rate_milli));
    }

    fn change_rate(&mut self, up: bool) {
        let next = rate_step(self.rate_milli, up);
        self.set_rate(next);
    }

    fn change_zoom(&mut self, up: bool) {
        self.zoom_milli = zoom_step(self.zoom_milli, up);
        self.show_osd(zoom_osd(self.zoom_milli));
    }

    /// Step along VLC's Crop menu and say which shape the picture keeps. What is
    /// cut away is gone from the picture rather than hidden behind an edge, so
    /// what is left is the part the frame now fills.
    fn change_crop(&mut self, forward: bool) {
        self.crop = crop_step(self.crop, forward);
        self.show_osd(crop_osd(self.crop));
    }

    /// Step along VLC's aspect-ratio menu and say which shape the picture now
    /// takes. Nothing is cut: the same pixels are simply drawn wider or taller,
    /// which is why the menu's Default hands back the shape the file declared.
    fn change_aspect(&mut self, forward: bool) {
        self.aspect = aspect_step(self.aspect, forward);
        self.show_osd(aspect_osd(self.aspect));
    }

    /// Open or close the panel saying what the item is. VLC keeps those facts in
    /// a window of its own; the player has no windows to spare, so they go over
    /// the picture and stay there until the key closes them.
    fn toggle_info(&mut self) {
        if !self.item_open() {
            self.show_osd("Nothing open");
            return;
        }
        self.info = !self.info;
    }

    /// Open or close the dialog of picture settings, the one VLC keeps under
    /// Ctrl+E in its Tools menu. VLC lets its adjust filter be configured even
    /// with nothing playing, so the dialog does not wait for an item either.
    fn toggle_effects(&mut self) {
        self.effects = !self.effects;
    }

    /// Rebuild the RGB texture when the settings bundle has changed while the
    /// same frame stood on screen — a slider moved over a paused picture, say.
    /// The GPU path needs no such work: its bundle rides the uniform, and the
    /// repaint a slider already causes picks it up the way it picks up a crop.
    fn refresh_rgb_texture(&mut self) {
        let scalars = adjust_scalars(&self.adjust);
        let Some(texture) = self.texture.as_mut() else {
            return;
        };
        let Some((raw, dimensions, applied)) = self.rgb_frame.as_mut() else {
            return;
        };
        if *applied == scalars {
            return;
        }
        let mut adjusted = Vec::new();
        let pixels = if scalars == ADJUST_IDENTITY {
            raw.as_slice()
        } else {
            adjust_rgb(raw, &mut adjusted, &scalars);
            &adjusted
        };
        texture.set(
            egui::ColorImage::from_rgb(*dimensions, pixels),
            egui::TextureOptions::LINEAR,
        );
        *applied = scalars;
    }

    /// Gather the subtitle sources of a file: a SubRip, WebVTT or ASS file
    /// sitting next to the video, the way VLC loads a sidecar without being
    /// told, then the plain-text tracks the container itself carries. The first
    /// source wins, so a sidecar the user placed deliberately beats an embedded
    /// track; a file nothing can be read from leaves the picture bare. A file
    /// `--sub-file` names leads all of them, and one that yields nothing says so
    /// rather than falling back unheard.
    fn load_subtitles(&mut self, path: &std::path::Path) {
        self.subtitle_sources = Vec::new();
        self.subtitle_source = 0;
        let mut unread = None;
        if let Some(named) = self.subtitle_file.clone() {
            match subtitles::load(&named) {
                Some(cues) => self.subtitle_sources.push(SubtitleSource {
                    label: named
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    cues,
                }),
                None => unread = Some(named),
            }
        }
        for candidate in subtitles::sidecars(path) {
            if let Some(cues) = subtitles::load(&candidate) {
                self.subtitle_sources.push(SubtitleSource {
                    label: candidate
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    cues,
                });
                break;
            }
        }
        self.subtitle_sources.extend(embedded_subtitles(path));
        if let Some(nth) = self
            .preferred_subtitle
            .filter(|nth| *nth < self.subtitle_sources.len())
        {
            self.subtitle_source = nth;
        }
        self.apply_subtitle_source();
        if let Some(named) = unread {
            // The name that was typed out loud outranks the list it fell back
            // to, so the fallback does not answer for it.
            self.show_osd(format!("Cannot read subtitles: {}", named.display()));
            return;
        }
        if !self.subtitle_name.is_empty() {
            self.show_osd(format!("Subtitles: {}", self.subtitle_name));
        }
    }

    /// Put a file the viewer named among the sources of the item on screen and
    /// turn to it, the way the sidecar and `--sub-file` are turned to. It joins
    /// the list rather than leading it, since it was asked for after everything
    /// the file came with; a file nothing can be read from leaves the list as it
    /// was and says so, the way a `--sub-file` that yields nothing does.
    fn add_subtitle_file(&mut self, path: PathBuf) {
        let named = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let Some(cues) = subtitles::load(&path) else {
            self.show_osd(format!("Cannot read subtitles: {}", path.display()));
            return;
        };
        self.subtitle_sources
            .push(SubtitleSource { label: named, cues });
        self.subtitle_source = self.subtitle_sources.len() - 1;
        self.apply_subtitle_source();
        self.subtitle_shown = true;
        self.show_osd(format!("Subtitles: {}", self.subtitle_name));
    }

    /// Put the selected source's cues on screen.
    fn apply_subtitle_source(&mut self) {
        match self.subtitle_sources.get(self.subtitle_source) {
            Some(source) => {
                self.cues = source.cues.clone();
                self.subtitle_name = source.label.clone();
            }
            None => {
                self.cues = Vec::new();
                self.subtitle_name.clear();
            }
        }
    }

    /// Walk the subtitle sources of the open file: sidecar first, then the
    /// container's tracks, wrapping at both ends.
    fn cycle_subtitle_track(&mut self, forward: bool) {
        let len = self.subtitle_sources.len();
        if len < 2 {
            self.show_osd(if len == 0 {
                "No subtitles"
            } else {
                "Subtitles 1/1"
            });
            return;
        }
        let nth = track_step(self.subtitle_source, len, forward);
        self.subtitle_source = nth;
        self.apply_subtitle_source();
        self.show_osd(format!(
            "Subtitles {}/{} {}",
            nth + 1,
            len,
            self.subtitle_name
        ));
    }

    /// Write the picture on screen out beside the file it came from. Only the
    /// video is saved: the subtitle line and the controls are drawn over it and
    /// never reach the frame.
    fn take_snapshot(&mut self) {
        let Some(path) = self.opened.clone() else {
            self.show_osd("Nothing to save");
            return;
        };
        if self.presented.is_none() {
            self.show_osd("Nothing to save");
            return;
        }
        let target = snapshot_name(&path, self.timeline().0.unwrap_or_default());
        let bytes = match self.picture() {
            Ok(bytes) => bytes,
            Err(error) => {
                self.show_osd(format!("Snapshot failed: {error}"));
                return;
            }
        };
        match std::fs::write(&target, bytes) {
            Ok(()) => self.show_osd(format!(
                "Saved {}",
                target.file_name().unwrap_or_default().to_string_lossy()
            )),
            Err(error) => self.show_osd(format!("Snapshot failed: {error}")),
        }
    }

    /// PNG bytes of the frame last shown, in whichever layout it arrived.
    fn picture(&self) -> crate::Result<Vec<u8>> {
        let frame = self
            .presented
            .as_ref()
            .ok_or_else(|| crate::invalid("nothing has been shown yet"))?;
        match &frame.pixels {
            Pixels::Planar(planes) => {
                let mut rgb = Vec::new();
                crate::playback_native::planar8_to_rgb(
                    planes,
                    &mut rgb,
                    planes.width * planes.height * 3,
                )?;
                crate::snapshot::png(planes.width, planes.height, &rgb)
            }
            Pixels::Rgb(rgb) => crate::snapshot::png(frame.dimensions[0], frame.dimensions[1], rgb),
        }
    }

    /// The line to paint over the picture at a media time, if one is live.
    /// A positive delay holds the cues back, so the picture is asked what was
    /// on screen that much earlier.
    fn subtitle_line(&self, at: Duration) -> Option<&str> {
        if !self.subtitle_shown || self.cues.is_empty() {
            return None;
        }
        let shift = Duration::from_millis(self.subtitle_delay_ms.unsigned_abs());
        let shifted = if self.subtitle_delay_ms >= 0 {
            at.checked_sub(shift)?
        } else {
            at + shift
        };
        subtitles::active(&self.cues, shifted, Duration::ZERO).map(|cue| cue.text.as_str())
    }

    /// Carry one keyboard control through to playback state.
    fn apply(&mut self, control: Control) {
        match control {
            Control::Volume(up) => self.change_volume(up),
            Control::Mute => self.toggle_mute(),
            Control::Rate(up) => self.change_rate(up),
            Control::RateFine(up) => self.set_rate(rate_fine(self.rate_milli, up)),
            Control::Reset => self.set_rate(1_000),
            Control::Jump(size, forward) => self.jump(size, forward),
            Control::Chapter(forward) => self.walk_chapter(forward),
            Control::Position(fraction) => self.seek_fraction(fraction),
            Control::Frame(forward) => self.step_frame(forward),
            Control::Subtitles => {
                if self.cues.is_empty() {
                    self.show_osd("No subtitles");
                } else {
                    self.subtitle_shown = !self.subtitle_shown;
                    self.show_osd(if self.subtitle_shown {
                        "Subtitles on"
                    } else {
                        "Subtitles off"
                    });
                }
            }
            Control::SubtitleDelay(later) => {
                self.subtitle_delay_ms = delay_step(self.subtitle_delay_ms, later);
                self.show_osd(format!("Subtitle delay {} ms", self.subtitle_delay_ms));
            }
            Control::SubtitleMargin(upper) => {
                self.subtitle_margin =
                    (self.subtitle_margin + if upper { -16.0 } else { 16.0 }).clamp(8.0, 400.0);
                self.show_osd(format!("Subtitles at {:.0} px", self.subtitle_margin));
            }
            Control::SubtitleSize(larger) => {
                self.subtitle_font =
                    (self.subtitle_font + if larger { 2.0 } else { -2.0 }).clamp(10.0, 72.0);
                self.show_osd(format!("Subtitle size {:.0}", self.subtitle_font));
            }
            Control::SubtitleTrack(forward) => self.cycle_subtitle_track(forward),
            Control::Next => self.step_queue(false),
            Control::Previous => self.step_queue(true),
            Control::Repeat => {
                self.repeat = cycle_repeat(self.repeat);
                self.show_osd(repeat_osd(self.repeat));
            }
            Control::Shuffle => self.toggle_shuffle(),
            Control::Loop => self.press_loop(),
            Control::AudioDelay(later) => {
                self.audio_delay_ms = delay_step(self.audio_delay_ms, later);
                self.show_osd(format!("Audio delay {} ms", self.audio_delay_ms));
            }
            Control::AudioTrack(forward) => self.cycle_audio_track(forward),
            Control::Snapshot => self.take_snapshot(),
            Control::Zoom(up) => self.change_zoom(up),
            Control::Crop(forward) => self.change_crop(forward),
            Control::Aspect(forward) => self.change_aspect(forward),
            Control::Info => self.toggle_info(),
            Control::Effects => self.toggle_effects(),
            Control::LoadSubtitles => self.pick_subtitles(),
        }
    }

    /// Press the loop key: the first go marks A, the next closes the region at
    /// B, and a third clears both. Rewinding is a seek, so a track that cannot
    /// seek has nothing to loop.
    fn press_loop(&mut self) {
        if !self.seekable {
            self.show_osd("Loop needs seeking");
            return;
        }
        let Some(now) = self.timeline().0 else {
            return;
        };
        let mark = loop_press(now, self.loop_a, self.loop_b);
        match mark {
            LoopMark::MarkedA(at) => {
                self.loop_a = Some(at);
                self.loop_b = None;
            }
            LoopMark::MarkedB(at) => self.loop_b = Some(at),
            LoopMark::Cleared => {
                self.loop_a = None;
                self.loop_b = None;
            }
        }
        self.show_osd(loop_osd(mark));
    }

    /// End the item once the `--stop-time` it was opened with is reached, the
    /// same way its last frame would. Without a position yet there is nothing to
    /// compare against, so a seek back past the bound lets the item run on.
    fn check_stop(&mut self) {
        let Some(stop) = self.stop_time else {
            return;
        };
        if self.timeline().0.is_some_and(|at| at >= stop) {
            self.stop_time = None;
            self.continue_queue();
        }
    }

    /// Replace the queue with `paths`, open the first of them, and remember
    /// where in the list the picture came from.
    fn open_queue(&mut self, paths: Vec<PathBuf>) {
        self.queue = paths;
        self.index = 0;
        if self.shuffle {
            // The old cycle belonged to the old list; a shuffled player gets
            // a freshly dealt one.
            self.deal_cycle();
        }
        let Some(path) = self.queue.first().cloned() else {
            return;
        };
        self.try_open(path);
    }

    /// Show item `index` of the queue, leaving the list itself alone. A file
    /// that will not open is reported and the picture stays where it was.
    fn play_index(&mut self, index: usize) {
        let Some(path) = self.queue.get(index).cloned() else {
            return;
        };
        if self.try_open(path) {
            self.index = index;
        }
    }

    /// Turn Random on or off. Turning it on keeps the item on screen and lines
    /// the rest of the list up behind it in a drawn order, the way VLC's
    /// shuffle places the current item in the new permutation and plays on
    /// from there; turning it off forgets the cycle and lets the plain order
    /// resume from wherever the walk stopped.
    fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        if self.shuffle {
            self.start_shuffle();
            self.show_osd("Shuffle on");
        } else {
            self.upcoming.clear();
            self.walked.clear();
            self.show_osd("Shuffle off");
        }
    }

    /// Begin a fresh shuffled cycle from the item on screen, mixing the clock
    /// into the seed so that the same list walks a different way each time it
    /// is started out with `--random` or turned on by the key.
    fn start_shuffle(&mut self) {
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        self.shuffle_seed ^= u64::from(since.subsec_nanos()) ^ since.as_secs();
        self.deal_cycle();
    }

    /// Line the items the list holds besides the one on screen up in a drawn
    /// order and start the walk fresh from the item on screen.
    fn deal_cycle(&mut self) {
        self.upcoming = deal_cycle(self.queue.len(), self.index, &mut self.shuffle_seed);
        self.walked = vec![self.index];
    }

    /// The next item of a shuffled walk: the last draw of what the cycle has
    /// left. A cycle that has run out deals the list anew when it repeats, as
    /// VLC does, and ends it otherwise; a list of one has nothing to draw, so
    /// repeating it is all there is.
    fn shuffle_forward(&mut self) -> Option<usize> {
        if let Some(next) = self.upcoming.pop() {
            self.walked.push(next);
            return Some(next);
        }
        if self.repeat != Repeat::All || self.queue.is_empty() {
            return None;
        }
        self.deal_cycle();
        match self.upcoming.pop() {
            Some(next) => {
                self.walked.push(next);
                Some(next)
            }
            None => Some(self.index),
        }
    }

    /// The previous item of a shuffled walk: back along the path it walked,
    /// leaving the item stepped past open for the next draw again. A cycle at
    /// its first item has nothing behind it.
    fn shuffle_back(&mut self) -> Option<usize> {
        if self.walked.len() < 2 {
            return None;
        }
        self.upcoming.push(self.index);
        self.walked.pop();
        self.walked.last().copied()
    }

    /// Step through the list with the next and previous keys; with Random on,
    /// the drawn cycle takes their walk instead of the numeric order.
    fn step_queue(&mut self, back: bool) {
        let len = self.queue.len();
        let target = if self.shuffle {
            if back {
                self.shuffle_back()
            } else {
                self.shuffle_forward()
            }
        } else if back {
            retreat(self.index, len, self.repeat)
        } else {
            advance(self.index, len, self.repeat)
        };
        match target {
            Some(target) => self.play_index(target),
            None => self.show_osd(if back {
                "First item"
            } else {
                "End of playlist"
            }),
        }
    }

    /// Carry playback into the next item when the picture runs out, the way a
    /// playlist continues by itself; a shuffled list continues along its cycle.
    fn continue_queue(&mut self) {
        let target = match self.repeat {
            Repeat::One => Some(self.index),
            _ if self.shuffle => self.shuffle_forward(),
            _ => advance(self.index, self.queue.len(), self.repeat),
        };
        match target {
            Some(target) => self.play_index(target),
            None => self.ended = true,
        }
    }

    fn try_open(&mut self, path: PathBuf) -> bool {
        match self.open(path.clone()) {
            Ok(()) => true,
            Err(error) => {
                let message = format!("{}: {error}", path.display());
                eprintln!("{message}");
                self.error = Some(message);
                false
            }
        }
    }

    fn pick_file(&mut self, what: Pick) {
        if self.dialog.is_some() {
            return;
        }
        // Each list is what the player can take at that place: the picture
        // readers' own set of containers, the bare audio files the frame walkers
        // open, and the three cue formats a sidecar or a `--sub-file` is read
        // from. VLC's dialog offers sound on its own, and a list that hides the
        // files behind the open dialog's filter would be a worse picker than the
        // command line.
        let dialog = rfd::AsyncFileDialog::new();
        let dialog = match what {
            Pick::Item => dialog
                .add_filter("Media", &["y4m", "mp4", "mov", "m4a", "webm", "mkv", "mka"])
                .add_filter(
                    "Audio",
                    &[
                        "mp3", "mp2", "flac", "aac", "ac3", "wav", "aif", "aiff", "au", "snd",
                        "ogg", "mid", "xm",
                    ],
                ),
            Pick::Subtitles => {
                dialog.add_filter("Subtitles", &["srt", "vtt", "ass", "ssa", "smi", "smil"])
            }
        };
        self.dialog = Some((what, Box::pin(dialog.pick_file())));
    }

    /// Ask for a file to caption the item on screen with. An item that is not
    /// open has nothing to be captioned by, and says so rather than opening a
    /// picker whose answer would be dropped.
    fn pick_subtitles(&mut self) {
        if !self.item_open() {
            self.show_osd("Nothing to caption");
            return;
        }
        self.pick_file(Pick::Subtitles);
    }

    fn poll_dialog(&mut self) {
        let Some((what, dialog)) = &mut self.dialog else {
            return;
        };
        let mut cx = Context::from_waker(Waker::noop());
        if let Poll::Ready(handle) = dialog.as_mut().poll(&mut cx) {
            let what = *what;
            self.dialog = None;
            if let Some(handle) = handle {
                let path = handle.path().to_path_buf();
                match what {
                    Pick::Item => self.open_queue(vec![path]),
                    Pick::Subtitles => self.add_subtitle_file(path),
                }
            }
        }
    }

    fn toggle_pause(&mut self) {
        if self.playback.is_none() {
            self.toggle_sound();
            return;
        }
        let Some(playback) = &self.playback else {
            return;
        };
        if self.ended {
            self.restart();
            return;
        }
        self.paused = !self.paused;
        if self.paused {
            playback.pause();
            if let Some(audio) = &self.audio {
                audio.pause();
            }
        } else {
            playback.play();
            if let Some(audio) = &self.audio {
                audio.play();
            }
            self.deadline = Instant::now();
            if let Some((_, count)) = &mut self.presentation_stats {
                *count = 0;
            }
        }
    }

    /// Pause or resume an item that is only sound. The audio thread is the whole
    /// playback there, so it is the only thing to hold and release.
    fn toggle_sound(&mut self) {
        let Some(audio) = &self.audio else {
            return;
        };
        if self.ended {
            self.restart();
            return;
        }
        self.paused = !self.paused;
        if self.paused {
            audio.pause();
        } else {
            audio.play();
        }
    }

    fn restart(&mut self) {
        match (&mut self.playback, &mut self.audio) {
            (Some(playback), audio) => {
                playback.rewind();
                if let Some(audio) = audio {
                    audio.rewind();
                }
            }
            // With no picture there is no preroll to wait for, so the sound has
            // to be let go here rather than by the frame clock.
            (None, Some(audio)) => {
                audio.rewind();
                audio.play();
            }
            (None, None) => return,
        }
        self.audio_ended = false;
        self.queued = None;
        self.buffering = true;
        self.seek_preview = false;
        self.seek_target = None;
        self.ended = false;
        self.paused = false;
        self.error = None;
        self.deadline = Instant::now();
        if let Some((_, count)) = &mut self.presentation_stats {
            *count = 0;
        }
    }

    /// Jump to `fraction` of the known duration; playback state is kept.
    fn seek_fraction(&mut self, fraction: f32) {
        let Some(total) = self.duration else {
            return;
        };
        let target = total.mul_f32(fraction.clamp(0.0, 1.0));
        self.seek_time(target);
    }

    /// Move the timeline to an absolute position, clamped to the known end.
    /// Playback state is kept, and a track without a known duration can only
    /// move forward.
    fn seek_time(&mut self, target: Duration) {
        let target = match self.duration {
            Some(total) => target.min(total),
            None => target,
        };
        if let Some(playback) = &mut self.playback {
            playback.seek(target);
        } else if self.audio.is_none() {
            return;
        }
        if let Some(audio) = &mut self.audio {
            audio.seek(target);
        }
        self.audio_ended = false;
        // The placeholder position stands in until the decoder hands over the
        // frame at the other end of the seek; an audio thread answers at once.
        if self.playback.is_some() {
            self.seek_target = Some(target);
            self.seek_preview = true;
            self.queued = None;
            self.buffering = true;
        }
        self.ended = false;
        self.error = None;
        self.deadline = Instant::now();
        if let Some((_, count)) = &mut self.presentation_stats {
            *count = 0;
        }
    }

    /// Move the timeline by a VLC jump size, from where the picture stands now.
    fn jump(&mut self, delta: Duration, forward: bool) {
        if !self.seekable {
            return;
        }
        let Some(now) = self.timeline().0 else {
            return;
        };
        let target = if forward {
            now.saturating_add(delta)
        } else {
            now.saturating_sub(delta)
        };
        self.seek_time(target);
        self.show_osd(clock(target));
    }

    /// Walk the chapters of the open file: the picture moves to the start of
    /// the next or previous chapter, and the message names the one arrived at.
    fn walk_chapter(&mut self, forward: bool) {
        if self.chapters.is_empty() {
            self.show_osd("No chapters");
            return;
        }
        if !self.seekable {
            self.show_osd("Cannot seek");
            return;
        }
        let Some(now) = self.timeline().0 else {
            return;
        };
        let starts: Vec<Duration> = self.chapters.iter().map(|mark| mark.start).collect();
        let Some(nth) = chapter_ahead(&starts, now, forward) else {
            self.show_osd(if forward {
                "Last chapter"
            } else {
                "First chapter"
            });
            return;
        };
        let start = self.chapters[nth].start;
        let title = self.chapters[nth].title.clone();
        self.seek_time(start);
        let named = if title.is_empty() {
            String::new()
        } else {
            format!(" {title}")
        };
        self.show_osd(format!(
            "Chapter {}/{}{named}",
            nth + 1,
            self.chapters.len()
        ));
    }

    /// Show exactly one more frame, like VLC's next-frame key. Playback is
    /// paused first; the decoder runs just long enough to hand over the frame.
    fn step_frame(&mut self, back: bool) {
        if self.playback.is_none() {
            if self.item_open() {
                self.show_osd("No picture");
            }
            return;
        }
        if !self.paused && !self.ended {
            self.paused = true;
            if let Some(audio) = &self.audio {
                audio.pause();
            }
        }
        self.ended = false;
        if back {
            let period = self.period;
            self.jump(period, false);
            return;
        }
        if let Some(playback) = &self.playback {
            playback.play();
        }
        self.step = self.step.saturating_add(1);
        self.buffering = true;
    }

    /// Check if a frame is ready for presentation based on A/V sync.
    /// Returns true if the frame can be shown now (audio clock has reached frame PTS).
    fn frame_ready_for_sync(&self, frame: &Frame) -> bool {
        let Some(audio) = &self.audio else {
            return true;
        };
        if self.audio_ended {
            return true;
        }
        let Some((pts, timescale)) = frame.pts else {
            return true;
        };
        if timescale == 0 {
            return true;
        }
        let audio_pos = delayed_clock(audio.position(), self.audio_delay_ms);
        let frame_seconds = pts as f64 / timescale as f64;
        let audio_seconds = audio_pos.as_secs_f64();
        audio_seconds >= frame_seconds - AUDIO_SYNC_SLACK
    }

    /// Drain audio-thread events. Audio that has run out or failed must stop
    /// gating presentation, otherwise the picture freezes at the last clock.
    fn poll_audio(&mut self) {
        let Some(audio) = &self.audio else {
            return;
        };
        let generation = audio.generation();
        let mut ran_out = false;
        let mut failure = None;
        while let Some(event) = audio.poll() {
            match event {
                crate::audio_thread::AudioEvent::Started => {}
                crate::audio_thread::AudioEvent::Ended(at) => {
                    if at >= generation {
                        self.audio_ended = true;
                        ran_out = true;
                    }
                }
                crate::audio_thread::AudioEvent::Error(error) => {
                    self.audio_ended = true;
                    eprintln!("audio: {error}");
                    failure = Some(error);
                }
            }
        }
        // For an item that is only sound the track running out is the item
        // running out, so the queue carries on as it does after a last frame.
        if (ran_out || failure.is_some()) && self.playback.is_none() && !self.ended {
            self.stop_time = None;
            self.continue_queue();
        }
    }

    /// Follow an item that is only sound. Its clock runs on the audio thread, so
    /// the screen has nothing to time: it keeps up with the position, honours the
    /// A–B region and the `--stop-time`, and asks for another look.
    fn tick_sound(&mut self, ctx: &egui::Context) {
        if !self.item_open() || self.paused || self.ended {
            return;
        }
        if let Some(target) = loop_rewind(
            self.timeline().0.unwrap_or_default(),
            self.loop_a,
            self.loop_b,
        ) {
            self.seek_time(target);
        }
        self.check_stop();
        ctx.request_repaint();
    }

    /// Take decoded frames from the thread and show the one whose time has come.
    fn present(&mut self, ctx: &egui::Context) {
        self.poll_audio();
        if self.playback.is_none() {
            self.tick_sound(ctx);
            return;
        }
        let stepping = self.step > 0;
        if self.paused && !self.seek_preview && !stepping {
            return;
        }
        let Some(playback) = &self.playback else {
            return;
        };
        let generation = playback.generation();
        let mut ran_out = false;
        while self.queued.is_none() {
            match playback.poll() {
                Some(Event::Frame(frame)) if frame.generation < generation => continue,
                Some(Event::Frame(frame)) => self.queued = Some(frame),
                Some(Event::Ended(at)) => {
                    if at >= generation {
                        ran_out = true;
                    }
                }
                Some(Event::Error(error)) => {
                    self.error = Some(error);
                    self.seek_target = None;
                    self.ended = true;
                }
                None => break,
            }
        }
        // A queue carries on by itself; the last picture of the last item is
        // where a single file stops.
        if ran_out && !stepping {
            self.continue_queue();
        }
        let now = Instant::now();
        if self.buffering
            && let Some(frame) = &self.queued
        {
            self.deadline = if self.paused || stepping {
                now
            } else if self.seek_preview {
                now + frame.period.min(Duration::from_millis(33))
            } else {
                now + crate::playback_thread::startup_buffer(frame.period)
            };
            self.buffering = false;
            // Sound and picture have to start from the same instant: the audio
            // thread was held paused across the preroll, so this is the first
            // moment its clock can run. Starting it any earlier leaves the
            // picture behind by the preroll for the whole file, and the sync
            // gate below can only ever hold video back, never hand it time.
            if !self.paused
                && let Some(audio) = &self.audio
            {
                audio.play();
            }
        }
        if self.queued.is_some() {
            // Clock and vsync jitter must not postpone a ready frame by an
            // entire refresh. The media clock still advances by the exact PTS interval.
            let tolerance = (self.period / 8).min(Duration::from_millis(1));
            let frame = self.queued.as_ref().unwrap();
            let time_ready = stepping || now + tolerance >= self.deadline;
            let av_ready = self.frame_ready_for_sync(frame);
            if time_ready && av_ready {
                let frame = self.queued.take().unwrap();
                self.seek_preview = false;
                self.seek_target = None;
                match &frame.pixels {
                    Pixels::Planar(planes) => {
                        self.video = Some((planes.clone(), frame.serial));
                        self.texture = None;
                        self.rgb_frame = None;
                    }
                    Pixels::Rgb(rgb) => {
                        // The bundle is applied on the way in, and the frame is
                        // kept as the reader wrote it so a slider moved over it
                        // can rebuild what stands on screen.
                        let scalars = adjust_scalars(&self.adjust);
                        let mut adjusted = Vec::new();
                        let pixels = if scalars == ADJUST_IDENTITY {
                            rgb.as_slice()
                        } else {
                            adjust_rgb(rgb, &mut adjusted, &scalars);
                            &adjusted
                        };
                        let image = egui::ColorImage::from_rgb(frame.dimensions, pixels);
                        if let Some(texture) = &mut self.texture {
                            texture.set(image, egui::TextureOptions::LINEAR);
                        } else {
                            self.texture = Some(ctx.load_texture(
                                "video",
                                image,
                                egui::TextureOptions::LINEAR,
                            ));
                        }
                        self.rgb_frame = Some((rgb.clone(), frame.dimensions, scalars));
                        self.video = None;
                    }
                }
                if let Some((start, count)) = &mut self.presentation_stats {
                    if *count == 0 {
                        *start = Instant::now();
                        self.starved_polls = 0;
                    }
                    *count += 1;
                    if *count == 301 {
                        eprintln!(
                            "player presentation: 300 intervals in {:?}, {:.2} fps, visible={:?}, focused={:?}, starved_polls={}",
                            start.elapsed(),
                            300.0 / start.elapsed().as_secs_f64(),
                            ctx.input(|i| i.viewport().visible()),
                            ctx.input(|i| i.viewport().focused),
                            self.starved_polls
                        );
                        self.starved_polls = 0;
                        *count = 1;
                        *start = Instant::now();
                    }
                }
                self.dimensions = frame.dimensions;
                self.period = frame.period;
                self.interval = frame.interval;
                // Keep the cadence while the decoder keeps up; when it falls
                // behind, or a single frame was asked for, show what arrived
                // instead of piling up debt.
                let period = paced_period(frame.period, self.rate_milli);
                self.presented = Some(frame);
                self.deadline = if stepping || self.deadline + period < now {
                    now
                } else {
                    self.deadline + period
                };
                if stepping {
                    self.step -= 1;
                    if self.step == 0 {
                        // A single-frame step parks the decoder again as soon as
                        // the picture is up, so pause does not keep burning CPU.
                        if let Some(playback) = &self.playback {
                            playback.pause();
                        }
                    }
                }
                // A closed A–B region sends the picture back to A once it has
                // shown B. Stepping by hand is an explicit walk, so it does not
                // trigger the loop.
                if !stepping
                    && let Some(target) = loop_rewind(
                        self.timeline().0.unwrap_or_default(),
                        self.loop_a,
                        self.loop_b,
                    )
                {
                    self.seek_time(target);
                }
                if !stepping {
                    self.check_stop();
                }
                ctx.request_repaint_after(self.deadline.saturating_duration_since(Instant::now()));
            } else {
                ctx.request_repaint_after(self.deadline.saturating_duration_since(now));
            }
        } else if (!self.paused || self.seek_preview || stepping) && !self.ended {
            if now >= self.deadline && self.presentation_stats.is_some() {
                self.starved_polls = self.starved_polls.saturating_add(1);
            }
            // Waiting on the decoder: check again soon.
            ctx.request_repaint_after(Duration::from_millis(8));
        }
        if !self.paused && !self.ended {
            // Let vsync pace rendering; a timer at the presentation deadline
            // can wake too late to submit that frame for the next refresh.
            ctx.request_repaint();
        }
    }

    /// Elapsed time at the end of the frame on screen, and the total when known.
    fn timeline(&self) -> (Option<Duration>, Option<Duration>) {
        if !self.item_open() {
            return (None, None);
        }
        if let Some(target) = self.seek_target {
            return (Some(target), self.duration);
        }
        if self.playback.is_none() {
            // With no picture the audio thread's own clock is the timeline.
            return (
                self.audio.as_ref().map(|audio| audio.position()),
                self.duration,
            );
        }
        let elapsed = self
            .interval
            .filter(|(_, _, scale)| *scale > 0)
            .map(|(_, end, scale)| {
                let nanos = end * 1_000_000_000 / u128::from(scale);
                Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
            });
        (elapsed, self.duration)
    }

    fn subtitle(&self) -> String {
        if !self.item_open() {
            return String::new();
        }
        let head = if self.playback.is_none() {
            // Nothing to give the eye but the track the ear is getting.
            self.audio_tracks
                .get(self.audio_track)
                .map(crate::audio::AudioTrack::label)
                .unwrap_or_else(|| "sound".to_owned())
        } else {
            let [w, h] = self.dimensions;
            let decoder = if self.hardware {
                " · VideoToolbox"
            } else {
                ""
            };
            format!("{w}×{h} · {} fps{decoder}", fps_text(self.period))
        };
        let rate = if self.rate_milli == 1_000 {
            String::new()
        } else {
            format!(" · {}", rate_osd(self.rate_milli))
        };
        let level = if self.muted {
            " · muted".to_owned()
        } else if self.volume_milli != 1_000 {
            format!(" · {}%", self.volume_milli / 10)
        } else {
            String::new()
        };
        // Where this item sits in the queue, once there is more than one.
        let item = if self.queue.len() > 1 {
            format!(" · {}", playlist_osd(self.index, self.queue.len()))
        } else {
            String::new()
        };
        let sound = if self.audio_delay_ms == 0 {
            String::new()
        } else {
            format!(" · delay {} ms", self.audio_delay_ms)
        };
        format!("{head}{rate}{level}{item}{sound}")
    }

    /// What the item on screen is made of, one line for each kind of thing it
    /// carries: the picture and the codec it decodes from, the tracks the sound
    /// and the cues are taken from, how much file the timeline fills, and how
    /// many parts the container names. Every value is one the player already
    /// keeps, so opening the panel reads nothing from the file.
    fn info_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (label, value) in [
            ("Name", &self.file_tags.title),
            ("Artist", &self.file_tags.artist),
            ("Album artist", &self.file_tags.album_artist),
            ("Album", &self.file_tags.album),
            ("Track", &self.file_tags.track),
            ("Disc", &self.file_tags.disc),
            ("Genre", &self.file_tags.genre),
            ("Publisher", &self.file_tags.publisher),
            ("Date", &self.file_tags.date),
            ("Comment", &self.file_tags.comment),
            ("Copyright", &self.file_tags.copyright),
            ("Description", &self.file_tags.description),
            ("Rating", &self.file_tags.rating),
        ] {
            if !value.is_empty() {
                lines.push(format!("{label}: {value}"));
            }
        }
        if self.playback.is_some() {
            let [w, h] = self.dimensions;
            let pixels = if self.pixel_aspect == (1, 1) {
                String::new()
            } else {
                let (x, y) = self.pixel_aspect;
                format!(" · pixels {x}:{y}")
            };
            let decoder = if self.hardware {
                " · VideoToolbox"
            } else {
                ""
            };
            lines.push(format!(
                "Video: {} · {w}×{h} · {} fps{pixels}{decoder}",
                self.video_codec,
                fps_text(self.period)
            ));
            if let Some(line) = colour_line(self.signal) {
                lines.push(line);
            }
            if let Some(line) = light_line(&self.hdr) {
                lines.push(line);
            }
            if let Some(line) = &self.grade_line {
                lines.push(line.clone());
            }
        }
        if let Some(track) = self.audio_tracks.get(self.audio_track) {
            lines.push(format!(
                "Sound: {} · {} · {}",
                playlist_osd(self.audio_track, self.audio_tracks.len()),
                self.sound_codec,
                track.label()
            ));
        }
        if let Some(source) = self.subtitle_sources.get(self.subtitle_source) {
            lines.push(format!(
                "Subtitles: {} · {}",
                playlist_osd(self.subtitle_source, self.subtitle_sources.len()),
                source.label
            ));
        }
        if let Some(bytes) = self.bytes {
            let mut line = format!("File: {}", byte_size(bytes));
            if let Some(total) = self.duration {
                line += &format!(" · {}", clock(total));
                if let Some(rate) = bitrate_text(bytes, total) {
                    line += &format!(" · {rate}");
                }
            }
            lines.push(line);
        }
        if !self.chapters.is_empty() {
            lines.push(format!("Chapters: {}", self.chapters.len()));
        }
        lines
    }

    fn controls_visible(&self) -> bool {
        !self.item_open()
            || self.info
            || self.effects
            || self.paused
            || self.ended
            || self.error.is_some()
            || self.activity.elapsed() < HIDE_AFTER
    }
}

fn clock(time: Duration) -> String {
    let total = time.as_secs();
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// How many bytes a path takes up, once the file system has answered for it.
fn file_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|meta| meta.len())
}

/// How many frames a second of the picture lasts for, written whole whenever it
/// comes within a twentieth of a frame of one: 41.7 ms per frame reads as `24`,
/// not `24.00`, and only a rate standing further off keeps its decimals.
fn fps_text(period: Duration) -> String {
    let seconds = period.as_secs_f64();
    if seconds <= 0.0 {
        return "0".to_owned();
    }
    let fps = 1.0 / seconds;
    if (fps - fps.round()).abs() < 0.05 {
        format!("{}", fps.round() as u32)
    } else {
        format!("{fps:.2}")
    }
}

/// The sound of the open item, named the way a viewer names it. Every
/// container files the same coding under its own tag — AAC is `mp4a` in MP4,
/// PCM is `sowt` in QuickTime and `A_PCM/INT/LIT` in Matroska, and Microsoft's
/// ADPCM reaches this build under the decoder's dispatch names — so one word
/// per coding needs the table the picture's `mp4_codec` is for it. A coding
/// none of the readers can produce arrives under the name its own container
/// gave it, because some name on the panel beats a blank one.
fn sound_codec(tag: &str) -> String {
    match tag {
        "A_VORBIS" => "Vorbis",
        "A_MPEG/L3" => "MP3",
        "A_MPEG/L2" => "MP2",
        "A_FLAC" => "FLAC",
        "A_ALAC" | "alac" => "ALAC",
        "A_AC3" | "ac-3" => "Dolby Digital",
        "mp4a" => "AAC",
        "A_PCM/INT/LIT" | "A_PCM/INT/BIG" | "A_PCM/FLOAT/IEEE" | "sowt" | "twos" | "fl32"
        | "fl64" => "PCM",
        "pcm_alaw" => "G.711 (a-law)",
        "pcm_mulaw" => "G.711 (mu-law)",
        "adpcm_ms" => "ADPCM (MS)",
        "adpcm_ima_wav" | "adpcm_ima_qt" => "ADPCM (IMA)",
        "midi" => "MIDI",
        "xm" => "XM",
        _ => return tag.to_owned(),
    }
    .to_owned()
}

/// A file's size in the units an information panel counts in: the largest one
/// that still leaves a whole number, with a decimal place only while the number
/// has a single digit, so that 9 MB and 9.4 MB do not read as one size. Bytes
/// are grouped by a thousand, the way the system names a disk's capacity.
fn byte_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes} B");
    }
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// The rate the whole item adds up to: every byte of it counted once and spread
/// over its timeline, which is what a container's length and its size state
/// together. `None` without a length to spread them over, since a rate of one of
/// the two alone would be a guess.
fn bitrate_text(bytes: u64, duration: Duration) -> Option<String> {
    let micros = duration.as_micros();
    if micros == 0 {
        return None;
    }
    let bits = u128::from(bytes) * 8 * 1_000_000 / micros;
    Some(if bits >= 1_000_000 {
        format!("{:.1} Mb/s", bits as f64 / 1_000_000.0)
    } else {
        format!("{:.0} kb/s", bits as f64 / 1_000.0)
    })
}

/// A round hit area with an icon painted by `draw`; returns whether it was clicked.
fn round_button(
    ui: &mut egui::Ui,
    id: &str,
    center: Pos2,
    size: f32,
    background: Color32,
    draw: impl FnOnce(&egui::Painter, Pos2, Color32),
) -> bool {
    let rect = Rect::from_center_size(center, Vec2::splat(size));
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    let painter = ui.painter();
    let fill = if response.hovered() {
        CHIP_STRONG
    } else {
        background
    };
    if fill != Color32::TRANSPARENT {
        painter.circle_filled(center, size / 2.0, fill);
    }
    draw(painter, center, TEXT);
    response.clicked()
}

/// A crotched note: the stage of an item that is only sound.
fn icon_note(painter: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    let head = s * 0.16;
    let stem = c.x + s * 0.22;
    painter.circle_filled(Pos2::new(stem - head * 1.6, c.y + s * 0.38), head, color);
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(stem, c.y - s * 0.42),
            Pos2::new(stem + s * 0.06, c.y + s * 0.38),
        ),
        CornerRadius::ZERO,
        color,
    );
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(stem + s * 0.06, c.y - s * 0.42),
            Pos2::new(stem + s * 0.3, c.y - s * 0.42 + s * 0.12),
        ),
        CornerRadius::ZERO,
        color,
    );
}

fn icon_play(painter: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    let points = vec![
        Pos2::new(c.x - s * 0.36, c.y - s * 0.5),
        Pos2::new(c.x + s * 0.5, c.y),
        Pos2::new(c.x - s * 0.36, c.y + s * 0.5),
    ];
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

fn icon_pause(painter: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    let bar = Vec2::new(s * 0.3, s);
    for dx in [-s * 0.3, s * 0.3] {
        painter.rect_filled(
            Rect::from_center_size(Pos2::new(c.x + dx, c.y), bar),
            CornerRadius::same(2),
            color,
        );
    }
}

fn icon_restart(painter: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    let stroke = Stroke::new(2.0, color);
    let left = c.x - s * 0.45;
    painter.line_segment(
        [
            Pos2::new(left, c.y - s * 0.5),
            Pos2::new(left, c.y + s * 0.5),
        ],
        stroke,
    );
    let points = vec![
        Pos2::new(c.x + s * 0.5, c.y - s * 0.5),
        Pos2::new(c.x + s * 0.5, c.y + s * 0.5),
        Pos2::new(left + 4.0, c.y),
    ];
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

fn icon_fullscreen(painter: &egui::Painter, c: Pos2, s: f32, color: Color32, exit: bool) {
    let stroke = Stroke::new(2.0, color);
    let h = s / 2.0;
    let arm = s * 0.3;
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let corner = Pos2::new(c.x + sx * h, c.y + sy * h);
        let (a, b) = if exit {
            let inner = Pos2::new(c.x + sx * (h - arm), c.y + sy * (h - arm));
            (
                [inner, Pos2::new(inner.x, corner.y)],
                [inner, Pos2::new(corner.x, inner.y)],
            )
        } else {
            (
                [corner, Pos2::new(corner.x, corner.y - sy * arm)],
                [corner, Pos2::new(corner.x - sx * arm, corner.y)],
            )
        };
        painter.line_segment(a, stroke);
        painter.line_segment(b, stroke);
    }
}

fn icon_close(painter: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    let stroke = Stroke::new(2.0, color);
    let h = s / 2.0;
    painter.line_segment(
        [Pos2::new(c.x - h, c.y - h), Pos2::new(c.x + h, c.y + h)],
        stroke,
    );
    painter.line_segment(
        [Pos2::new(c.x - h, c.y + h), Pos2::new(c.x + h, c.y - h)],
        stroke,
    );
}

/// A control the keyboard asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Control {
    /// Up or down one volume step.
    Volume(bool),
    Mute,
    /// Next or previous VLC rate preset, or an absolute rate to return to.
    Rate(bool),
    /// VLC's `,` and `.`: nudge the rate by a tenth of its own name, inside
    /// the bounds the preset ladder reaches.
    RateFine(bool),
    Reset,
    /// A VLC jump size, forward or back.
    Jump(Duration, bool),
    Position(f32),
    /// One frame forward, or back to the frame before the one on screen.
    Frame(bool),
    Subtitles,
    /// Show the cues later (true) or earlier (false).
    SubtitleDelay(bool),
    /// Move the cue block up (true) or down (false).
    SubtitleMargin(bool),
    SubtitleSize(bool),
    /// Next or previous subtitle source of the open file.
    SubtitleTrack(bool),
    /// Next or previous item of the open list.
    Next,
    Previous,
    /// Cycle the end-of-item behaviour.
    Repeat,
    /// Turn VLC's Random on or off: the list walks in a drawn order.
    Shuffle,
    /// Press the A–B loop key: mark A, then B, then clear both.
    Loop,
    /// Let sound arrive later (true) or earlier (false).
    AudioDelay(bool),
    /// Next or previous audio track of the open file.
    AudioTrack(bool),
    /// Write the picture on screen out as a PNG file.
    Snapshot,
    /// Next (true) or previous chapter of the open file.
    Chapter(bool),
    /// Up or down one rung of VLC's Zoom menu.
    Zoom(bool),
    /// Forward or back along VLC's Crop menu.
    Crop(bool),
    /// Forward or back along VLC's aspect-ratio menu.
    Aspect(bool),
    /// Open or close the panel of what the open item is.
    Info,
    /// Name a file to caption the open item with, which VLC loads from its
    /// Media menu.
    LoadSubtitles,
    /// Open or close VLC's Ctrl+E dialog of picture settings.
    Effects,
}

/// What happens when the picture reaches the end of the list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Repeat {
    /// Stop on the last item.
    #[default]
    Off,
    /// Wrap around to the first item.
    All,
    /// Start the current item again.
    One,
}

/// The item to start once the current one runs out, or `None` to stop.
fn advance(index: usize, len: usize, repeat: Repeat) -> Option<usize> {
    if len == 0 {
        return None;
    }
    if index + 1 < len {
        return Some(index + 1);
    }
    (repeat == Repeat::All).then_some(0)
}

/// The item before the current one, wrapping only when the list repeats.
fn retreat(index: usize, len: usize, repeat: Repeat) -> Option<usize> {
    if len == 0 {
        return None;
    }
    if index > 0 {
        return Some(index - 1);
    }
    (repeat == Repeat::All).then_some(len - 1)
}

/// The next audio track to open, wrapping within the tracks a decoder exists
/// for. The caller only asks when there is more than one.
fn track_step(current: usize, len: usize, forward: bool) -> usize {
    (current + if forward { 1 } else { len - 1 }) % len
}

/// One draw of a splitmix64 generator: the value and the state advanced past
/// it. Cheap, and random enough for an order nobody can bet on.
fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Deal one shuffled cycle of the list: every index but the one on screen, in
/// an order drawn by a Fisher-Yates walk over the draws of `seed`, which is
/// left advanced for the next dealing. The last item of the list is the next
/// draw, so a walk over the cycle plays each of them once before the list
/// ends or deals again.
fn deal_cycle(len: usize, current: usize, seed: &mut u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).filter(|&i| i != current).collect();
    for i in 1..order.len() {
        let draw = (splitmix(seed) >> 32) as usize;
        order.swap(i, draw % (i + 1));
    }
    order
}

fn cycle_repeat(repeat: Repeat) -> Repeat {
    match repeat {
        Repeat::Off => Repeat::All,
        Repeat::All => Repeat::One,
        Repeat::One => Repeat::Off,
    }
}

fn repeat_osd(repeat: Repeat) -> &'static str {
    match repeat {
        Repeat::Off => "Repeat off",
        Repeat::All => "Repeat all",
        Repeat::One => "Repeat one",
    }
}

/// Position in the list, one-based, as VLC writes it.
fn playlist_osd(index: usize, len: usize) -> String {
    format!("{}/{len}", index + 1)
}

/// Expand playlist files into the videos they list; anything else is already a
/// path, including one that does not exist, which the caller reports on open. A
/// list with nothing usable in it is handed back untouched so opening it says
/// why rather than leaving the window blank.
fn expand_inputs(inputs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::with_capacity(inputs.len());
    for input in inputs {
        let listing = matches!(
            input
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .as_deref(),
            Some("m3u") | Some("pls")
        );
        match listing {
            true => {
                if let Ok(text) = std::fs::read_to_string(input) {
                    let base = input.parent().unwrap_or(Path::new(""));
                    out.extend(parse_playlist(&text, base));
                }
            }
            false => out.push(input.clone()),
        }
    }
    if out.is_empty() && !inputs.is_empty() {
        return inputs.to_vec();
    }
    out
}

/// Read the video paths out of an M3U (`#EXTM3U`, `#EXTINF:` notes, then one
/// path per line) or a PLS (`FileN=path`). Entries are resolved against the
/// list's own directory, and ones that are not on disk are dropped rather than
/// failing the whole list. Network entries are not something the native player
/// can open, so they are skipped too.
fn parse_playlist(text: &str, base: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let entry = match line.split_once('=') {
            Some((key, value)) => {
                if key
                    .trim()
                    .trim_end_matches(char::is_numeric)
                    .eq_ignore_ascii_case("file")
                {
                    value
                } else {
                    continue;
                }
            }
            None => {
                if line.is_empty() || line.starts_with(['#', ';']) {
                    continue;
                }
                line
            }
        };
        let entry = entry.trim();
        if entry.is_empty() || entry.contains("://") {
            continue;
        }
        let path = if Path::new(entry).is_absolute() {
            PathBuf::from(entry)
        } else {
            base.join(entry)
        };
        if path.is_file() {
            out.push(path);
        }
    }
    out
}

/// VLC's jump ladder: very short, short, medium, long.
const JUMP_VERY_SHORT: Duration = Duration::from_secs(3);
const JUMP_SHORT: Duration = Duration::from_secs(10);
const JUMP_MEDIUM: Duration = Duration::from_secs(60);
const JUMP_LONG: Duration = Duration::from_secs(300);

/// The size of a plain arrow jump for these modifiers. Control belongs to the
/// volume here, so it is handled by the caller before this is consulted.
fn jump_size(shift: bool, alt: bool) -> Duration {
    if shift {
        JUMP_SHORT
    } else if alt {
        JUMP_MEDIUM
    } else {
        JUMP_VERY_SHORT
    }
}

/// Shift the cue timeline by one delay step, bounded at ten seconds either
/// way as VLC bounds it.
fn delay_step(current: i64, later: bool) -> i64 {
    let step = i64::try_from(subtitles::DELAY_STEP.as_millis()).unwrap_or(50);
    (current + if later { step } else { -step }).clamp(-10_000, 10_000)
}

/// What the A–B key does with the position it was pressed at.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LoopMark {
    /// First press: A is now, and the region is not closed yet.
    MarkedA(Duration),
    /// Second press: B is now, so the picture will return to A once it gets here.
    MarkedB(Duration),
    /// Third press: both marks go away and playback carries on normally.
    Cleared,
}

/// VLC's single loop key presses A, then B, then clears the pair. A mark behind
/// the one already set would loop backwards, so it starts a new region instead.
fn loop_press(now: Duration, a: Option<Duration>, b: Option<Duration>) -> LoopMark {
    match b {
        Some(_) => LoopMark::Cleared,
        None => match a {
            // A mark before the one already set cannot close the region, so it
            // starts a new one instead of looping backwards.
            Some(start) if now > start => LoopMark::MarkedB(now),
            _ => LoopMark::MarkedA(now),
        },
    }
}

/// Where to rewind after showing a frame, if the open A–B region has run out.
fn loop_rewind(now: Duration, a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) if now >= b => Some(a),
        _ => None,
    }
}

fn loop_osd(mark: LoopMark) -> String {
    match mark {
        LoopMark::MarkedA(at) => format!("Loop A {}", clock(at)),
        LoopMark::MarkedB(at) => format!("Loop A to B {}", clock(at)),
        LoopMark::Cleared => "Loop cleared".to_owned(),
    }
}

/// The audio clock as the picture sees it. A positive delay lets sound arrive
/// later, so the gate treats audio as being that much further ahead.
fn delayed_clock(position: Duration, delay_ms: i64) -> Duration {
    let shift = Duration::from_millis(delay_ms.unsigned_abs());
    if delay_ms >= 0 {
        position.saturating_add(shift)
    } else {
        position.saturating_sub(shift)
    }
}

/// Where a snapshot of `video` at a media time is written: beside the file,
/// under its own name, marked to the millisecond.
fn snapshot_name(video: &Path, at: Duration) -> PathBuf {
    let total = at.as_millis();
    let (hours, minutes, seconds, millis) = (
        total / 3_600_000,
        total / 60_000 % 60,
        total / 1_000 % 60,
        total % 1_000,
    );
    let stem = video
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    video.parent().unwrap_or(Path::new(".")).join(format!(
        "{stem}-{hours:02}h{minutes:02}m{seconds:02}s{millis:03}.png"
    ))
}

/// The plain-text subtitle tracks inside a file, in container order, read by
/// whichever container reader its first bytes belong to: Matroska says so with
/// its EBML magic, and everything else this player opens that can hold a text
/// track is MP4 or QuickTime. A Y4M stream, or a file either reader refuses,
/// offers none.
fn embedded_subtitles(path: &Path) -> Vec<SubtitleSource> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let mut buffered = BufReader::new(file);
    let mut magic = [0u8; 4];
    if buffered.read_exact(&mut magic).is_err() || buffered.rewind().is_err() {
        return Vec::new();
    }
    if magic == *b"\x1a\x45\xdf\xa3" {
        let Ok(mut reader) = crate::playback_webm_subtitles::WebmSubtitleReader::open(
            buffered,
            crate::container::webm::Limits::default(),
        ) else {
            return Vec::new();
        };
        let labels: Vec<String> = reader
            .tracks()
            .iter()
            .map(crate::playback_webm_subtitles::SubtitleTrack::label)
            .collect();
        return listed(&labels, &mut |nth| reader.cues(nth));
    }
    let Ok(mut reader) = crate::playback_mp4_subtitles::Mp4SubtitleReader::open(
        buffered,
        crate::container::mp4::Limits::default(),
    ) else {
        return Vec::new();
    };
    let labels: Vec<String> = reader
        .tracks()
        .iter()
        .map(crate::playback_mp4_subtitles::Mp4SubtitleTrack::label)
        .collect();
    listed(&labels, &mut |nth| reader.cues(nth))
}

/// What a container says about the file as a whole besides its tracks: the
/// chapter points it names and the facts its own tags state about it. Read by
/// whichever reader the file's first bytes belong to, as `embedded_subtitles`
/// picks its reader. This costs the second index pass a container reader always
/// makes, which is what reading the file's own tracks costs anyway, and it is
/// what both kinds of item pay alike — a file with nothing to show is named here
/// too. A stream carrying neither, or one the reader cannot open, leaves both
/// empty.
fn container_facts(path: &Path) -> ContainerFacts {
    let Ok(file) = File::open(path) else {
        return ContainerFacts::default();
    };
    let mut buffered = BufReader::new(file);
    let mut magic = [0u8; 4];
    if buffered.read_exact(&mut magic).is_err() || buffered.rewind().is_err() {
        return ContainerFacts::default();
    }
    let (runs, tags): (Vec<(u64, String)>, FileTags) = if magic == *b"\x1a\x45\xdf\xa3" {
        crate::container::webm::WebmReader::open(
            buffered,
            crate::container::webm::Limits::default(),
        )
        .map(|reader| {
            (
                reader
                    .chapters
                    .iter()
                    .map(|chapter| (chapter.start_ns, chapter.title.clone()))
                    .collect(),
                reader.tags,
            )
        })
        .unwrap_or_default()
    } else {
        crate::container::mp4::Mp4Reader::open(buffered, crate::container::mp4::Limits::default())
            .map(|reader| {
                (
                    reader
                        .chapters()
                        .iter()
                        .map(|chapter| (chapter.start_ns, chapter.title.clone()))
                        .collect(),
                    reader.tags().clone(),
                )
            })
            .unwrap_or_default()
    };
    ContainerFacts {
        chapters: runs
            .into_iter()
            .map(|(start_ns, title)| ChapterMark {
                start: Duration::from_nanos(start_ns),
                title,
            })
            .collect(),
        tags,
    }
}

/// The two answers a container gives about the file itself, gathered by the one
/// walk `container_facts` makes.
#[derive(Default)]
struct ContainerFacts {
    chapters: Vec<ChapterMark>,
    tags: FileTags,
}

/// Which chapter the next or previous key answers with: the first one starting
/// after the position, or the last one starting before it. Neither end of the
/// film has anywhere further to walk.
fn chapter_ahead(starts: &[Duration], at: Duration, forward: bool) -> Option<usize> {
    if forward {
        (0..starts.len()).find(|&nth| starts[nth] > at)
    } else {
        (0..starts.len()).rev().find(|&nth| starts[nth] < at)
    }
}

/// One source per track whose cues are there to show: a track that yields
/// nothing is not a choice worth cycling through, and a track that cannot be
/// read leaves its neighbours alone.
fn listed(
    labels: &[String],
    cues: &mut dyn FnMut(usize) -> crate::Result<Vec<Cue>>,
) -> Vec<SubtitleSource> {
    let mut sources = Vec::with_capacity(labels.len());
    for (nth, label) in labels.iter().enumerate() {
        if let Ok(cues) = cues(nth)
            && !cues.is_empty()
        {
            sources.push(SubtitleSource {
                label: label.clone(),
                cues,
            });
        }
    }
    sources
}

/// Keys the player answers, mapped onto VLC's defaults: arrows jump, with
/// Shift, Alt and Control taking the longer ladder rungs; Control or Command
/// with up and down moves the volume; `m` mutes; `=` and `-` step through the
/// rate presets, `,` and `.` nudge the rate by a tenth between them; `\`
/// returns to normal speed; `e` advances one frame, or one
/// back with Shift; Home and End go to either end of the timeline; and a number
/// key jumps to that tenth of the file, and `n`, `p` and `r` walk the queue and
/// cycle what happens when an item ends, with `s` deciding whether that walk
/// follows a drawn order. `l` marks the A-B loop points, `j` and
/// `k` shift the audio clock, `a` walks the file's audio tracks, `b` its
/// subtitle tracks, and Shift+S writes the picture on screen out as a PNG.
/// `[` and `]` jump between the chapters the file names, and `z` and `Z` step
/// the picture along VLC's Zoom menu; a picture a magnification has grown past
/// the frame can then be dragged to the part of it the frame cannot show. `i`
/// opens the panel of what the item on screen is, and closes it again.
fn controls_pressed(ctx: &egui::Context) -> Vec<Control> {
    use egui::Key;
    let mut out = Vec::new();
    ctx.input(|input| {
        let key = |k: Key| input.key_pressed(k);
        let modifiers = input.modifiers;
        let held = modifiers.command || modifiers.ctrl;

        if held {
            match (key(Key::ArrowUp), key(Key::ArrowDown)) {
                (true, _) => out.push(Control::Volume(true)),
                (_, true) => out.push(Control::Volume(false)),
                _ => {}
            }
        }
        let back = key(Key::ArrowLeft);
        let forward = key(Key::ArrowRight);
        if (back || forward) && !modifiers.alt {
            if held && !modifiers.shift {
                out.push(Control::Jump(JUMP_LONG, forward));
            } else {
                out.push(Control::Jump(
                    jump_size(modifiers.shift, modifiers.alt),
                    forward,
                ));
            }
        }
        if key(Key::M) {
            out.push(Control::Mute);
        }
        if key(Key::Equals) || key(Key::Plus) {
            out.push(Control::Rate(true));
        }
        if key(Key::Minus) {
            out.push(Control::Rate(false));
        }
        // VLC's fine pair: `,` and `.` nudge the rate by a tenth, completing
        // the preset ladder of `=` and `-`.
        if key(Key::Comma) {
            out.push(Control::RateFine(false));
        }
        if key(Key::Period) {
            out.push(Control::RateFine(true));
        }
        if key(Key::Backslash) {
            out.push(Control::Reset);
        }
        // VLC's Zoom menu, which it gives no key of its own: `z` steps up the
        // four rungs it lists, `Z` back down.
        if key(Key::Z) {
            out.push(Control::Zoom(!modifiers.shift));
        }
        // The shapes VLC lists for cropping, which it cycles through its `crop`
        // variable and fvid puts on one key: `c` walks the list, `C` back, and
        // the walk returns to the whole picture.
        if key(Key::C) {
            out.push(Control::Crop(!modifiers.shift));
        }
        // VLC's aspect-ratio menu, which it cycles through its `aspect-ratio`
        // variable and fvid puts on one key: `v` walks the ten shapes, `V`
        // back, and the walk returns to the shape the file declares. The older
        // player gives this menu `A`, which here lists the audio tracks.
        if key(Key::V) {
            out.push(Control::Aspect(!modifiers.shift));
        }
        // What the item is, which VLC opens as a window of its own on Ctrl+I and
        // fvid puts over the picture on `i`.
        if key(Key::I) {
            out.push(Control::Info);
        }
        // `[` and `]` walk the chapters of the open file.
        if key(Key::OpenBracket) {
            out.push(Control::Chapter(false));
        }
        if key(Key::CloseBracket) {
            out.push(Control::Chapter(true));
        }
        if key(Key::Home) {
            out.push(Control::Position(0.0));
        }
        if key(Key::End) {
            out.push(Control::Position(1.0));
        }
        if key(Key::E) {
            out.push(if held {
                Control::Effects
            } else {
                Control::Frame(!modifiers.shift)
            });
        }
        // `t` shows or hides the cues; with the command key held it is VLC's
        // "Load Subtitle File", which asks for a file to caption the item with.
        if key(Key::T) {
            out.push(if held {
                Control::LoadSubtitles
            } else {
                Control::Subtitles
            });
        }
        // `b` walks the subtitle sources of the open file, Shift back.
        if key(Key::B) {
            out.push(Control::SubtitleTrack(!modifiers.shift));
        }
        // VLC's playlist keys: `n` and `p` step through the queue, `r` cycles
        // what happens at the end of an item.
        if key(Key::N) {
            out.push(Control::Next);
        }
        if key(Key::P) {
            out.push(Control::Previous);
        }
        if key(Key::R) {
            out.push(Control::Repeat);
        }
        // VLC's `s` is Random: it joins the `n`, `p` and `r` trio by deciding
        // in what order the queue walks, not where it stops.
        if !modifiers.shift && key(Key::S) {
            out.push(Control::Shuffle);
        }
        // VLC's audio-delay pair: `k` lets sound arrive later, `j` earlier.
        if key(Key::J) {
            out.push(Control::AudioDelay(false));
        }
        if key(Key::K) {
            out.push(Control::AudioDelay(true));
        }
        // One key marks A, then B, then clears the pair.
        if key(Key::L) {
            out.push(Control::Loop);
        }
        // `a` walks the audio tracks of the open file, Shift back.
        if key(Key::A) {
            out.push(Control::AudioTrack(!modifiers.shift));
        }
        // VLC's snapshot key.
        if modifiers.shift && key(Key::S) {
            out.push(Control::Snapshot);
        }
        // VLC: `g` drops the cue delay, `h` raises it, so text comes later.
        if key(Key::G) {
            out.push(Control::SubtitleDelay(false));
        }
        if key(Key::H) {
            out.push(Control::SubtitleDelay(true));
        }
        if modifiers.alt {
            match (key(Key::ArrowUp), key(Key::ArrowDown)) {
                (true, _) => out.push(Control::SubtitleMargin(true)),
                (_, true) => out.push(Control::SubtitleMargin(false)),
                _ => {}
            }
        }
        if modifiers.alt {
            match (key(Key::Equals), key(Key::Minus)) {
                (true, _) => out.push(Control::SubtitleSize(true)),
                (_, true) => out.push(Control::SubtitleSize(false)),
                _ => {}
            }
        }
        for digit in 0..=9u8 {
            let key = match digit {
                0 => Key::Num0,
                1 => Key::Num1,
                2 => Key::Num2,
                3 => Key::Num3,
                4 => Key::Num4,
                5 => Key::Num5,
                6 => Key::Num6,
                7 => Key::Num7,
                8 => Key::Num8,
                _ => Key::Num9,
            };
            if input.key_pressed(key)
                && let Some(fraction) = position_from_digit(digit)
            {
                out.push(Control::Position(fraction));
            }
        }
    });
    out
}

impl eframe::App for Player {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if fullscreen {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.toggle_pause();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::O) && i.modifiers.command) {
            self.pick_file(Pick::Item);
        }
        for control in controls_pressed(ctx) {
            self.apply(control);
        }
        // A settings dialog left open over a paused frame moves sliders, and a
        // moved slider has to reach the picture the same way a moved crop does.
        self.refresh_rgb_texture();
        // One mouse notch is one VLC volume step; smaller trackpad ticks are
        // ignored rather than accumulated into a slide.
        if let Some(delta) = ctx.input(|i| Some(i.smooth_scroll_delta.y))
            && delta.abs() >= 24.0
        {
            self.change_volume(delta > 0.0);
        }
        if ctx
            .input(|i| i.pointer.is_moving() || i.pointer.any_pressed() || i.pointer.any_released())
        {
            self.activity = Instant::now();
        }
        // A drop of several files is a playlist, the way VLC queues them up.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            self.open_queue(expand_inputs(&dropped));
        }
        if self.dialog.is_some() {
            self.poll_dialog();
            if self.dialog.is_some() {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        self.present(ctx);
        if self.playback.is_some() && !self.paused && !self.ended {
            self.playback_activity
                .get_or_insert_with(fvid_platform::PlaybackActivity::new);
        } else {
            self.playback_activity = None;
        }
        // Wake up once to let the controls fade after the pointer rests.
        if self.controls_visible() && !self.paused {
            ctx.request_repaint_after(HIDE_AFTER.saturating_sub(self.activity.elapsed()));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let panel = egui::CentralPanel::default().frame(egui::Frame::NONE.fill(WINDOW));
        panel.show(ui, |ui| {
            let frame = ui.max_rect();
            let painter = ui.painter().with_clip_rect(frame);
            painter.rect_filled(frame, CornerRadius::ZERO, FRAME);

            // The picture, cropped to VLC's shape, drawn at the shape VLC's
            // aspect menu or its container gives it, fitted inside the frame and
            // then magnified the way its Zoom menu does, the window keeping its
            // size. Planar frames are converted to RGB by the GPU shader while
            // drawing, which is handed the crop as texture coordinates.
            if let Some((planes, serial)) = &self.video {
                let source = Vec2::new(planes.width as f32, planes.height as f32);
                let insets =
                    shown_insets(source, self.container_insets, self.pixel_aspect, self.crop);
                let size = shown_size(
                    cropped_size(source, insets),
                    self.pixel_aspect,
                    self.aspect,
                    frame.size(),
                );
                let rect = video_rect(frame, size, self.zoom_milli, self.pan);
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                    rect,
                    VideoCallback {
                        frame: planes.clone(),
                        serial: *serial,
                        window: uv_window(source, insets),
                        adjust: adjust_scalars(&self.adjust),
                    },
                ));
            } else if let Some(texture) = &self.texture {
                let source = texture.size_vec2();
                let insets =
                    shown_insets(source, self.container_insets, self.pixel_aspect, self.crop);
                let size = shown_size(
                    cropped_size(source, insets),
                    self.pixel_aspect,
                    self.aspect,
                    frame.size(),
                );
                let rect = video_rect(frame, size, self.zoom_milli, self.pan);
                let [u0, v0, u1, v1] = uv_window(source, insets);
                painter.image(
                    texture.id(),
                    rect,
                    Rect::from_min_max(Pos2::new(u0, v0), Pos2::new(u1, v1)),
                    Color32::WHITE,
                );
            }

            // An item that is only sound has no picture to show, so the stage
            // says what it is instead of looking like a failure.
            if self.playback.is_none() && self.item_open() {
                icon_note(
                    &painter,
                    Pos2::new(frame.center().x, frame.center().y - 70.0),
                    64.0,
                    DIM,
                );
            }

            // VLC's Ctrl+E dialog of picture settings: its switch, its five
            // sliders with its ranges and labels, and its button that puts the
            // defaults back. VLC keeps the window apart from the video; here
            // it stands over the picture, and the sliders write the bundle the
            // draw paths read every repaint.
            if self.effects {
                let adjust = &mut self.adjust;
                egui::Window::new("Adjustments and Effects")
                    .open(&mut self.effects)
                    .default_pos(frame.center() + Vec2::new(-120.0, -140.0))
                    .show(&ctx, |ui| {
                        ui.checkbox(&mut adjust.on, "Enable");
                        ui.add_enabled_ui(adjust.on, |ui| {
                            ui.add(
                                egui::Slider::new(&mut adjust.brightness, 0.0..=2.0)
                                    .text("Brightness"),
                            );
                            ui.add(
                                egui::Slider::new(&mut adjust.gamma, 0.01..=10.0)
                                    .logarithmic(true)
                                    .text("Gamma"),
                            );
                            ui.add(
                                egui::Slider::new(&mut adjust.saturation, 0.0..=3.0)
                                    .text("Colour saturation"),
                            );
                            ui.add(
                                egui::Slider::new(&mut adjust.contrast, 0.0..=2.0).text("Contrast"),
                            );
                            ui.add(egui::Slider::new(&mut adjust.hue, -180.0..=180.0).text("Tone"));
                            if ui.button("Restore Default Values").clicked() {
                                *adjust = Adjust {
                                    on: adjust.on,
                                    ..Adjust::default()
                                };
                            }
                        });
                    });
            }

            // Clicking the picture toggles playback; the buttons below take priority
            // because they are interacted with later in the same frame. A picture a
            // magnification has grown past the frame is dragged to the part of it
            // the frame cannot show, the way VLC pans a zoomed one; egui stops
            // counting a press that moved as a click, so the two answers do not
            // fight over the same gesture.
            let surface = ui.interact(frame, ui.id().with("surface"), Sense::click_and_drag());
            if surface.dragged() && self.zoom_milli > 1_000 {
                self.pan += surface.drag_delta();
            } else if surface.double_clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
            } else if surface.clicked() {
                if self.item_open() {
                    self.toggle_pause();
                } else {
                    self.pick_file(Pick::Item);
                }
            }

            let visible = self.controls_visible();
            if visible {
                ctx.set_cursor_icon(egui::CursorIcon::Default);
            } else {
                ctx.set_cursor_icon(egui::CursorIcon::None);
            }

            // Centre: one large play button while idle.
            if !self.item_open() || self.paused || self.ended {
                let center = frame.center();
                let size = 88.0_f32.min(frame.height() * 0.4);
                let clicked = round_button(ui, "big-play", center, size, CHIP, |p, c, color| {
                    icon_play(p, Pos2::new(c.x + size * 0.04, c.y), size * 0.34, color);
                });
                if clicked {
                    if self.item_open() {
                        self.toggle_pause();
                    } else {
                        self.pick_file(Pick::Item);
                    }
                }
                if !self.item_open() {
                    painter.text(
                        Pos2::new(center.x, center.y + size / 2.0 + 24.0),
                        Align2::CENTER_TOP,
                        "Open or drop a Y4M, MP4, MOV or Matroska file",
                        FontId::proportional(13.0),
                        MUTED,
                    );
                }
            }

            if !visible {
                return;
            }

            let pad = 28.0_f32.min(frame.width() * 0.05);

            // Top: title block and close.
            // Keep the title block clear of the macOS traffic lights over the hidden title bar.
            let title_left = if fullscreen { pad } else { pad.max(96.0) };
            let top = Pos2::new(frame.left() + title_left, frame.top() + 24.0);
            if self.item_open() {
                painter.text(
                    top,
                    Align2::LEFT_TOP,
                    &self.name,
                    FontId::proportional(17.0),
                    TEXT,
                );
                painter.text(
                    Pos2::new(top.x, top.y + 24.0),
                    Align2::LEFT_TOP,
                    self.subtitle(),
                    FontId::proportional(13.0),
                    MUTED,
                );
            } else {
                painter.text(
                    top,
                    Align2::LEFT_TOP,
                    "FVid",
                    FontId::proportional(17.0),
                    TEXT,
                );
            }
            if let Some(error) = &self.error {
                painter.text(
                    Pos2::new(top.x, top.y + 48.0),
                    Align2::LEFT_TOP,
                    error,
                    FontId::proportional(13.0),
                    ERROR,
                );
            }
            // What the item is, in the title's corner under the title, in the
            // subtitle line's type. An item that cannot be opened has nothing to
            // list here, so the panel and the error never share the corner.
            if self.info {
                let font = FontId::proportional(13.0);
                for (nth, line) in self.info_lines().into_iter().enumerate() {
                    painter.text(
                        Pos2::new(top.x, top.y + 48.0 + 18.0 * nth as f32),
                        Align2::LEFT_TOP,
                        line,
                        font.clone(),
                        TEXT,
                    );
                }
            }
            // Subtitle first, so a control message can still be read over it.
            if let Some(line) = self
                .timeline()
                .0
                .and_then(|at| self.subtitle_line(at))
                .map(str::to_owned)
            {
                let font = FontId::proportional(self.subtitle_font);
                let wrapped = painter.layout(
                    line.replace('\n', " "),
                    font,
                    Color32::WHITE,
                    frame.width() * 0.86,
                );
                let at = Pos2::new(
                    frame.center().x,
                    frame.bottom() - self.subtitle_margin - wrapped.size().y,
                );
                // A hard outline keeps white text readable on a bright picture.
                for offset in [
                    Vec2::new(-1.5, 0.0),
                    Vec2::new(1.5, 0.0),
                    Vec2::new(0.0, -1.5),
                    Vec2::new(0.0, 1.5),
                ] {
                    painter.galley(at + offset, wrapped.clone(), Color32::from_rgb(0, 0, 0));
                }
                painter.galley(at, wrapped, Color32::from_rgb(0xff, 0xff, 0xff));
            }

            if let Some((message, since)) = &self.osd
                && since.elapsed() < OSD_AFTER
            {
                painter.text(
                    Pos2::new(frame.center().x, frame.top() + 24.0),
                    Align2::CENTER_TOP,
                    message,
                    FontId::proportional(15.0),
                    TEXT,
                );
            }
            let close_at = Pos2::new(
                frame.right() - pad - BUTTON / 2.0,
                frame.top() + 24.0 + BUTTON / 2.0,
            );
            if round_button(ui, "close", close_at, BUTTON, CHIP, |p, c, color| {
                icon_close(p, c, 12.0, color);
            }) {
                if fullscreen {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }

            // Bottom: progress line, then the control row.
            let row_y = frame.bottom() - 22.0 - BUTTON / 2.0;
            let bar_y = row_y - BUTTON / 2.0 - 14.0;
            let bar = Rect::from_min_max(
                Pos2::new(frame.left() + pad, bar_y - 2.0),
                Pos2::new(frame.right() - pad, bar_y + 2.0),
            );
            painter.rect_filled(bar, CornerRadius::same(2), TRACK);
            let (elapsed, total) = self.timeline();
            let mut fraction = match (elapsed, total) {
                (Some(e), Some(t)) if t > Duration::ZERO => {
                    Some(if self.ended && self.error.is_none() {
                        1.0
                    } else {
                        (e.as_secs_f32() / t.as_secs_f32()).clamp(0.0, 1.0)
                    })
                }
                _ => None,
            };
            // The line takes clicks and drags on a taller hit area; the seek
            // itself happens on release so a drag decodes only once.
            if fraction.is_some() && self.seekable {
                let hit = Rect::from_min_max(
                    Pos2::new(bar.left(), bar_y - 12.0),
                    Pos2::new(bar.right(), bar_y + 12.0),
                );
                let seek = ui.interact(hit, ui.id().with("seek"), Sense::click_and_drag());
                let at = |pos: Pos2| ((pos.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
                if seek.dragged()
                    && let Some(pos) = seek.interact_pointer_pos()
                {
                    self.scrub = Some(at(pos));
                }
                if (seek.drag_stopped() || seek.clicked())
                    && let Some(pos) = seek.interact_pointer_pos()
                {
                    let target = at(pos);
                    self.scrub = None;
                    self.seek_fraction(target);
                    fraction = Some(target);
                }
                if seek.hovered() || self.scrub.is_some() {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if let Some(scrub) = self.scrub {
                    fraction = Some(scrub);
                }
            }
            if let Some(fraction) = fraction {
                let x = bar.left() + bar.width() * fraction;
                painter.rect_filled(
                    Rect::from_min_max(bar.min, Pos2::new(x, bar.max.y)),
                    CornerRadius::same(2),
                    ACCENT,
                );
                painter.circle_filled(Pos2::new(x, bar_y), 7.0, TEXT);
            }

            // Left group: play/pause, restart, time.
            let mut x = frame.left() + pad + BUTTON / 2.0;
            let playing = self.item_open() && !self.paused && !self.ended;
            let clicked = round_button(
                ui,
                "play",
                Pos2::new(x, row_y),
                BUTTON,
                Color32::TRANSPARENT,
                |p, c, color| {
                    if playing {
                        icon_pause(p, c, 16.0, color);
                    } else {
                        icon_play(p, Pos2::new(c.x + 1.0, c.y), 16.0, color);
                    }
                },
            );
            if clicked {
                if self.item_open() {
                    self.toggle_pause();
                } else {
                    self.pick_file(Pick::Item);
                }
            }
            x += BUTTON + 4.0;
            if round_button(
                ui,
                "restart",
                Pos2::new(x, row_y),
                BUTTON,
                Color32::TRANSPARENT,
                |p, c, color| {
                    icon_restart(p, c, 14.0, color);
                },
            ) {
                self.restart();
            }
            x += BUTTON / 2.0 + 16.0;
            if let Some(elapsed) = elapsed {
                let font = FontId::monospace(13.0);
                let end = painter.text(
                    Pos2::new(x, row_y),
                    Align2::LEFT_CENTER,
                    clock(elapsed),
                    font.clone(),
                    TEXT,
                );
                if let Some(total) = total {
                    painter.text(
                        Pos2::new(end.right() + 6.0, row_y),
                        Align2::LEFT_CENTER,
                        format!("/ {}", clock(total)),
                        font,
                        DIM,
                    );
                }
            }

            // Right group: open, fullscreen.
            let mut x = frame.right() - pad - BUTTON / 2.0;
            if round_button(
                ui,
                "fullscreen",
                Pos2::new(x, row_y),
                BUTTON,
                Color32::TRANSPARENT,
                |p, c, color| {
                    icon_fullscreen(p, c, 16.0, color, fullscreen);
                },
            ) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fullscreen));
            }
            x -= BUTTON + 4.0;
            let label = "Open…";
            let font = FontId::proportional(13.0);
            let width = painter
                .layout_no_wrap(label.to_owned(), font.clone(), TEXT)
                .size()
                .x
                + 24.0;
            let open_rect = Rect::from_center_size(
                Pos2::new(x + BUTTON / 2.0 - width / 2.0, row_y),
                Vec2::new(width, BUTTON),
            );
            let open = ui.interact(open_rect, ui.id().with("open"), Sense::click());
            if open.hovered() {
                painter.rect_filled(open_rect, CornerRadius::same(22), CHIP_STRONG);
            }
            painter.text(open_rect.center(), Align2::CENTER_CENTER, label, font, TEXT);
            if open.clicked() {
                self.pick_file(Pick::Item);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ADJUST_IDENTITY, ASPECTS, Adjust, Aspect, CROPS, ChapterMark, Control, FileTags, Frame,
        Grading, HIDE_AFTER, LoopMark, NO_CROP, PANEL_NITS, Panel, PathBuf, Pixels, Planar8,
        PlayArgs, PlayBounds, Player, Pos2, RATES, Rect, Repeat, SubtitleSource, VOLUME_MAX, Vec2,
        adjust_luma, adjust_rgb, adjust_scalars, advance, aspect_label, aspect_osd, aspect_step,
        bitrate_text, byte_size, chapter_ahead, colour_line, container_facts, crop_insets,
        crop_label, crop_osd, crop_step, cropped_size, cycle_repeat, deal_cycle, delay_step,
        delayed_clock, display_size, expand_inputs, file_size, fps_text, grade_text, jump_size,
        light_line, loop_press, loop_rewind, paced_period, parse_clock, parse_play_args,
        playlist_osd, position_from_digit, rate_fine, rate_osd, rate_step, repeat_osd, retreat,
        shown_insets, shown_size, snapshot_name, sound_codec, subtitles, track_step, uv_window,
        video_rect, volume_osd, volume_step, zoom_osd, zoom_step,
    };
    use super::{Event, NativeReader, Playback};
    use crate::color::{
        ColourDescription, ContentLight, DisplayTarget, HdrMetadata, Interpolation, Log, Lut,
        MasteringDisplay, Primaries, ToneMap, Transfer,
    };
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// `fvid play` sees strings; the tests write words.
    fn play_args(words: &[&str]) -> crate::Result<PlayArgs> {
        parse_play_args(
            &words
                .iter()
                .map(|word| word.to_string())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn volume_walks_the_slider_and_stops_at_both_ends() {
        assert_eq!(volume_step(1_000, true), 1_050);
        assert_eq!(volume_step(1_000, false), 950);
        assert_eq!(volume_step(0, false), 0);
        assert_eq!(volume_step(VOLUME_MAX, true), VOLUME_MAX);
    }

    #[test]
    fn rate_steps_through_the_vlc_presets() {
        let mut rate = 1_000;
        let mut up = Vec::new();
        for _ in 0..4 {
            rate = rate_step(rate, true);
            up.push(rate);
        }
        assert_eq!(up, vec![1_500, 2_000, 3_000, 4_000]);
        for _ in 0..8 {
            rate = rate_step(rate, false);
        }
        assert_eq!(rate, RATES[0]);
        // A rate from the command line snaps to the next preset on the ladder.
        assert_eq!(rate_step(1_200, true), 1_500);
        assert_eq!(rate_step(1_200, false), 1_000);
    }

    #[test]
    fn the_fine_rate_walks_by_tenths_inside_the_ladder() {
        assert_eq!(rate_fine(1_000, true), 1_100);
        assert_eq!(rate_fine(1_000, false), 900);
        // Fine steps up from normal reach the top rung and stop there, the
        // same way the preset ladder does at either end: it takes thirty
        // tenths to walk 1× to 4×, and more ask for a speed that is not there.
        let mut rate = 1_000;
        for _ in 0..35 {
            rate = rate_fine(rate, true);
        }
        assert_eq!(rate, *RATES.last().unwrap());
        for _ in 0..45 {
            rate = rate_fine(rate, false);
        }
        assert_eq!(rate, RATES[0]);
        // A rate the fine pair invented is still a rung the preset keys can
        // step back from: each snaps to the neighbouring preset on its side.
        assert_eq!(rate_step(rate_fine(1_000, true), true), 1_500);
        assert_eq!(rate_step(rate_fine(1_000, false), false), 500);
    }

    #[test]
    fn a_faster_rate_shows_each_picture_for_shorter() {
        let frame = Duration::from_millis(40);
        assert_eq!(paced_period(frame, 1_000), frame);
        assert_eq!(paced_period(frame, 2_000), Duration::from_millis(20));
        assert_eq!(paced_period(frame, 500), Duration::from_millis(80));
    }

    #[test]
    fn jump_modifiers_take_the_longer_rungs() {
        assert_eq!(jump_size(false, false), Duration::from_secs(3));
        assert_eq!(jump_size(true, false), Duration::from_secs(10));
        assert_eq!(jump_size(false, true), Duration::from_secs(60));
    }

    #[test]
    fn number_keys_are_tenths_of_the_timeline() {
        assert_eq!(position_from_digit(0), Some(0.0));
        assert_eq!(position_from_digit(5), Some(0.5));
        assert_eq!(position_from_digit(9), Some(0.9));
        assert_eq!(position_from_digit(10), None);
    }

    /// A wide pixel stretches the picture sideways and a tall one squeezes it; the
    /// height is the number that stands, since a pixel changes shape rather than
    /// adding rows.
    #[test]
    fn a_pixel_shape_changes_the_width_it_draws_at() {
        let coded = Vec2::new(64.0, 64.0);
        assert_eq!(display_size(coded, (1, 1)), coded);
        assert_eq!(display_size(coded, (2, 1)), Vec2::new(128.0, 64.0));
        assert_eq!(display_size(coded, (1, 2)), Vec2::new(32.0, 64.0));
        assert_eq!(display_size(coded, (4, 3)), Vec2::new(85.333336, 64.0));
    }

    /// A 16:9 picture in a 4:3 stage has the stage's width to live in, so it
    /// fits at half its coded size with a bar over and under. Zooming doubles
    /// what the fitting left, which sends the picture past the stage on every
    /// side — the part outside it is simply not shown, the way VLC's window
    /// keeps its own size under a magnification.
    #[test]
    fn a_magnified_picture_grows_past_the_frame_it_was_fitted_to() {
        let stage = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
        let picture = Vec2::new(1600.0, 900.0);
        assert_eq!(
            video_rect(stage, picture, 1_000, Vec2::ZERO),
            Rect::from_min_max(Pos2::new(0.0, 75.0), Pos2::new(800.0, 525.0))
        );
        assert_eq!(
            video_rect(stage, picture, 2_000, Vec2::ZERO),
            Rect::from_min_max(Pos2::new(-400.0, -150.0), Pos2::new(1200.0, 750.0))
        );
        assert_eq!(
            video_rect(stage, picture, 500, Vec2::ZERO),
            Rect::from_min_max(Pos2::new(200.0, 187.5), Pos2::new(600.0, 412.5))
        );
    }

    /// Sliding a picture only counts once it overhangs: the drag can bring an
    /// overhanging edge back over the stage, and no further, so a dragged
    /// picture never uncovers the bar the stage would have shown behind it.
    /// One that fits inside the stage cannot be moved at all.
    #[test]
    fn only_a_magnified_picture_slides_and_never_bare_an_edge() {
        let stage = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
        let picture = Vec2::new(1600.0, 900.0);
        // Doubled, the picture overhangs by half a stage each way.
        assert_eq!(
            video_rect(stage, picture, 2_000, Vec2::new(400.0, 150.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1600.0, 900.0))
        );
        assert_eq!(
            video_rect(stage, picture, 2_000, Vec2::new(5000.0, -5000.0)),
            Rect::from_min_max(Pos2::new(0.0, -300.0), Pos2::new(1600.0, 600.0))
        );
        // A quarter of the stage has room to spare on all sides, so it stays put.
        assert_eq!(
            video_rect(stage, picture, 500, Vec2::new(900.0, 900.0)),
            video_rect(stage, picture, 500, Vec2::ZERO)
        );
    }

    #[test]
    fn control_lines_read_like_vlc() {
        assert_eq!(volume_osd(1_050, false), "Volume 105%");
        assert_eq!(volume_osd(1_050, true), "Muted");
        assert_eq!(rate_osd(1_000), "1\u{d7}");
        assert_eq!(rate_osd(1_500), "1.5\u{d7}");
        assert_eq!(rate_osd(250), "0.25\u{d7}");
        assert_eq!(zoom_osd(1_000), "Zoom 1\u{d7}");
        assert_eq!(zoom_osd(250), "Zoom 0.25\u{d7}");
        assert_eq!(crop_osd((4, 3)), "Crop 4:3");
        assert_eq!(crop_osd((235, 100)), "Crop 2.35:1");
        assert_eq!(crop_osd(NO_CROP), "Crop Default");
        assert_eq!(aspect_osd(Aspect::Shape(4, 3)), "Aspect 4:3");
        assert_eq!(aspect_osd(Aspect::Shape(235, 100)), "Aspect 2.35:1");
        assert_eq!(aspect_osd(Aspect::Source), "Aspect Default");
        assert_eq!(aspect_osd(Aspect::Fill), "Aspect Fill");
    }

    /// The four rungs are VLC's Zoom menu, and the key stops at either end
    /// rather than wrapping back to the quarter size.
    #[test]
    fn the_zoom_key_walks_vlcs_menu_and_stops_at_both_ends() {
        assert_eq!(zoom_step(1_000, true), 2_000);
        assert_eq!(zoom_step(2_000, true), 2_000);
        assert_eq!(zoom_step(250, false), 250);
        // A size from the command line snaps to the next rung of the menu.
        assert_eq!(zoom_step(1_200, true), 2_000);
        assert_eq!(zoom_step(1_200, false), 1_000);
        let mut player = Player::default();
        assert_eq!(player.zoom_milli, 1_000);
        player.apply(Control::Zoom(true));
        assert_eq!(player.zoom_milli, 2_000);
        assert_eq!(player.osd.as_ref().unwrap().0, "Zoom 2\u{d7}");
        for _ in 0..4 {
            player.apply(Control::Zoom(false));
        }
        assert_eq!(player.zoom_milli, 250);
    }

    /// `--zoom` is VLC's own option: a multiple of the fitted picture, held to
    /// the span its menu and its scale keys cover between them.
    #[test]
    fn the_command_line_hands_the_picture_a_magnification_to_start_at() {
        let parsed = play_args(&["--zoom", "2", "clip.mp4"]).unwrap();
        assert_eq!(parsed.zoom_milli, 2_000);
        assert_eq!(play_args(&["--zoom=0.5", "c"]).unwrap().zoom_milli, 500);
        assert_eq!(
            play_args(&["--zoom", "1.25", "c"]).unwrap().zoom_milli,
            1_250
        );
        assert_eq!(play_args(&["c"]).unwrap().zoom_milli, 1_000);
        for words in [
            vec!["--zoom", "0.2"],
            vec!["--zoom", "11"],
            vec!["--zoom", "wide"],
            vec!["--zoom"],
            vec!["--zoom=0"],
        ] {
            assert!(
                play_args(&[words.as_slice(), &["c"]].concat()).is_err(),
                "{words:?} is not a magnification"
            );
        }
    }

    /// VLC's list starts with the whole picture, so walking it forward far
    /// enough returns there, and the same key walked back reaches the other end
    /// of the list the same way. A shape only `--crop` named is not on the list,
    /// so the walk takes it to the first shape the list does hold.
    #[test]
    fn the_crop_key_walks_vlcs_shapes_and_back_to_the_whole_picture() {
        assert_eq!(crop_step(NO_CROP, true), (16, 10));
        assert_eq!(crop_step(CROPS[CROPS.len() - 1], true), NO_CROP);
        assert_eq!(crop_step(NO_CROP, false), (1, 1));
        assert_eq!(crop_step((3, 2), true), (16, 10));
        let mut shape = NO_CROP;
        for _ in 0..CROPS.len() {
            shape = crop_step(shape, true);
        }
        assert_eq!(shape, NO_CROP);
        let mut player = Player::default();
        assert_eq!(player.crop, NO_CROP);
        player.apply(Control::Crop(true));
        assert_eq!(player.crop, (16, 10));
        assert_eq!(player.osd.as_ref().unwrap().0, "Crop 16:10");
    }

    /// A wider picture than the shape loses its sides, a taller one its top and
    /// bottom, and what is left is as large as it can be while staying centred.
    /// The shape the file already has cuts nothing at all.
    #[test]
    fn a_crop_keeps_the_largest_centred_part_of_the_picture() {
        let square = Vec2::new(800.0, 600.0);
        assert_eq!(crop_insets(square, (1, 1), (1, 1)), [100, 0, 100, 0]);
        assert_eq!(crop_insets(square, (1, 1), (16, 9)), [0, 75, 0, 75]);
        assert_eq!(crop_insets(square, (1, 1), (16, 10)), [0, 50, 0, 50]);
        assert_eq!(crop_insets(square, (1, 1), (4, 3)), [0, 0, 0, 0]);
        // The kept rows are 335 of 600, so the row no half-step can split falls
        // to the bottom edge.
        assert_eq!(crop_insets(square, (1, 1), (239, 100)), [0, 132, 0, 133]);
        // Cropping the 16:9 file to 4:3 gives up its sides, not its height.
        assert_eq!(
            crop_insets(Vec2::new(1600.0, 900.0), (1, 1), (4, 3)),
            [200, 0, 200, 0]
        );
    }

    /// A container that stretches each pixel sideways asks for fewer of them to
    /// span a shape, so the ratio names what is seen rather than what is stored:
    /// twice-as-wide pixels make this file's 800x600 look 1600x600, and its
    /// square is 300 stored pixels across.
    #[test]
    fn a_crop_counts_the_shape_the_container_draws_and_not_the_one_stored() {
        let stored = Vec2::new(800.0, 600.0);
        let insets = crop_insets(stored, (2, 1), (1, 1));
        assert_eq!(insets, [250, 0, 250, 0]);
        assert_eq!(
            display_size(cropped_size(stored, insets), (2, 1)),
            Vec2::new(600.0, 600.0)
        );
    }

    /// The container's borders and the Crop menu's shape add up from the coded
    /// edges inward: the menu measures its square against the picture that is
    /// left after the file's own borders, and the two cuts are then one window.
    /// A file with no borders is the menu alone, and the menu's Default leaves
    /// the file's borders exactly as they are.
    #[test]
    fn the_container_borders_and_the_crop_menu_stack_from_the_coded_edges() {
        // Nothing stated by the file: the menu decides on its own, both ways.
        let plain = Vec2::new(1600.0, 900.0);
        assert_eq!(shown_insets(plain, [0; 4], (1, 1), (16, 9)), [0; 4]);
        assert_eq!(
            shown_insets(plain, [0; 4], (1, 1), (4, 3)),
            crop_insets(plain, (1, 1), (4, 3))
        );
        // 16x16 coded, four pixels cut from each side and two from top and
        // bottom leaves 8x12; the menu's square keeps 8x8 of that, so its own
        // cut is two rows at each of the remaining ends.
        let small = Vec2::new(16.0, 16.0);
        assert_eq!(
            shown_insets(small, [4, 2, 4, 2], (1, 1), (1, 1)),
            [4, 4, 4, 4]
        );
        assert_eq!(
            shown_insets(small, [4, 2, 4, 2], (1, 1), NO_CROP),
            [4, 2, 4, 2]
        );
        // Borders that cannot fit inside the frame at all — as after a failed
        // rewind to a smaller stream — are dropped rather than shown as
        // nothing, and the menu then works on the whole picture.
        assert_eq!(
            shown_insets(plain, [800, 0, 800, 0], (1, 1), (4, 3)),
            [200, 0, 200, 0]
        );
        assert_eq!(
            shown_insets(plain, [800, 0, 800, 0], (1, 1), NO_CROP),
            [0; 4]
        );
    }

    /// Ctrl+E, the accelerator VLC gives its Tools ▸ Effects and Filters menu
    /// entry, opens and closes the dialog here. The settings outlive both the
    /// closed dialog and a change of item, as the filter's values outlive them
    /// in VLC until its switch turns the filter off.
    #[test]
    fn the_effects_dialog_opens_on_vlcs_own_accelerator_and_keeps_its_values() {
        let mut player = Player::default();
        assert!(!player.effects);
        assert_eq!(player.adjust, Adjust::default());
        player.apply(Control::Effects);
        assert!(player.effects);
        player.adjust.brightness = 1.4;
        player.apply(Control::Effects);
        assert!(!player.effects);
        player.apply(Control::Effects);
        assert_eq!(player.adjust.brightness, 1.4);
    }

    /// VLC's dialog opens on its filter's defaults — every multiplier at one,
    /// the turn at nothing — and those defaults, and the switch off, are each
    /// exactly the bundle that leaves a sample alone.
    #[test]
    fn the_settings_bundle_is_the_identity_at_vlcs_defaults() {
        assert_eq!(adjust_scalars(&Adjust::default()), ADJUST_IDENTITY);
        let on = Adjust {
            on: true,
            ..Adjust::default()
        };
        assert_eq!(adjust_scalars(&on), ADJUST_IDENTITY);
    }

    /// The sliders fold into the five numbers the way VLC's adjust filter
    /// folds them: contrast scales the stored sample, brightness offsets it
    /// with the contrast's own midpoint (`(b−1)·255 + 128·(1−c)`, so raising
    /// one without the other still anchors black and white as the filter's
    /// tables do), gamma enters inverted, and the hue's turn and the
    /// saturation's scale multiply into one pair.
    #[test]
    fn the_sliders_fold_into_the_bundle_the_way_vlcs_filter_folds_them() {
        let s = adjust_scalars(&Adjust {
            on: true,
            brightness: 1.5,
            contrast: 0.5,
            gamma: 2.0,
            saturation: 3.0,
            hue: 90.0,
        });
        assert_eq!(s[0], 0.5);
        assert_eq!(s[1], 0.5 * 255.0 + 128.0 * 0.5);
        assert!((s[2] - 0.5).abs() < 1e-6);
        // A quarter turn at three times the swing: the cosine vanishes and
        // the sine carries the whole saturation.
        assert!(s[3].abs() < 1e-5);
        assert!((s[4] - 3.0).abs() < 1e-5);
    }

    /// The luma half of the bundle, on stored samples: black and white hold
    /// their anchors through contrast, a raised gamma lifts the mid-grey the
    /// way VLC's gamma slider lifts it, and the identity bundle leaves every
    /// sample at itself.
    #[test]
    fn the_adjusted_luma_holds_its_anchors_and_bends_its_midpoint() {
        let mid = adjust_luma(128.0, &ADJUST_IDENTITY);
        assert!((mid - 128.0).abs() < 1e-4);
        assert_eq!(adjust_luma(0.0, &ADJUST_IDENTITY), 0.0);
        assert!((adjust_luma(255.0, &ADJUST_IDENTITY) - 255.0).abs() < 1e-4);
        let contrast = [2.0, 128.0 * (1.0 - 2.0), 1.0, 1.0, 0.0];
        assert_eq!(adjust_luma(0.0, &contrast), 0.0);
        assert_eq!(adjust_luma(255.0, &contrast), 255.0);
        assert!((adjust_luma(192.0, &contrast) - 255.0).abs() < 1e-4);
        let gamma = [1.0, 0.0, 0.5, 1.0, 0.0];
        assert!(adjust_luma(128.0, &gamma) > 180.0);
    }

    /// The software path's pixels are already RGB, so the filter's maths runs
    /// on them through the same 601 matrix the converter used to make them.
    /// At the identity the round trip costs at most a rounding step; zeroing
    /// the saturation drains the colour out to grey at the luma the file
    /// carried; a half-turn of hue turns red into its opposite on the colour
    /// wheel, cyan, exactly where VLC's Tone slider lands it.
    #[test]
    fn the_rgb_pixels_follow_the_bundle_the_way_the_planes_do() {
        let red = [255, 40, 40, 120, 120, 120];
        let mut out = Vec::new();
        adjust_rgb(&red, &mut out, &ADJUST_IDENTITY);
        for (got, put) in out.iter().zip(&red) {
            assert!((i16::from(*got) - i16::from(*put)).abs() <= 1);
        }
        // Saturation zero: the two chroma multipliers fall to nothing.
        adjust_rgb(&red, &mut out, &[1.0, 0.0, 1.0, 0.0, 0.0]);
        assert_eq!(out[0], out[1]);
        assert_eq!(out[1], out[2]);
        // A grey, whatever the bundle, stays a grey: it has no chroma to turn.
        adjust_rgb(&red, &mut out, &[1.0, 0.0, 1.0, -1.0, 0.4]);
        assert!(out[3] < 200);
        assert!((out[3] as i32 - out[5] as i32).abs() < 8);
        adjust_rgb(&red, &mut out, &[1.0, 0.0, 1.0, -1.0, 0.0]);
        // Half a turn on red: green and blue rise to where red falls.
        assert!(out[0] < out[1] && out[0] < out[2]);
        assert!((out[3] as i32 - out[5] as i32).abs() < 16);
    }

    /// The region a crop keeps is the whole of the picture as far as the frame
    /// is concerned: the fitting measures what is left, so a 16:9 file cropped to
    /// 4:3 fills a 4:3 stage edge to edge instead of shrinking inside it. The
    /// texture coordinates say where the same region sits in the frame that is
    /// still uploaded whole.
    #[test]
    fn a_cropped_picture_is_fitted_to_the_frame_by_what_is_left() {
        let stage = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
        let stored = Vec2::new(1600.0, 900.0);
        let insets = crop_insets(stored, (1, 1), (4, 3));
        let kept = display_size(cropped_size(stored, insets), (1, 1));
        assert_eq!(kept, Vec2::new(1200.0, 900.0));
        assert_eq!(
            video_rect(stage, kept, 1_000, Vec2::ZERO),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0))
        );
        assert_eq!(uv_window(stored, insets), [0.125, 0.0, 0.875, 1.0]);
        // The menu's Default leaves both the fitting and the coordinates alone.
        let none = crop_insets(stored, (1, 1), NO_CROP);
        assert_eq!(uv_window(stored, none), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(cropped_size(stored, none), stored);
        // A square crop of a 4:3 frame spans the middle of its columns.
        assert_eq!(
            uv_window(
                Vec2::new(800.0, 600.0),
                crop_insets(Vec2::new(800.0, 600.0), (1, 1), (1, 1))
            ),
            [0.125, 0.0, 0.875, 1.0]
        );
    }

    /// A cinema ratio is the menu's own number; the label VLC gives it is what
    /// the OSD says, and a shape it never lists keeps the numbers it was given.
    #[test]
    fn crop_shapes_read_the_way_vlcs_menu_writes_them() {
        assert_eq!(crop_label(NO_CROP), "Default");
        assert_eq!(crop_label((16, 9)), "16:9");
        assert_eq!(crop_label((185, 100)), "1.85:1");
        assert_eq!(crop_label((221, 100)), "2.21:1");
        assert_eq!(crop_label((3, 2)), "3:2");
    }

    /// `--crop` is VLC's option of the same name: a shape split by `:`, which
    /// `none` clears. Everything else about it is a mistake at the door.
    #[test]
    fn the_command_line_names_the_shape_to_start_cropped_to() {
        assert_eq!(play_args(&["--crop", "16:9", "c"]).unwrap().crop, (16, 9));
        assert_eq!(play_args(&["--crop=4:3", "c"]).unwrap().crop, (4, 3));
        assert_eq!(
            play_args(&["--crop", "185:100", "c"]).unwrap().crop,
            (185, 100)
        );
        assert_eq!(play_args(&["--crop", "none", "c"]).unwrap().crop, NO_CROP);
        assert_eq!(play_args(&["c"]).unwrap().crop, NO_CROP);
        for words in [
            vec!["--crop", "16"],
            vec!["--crop", "0:1"],
            vec!["--crop", "1001:100"],
            vec!["--crop", "16:"],
            vec!["--crop", "wide"],
            vec!["--crop"],
            vec!["--crop=2:3:4"],
        ] {
            assert!(
                play_args(&[words.as_slice(), &["c"]].concat()).is_err(),
                "{words:?} is not a shape"
            );
        }
    }

    /// The five picture settings come over the command line under the names
    /// VLC gives its adjust options, minus its prefix. As in VLC, asking for
    /// one is asking for the filter: the switch goes on and the settings not
    /// named start at their defaults. A number outside its slider's span is a
    /// mistake at the door.
    #[test]
    fn the_command_line_names_the_picture_settings_to_start_with() {
        let parsed = play_args(&["--brightness", "1.5", "--hue=-30", "c"]).unwrap();
        assert!(parsed.adjust.on);
        assert_eq!(parsed.adjust.brightness, 1.5);
        assert_eq!(parsed.adjust.hue, -30.0);
        assert_eq!(parsed.adjust.contrast, 1.0);
        let parsed = play_args(&["c"]).unwrap();
        assert!(!parsed.adjust.on);
        assert_eq!(parsed.adjust, Adjust::default());
        for words in [
            vec!["--brightness", "2.5"],
            vec!["--brightness", "wide"],
            vec!["--gamma", "0"],
            vec!["--saturation", "3.1"],
            vec!["--contrast", "-1"],
            vec!["--hue", "181"],
            vec!["--hue"],
        ] {
            assert!(
                play_args(&[words.as_slice(), &["c"]].concat()).is_err(),
                "{words:?} is not a setting"
            );
        }
    }

    /// VLC's aspect-ratio menu starts at the shape the file declares, so walking
    /// it far enough returns there, and its last rung is the window's own shape.
    /// A shape only `--aspect` named is not on the menu, so the walk takes it to
    /// the first shape the menu does hold.
    #[test]
    fn the_aspect_key_walks_vlcs_shapes_and_back_to_the_files_own() {
        assert_eq!(aspect_step(Aspect::Source, true), Aspect::Shape(16, 9));
        assert_eq!(aspect_step(Aspect::Shape(5, 4), true), Aspect::Fill);
        assert_eq!(
            aspect_step(ASPECTS[ASPECTS.len() - 1], true),
            Aspect::Source
        );
        assert_eq!(aspect_step(Aspect::Source, false), Aspect::Fill);
        assert_eq!(aspect_step(Aspect::Shape(5, 3), true), Aspect::Shape(16, 9));
        let mut shape = Aspect::Source;
        for _ in 0..ASPECTS.len() {
            shape = aspect_step(shape, true);
        }
        assert_eq!(shape, Aspect::Source);
        let mut player = Player::default();
        assert_eq!(player.aspect, Aspect::Source);
        player.apply(Control::Aspect(true));
        assert_eq!(player.aspect, Aspect::Shape(16, 9));
        assert_eq!(player.osd.as_ref().unwrap().0, "Aspect 16:9");
    }

    /// A forced shape is the proportion the picture is drawn at, whatever the
    /// container said: the height stands and the width follows, as it does for
    /// the file's own pixels. `Fill` asks for the frame instead of a number.
    #[test]
    fn a_forced_shape_is_the_proportion_the_picture_is_drawn_at() {
        let square = Vec2::new(64.0, 64.0);
        assert_eq!(
            shown_size(square, (2, 1), Aspect::Source, Vec2::new(1280.0, 720.0)),
            Vec2::new(128.0, 64.0)
        );
        assert_eq!(
            shown_size(
                Vec2::new(800.0, 600.0),
                (2, 1),
                Aspect::Shape(4, 3),
                Vec2::ZERO
            ),
            Vec2::new(800.0, 600.0)
        );
        assert_eq!(
            shown_size(
                Vec2::new(1920.0, 1080.0),
                (1, 1),
                Aspect::Shape(16, 9),
                Vec2::ZERO
            ),
            Vec2::new(1920.0, 1080.0)
        );
        assert_eq!(
            shown_size(square, (1, 1), Aspect::Shape(235, 100), Vec2::ZERO),
            Vec2::new(150.4, 64.0)
        );
        assert_eq!(
            shown_size(square, (1, 1), Aspect::Fill, Vec2::new(1280.0, 720.0)),
            Vec2::new(1280.0, 720.0)
        );
    }

    /// The forced shape reaches the screen through the same fitting as the
    /// container's own: a square file asked to be twice as wide as tall spans the
    /// stage's width and is letterboxed above and below, and `Fill` leaves
    /// nothing to letterbox.
    #[test]
    fn a_forced_shape_is_fitted_to_the_frame_like_the_files_own() {
        let stage = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
        let square = Vec2::new(64.0, 64.0);
        assert_eq!(
            video_rect(
                stage,
                shown_size(square, (1, 1), Aspect::Shape(2, 1), stage.size()),
                1_000,
                Vec2::ZERO
            ),
            Rect::from_min_max(Pos2::new(0.0, 100.0), Pos2::new(800.0, 500.0))
        );
        assert_eq!(
            video_rect(
                stage,
                shown_size(square, (1, 1), Aspect::Fill, stage.size()),
                1_000,
                Vec2::ZERO
            ),
            stage
        );
        // Leaving the shape alone keeps the square it always drew.
        assert_eq!(
            video_rect(
                stage,
                shown_size(square, (1, 1), Aspect::Source, stage.size()),
                1_000,
                Vec2::ZERO
            ),
            Rect::from_min_max(Pos2::new(100.0, 0.0), Pos2::new(700.0, 600.0))
        );
    }

    /// A crop is cut from the stored picture before a shape is forced on what is
    /// left, the way VLC's two overrides stack, so a 4:3 crop of a 16:9 file
    /// under a forced 16:9 ends up 16:9 again. The crop still measures itself
    /// against the shape the container declared, forced shape or not.
    #[test]
    fn a_crop_and_a_forced_shape_apply_one_after_the_other() {
        let stored = Vec2::new(1600.0, 900.0);
        let insets = crop_insets(stored, (1, 1), (4, 3));
        assert_eq!(
            shown_size(
                cropped_size(stored, insets),
                (1, 1),
                Aspect::Shape(16, 9),
                Vec2::ZERO
            ),
            Vec2::new(1600.0, 900.0)
        );
        let wide = crop_insets(Vec2::new(800.0, 600.0), (2, 1), (1, 1));
        assert_eq!(wide, [250, 0, 250, 0]);
        assert_eq!(
            uv_window(Vec2::new(800.0, 600.0), wide),
            [0.3125, 0.0, 0.6875, 1.0]
        );
    }

    /// A cinema ratio is the menu's own number and its label is what the OSD
    /// says; the two rungs that name no shape say so in words.
    #[test]
    fn aspect_shapes_read_the_way_vlcs_menu_writes_them() {
        assert_eq!(aspect_label(Aspect::Source), "Default");
        assert_eq!(aspect_label(Aspect::Shape(16, 9)), "16:9");
        assert_eq!(aspect_label(Aspect::Shape(235, 100)), "2.35:1");
        assert_eq!(aspect_label(Aspect::Shape(221, 100)), "2.21:1");
        assert_eq!(aspect_label(Aspect::Shape(3, 2)), "3:2");
        assert_eq!(aspect_label(Aspect::Fill), "Fill");
    }

    /// `--aspect` is VLC's option of the same name: a shape split by `:`, or one
    /// of the two words for the rungs that name no shape. Everything else about
    /// it is a mistake at the door.
    #[test]
    fn the_command_line_names_the_shape_to_start_drawn_at() {
        assert_eq!(
            play_args(&["--aspect", "16:9", "c"]).unwrap().aspect,
            Aspect::Shape(16, 9)
        );
        assert_eq!(
            play_args(&["--aspect=4:3", "c"]).unwrap().aspect,
            Aspect::Shape(4, 3)
        );
        assert_eq!(
            play_args(&["--aspect", "235:100", "c"]).unwrap().aspect,
            Aspect::Shape(235, 100)
        );
        assert_eq!(
            play_args(&["--aspect", "fill", "c"]).unwrap().aspect,
            Aspect::Fill
        );
        assert_eq!(
            play_args(&["--aspect", "default", "c"]).unwrap().aspect,
            Aspect::Source
        );
        assert_eq!(play_args(&["c"]).unwrap().aspect, Aspect::Source);
        for words in [
            vec!["--aspect", "16"],
            vec!["--aspect", "0:1"],
            vec!["--aspect", "1001:100"],
            vec!["--aspect", "16:"],
            vec!["--aspect", "wide"],
            vec!["--aspect"],
            vec!["--aspect=2:3:4"],
        ] {
            assert!(
                play_args(&[words.as_slice(), &["c"]].concat()).is_err(),
                "{words:?} is not an aspect"
            );
        }
    }

    /// The rate is written whole whenever it comes within a twentieth of a frame
    /// of one, which is how close the NTSC pull-down is: 41.7 ms per frame reads
    /// as `24`, and only a rate standing further off keeps its decimals. A
    /// stream with no period has no rate at all.
    #[test]
    fn a_frame_rate_is_whole_when_it_rounds_to_whole() {
        assert_eq!(fps_text(Duration::from_millis(40)), "25");
        assert_eq!(fps_text(Duration::from_nanos(41_708_333)), "24");
        assert_eq!(fps_text(Duration::from_nanos(42_553_191)), "23.50");
        assert_eq!(fps_text(Duration::ZERO), "0");
    }

    /// Sizes are counted by a thousand, the way a disk's capacity is named, and
    /// keep a decimal only while the number has a single digit to give.
    #[test]
    fn a_file_size_takes_the_largest_unit_it_fills() {
        assert_eq!(byte_size(812), "812 B");
        assert_eq!(byte_size(1_000), "1.0 kB");
        assert_eq!(byte_size(9_440_000), "9.4 MB");
        assert_eq!(byte_size(12_345_678), "12 MB");
        assert_eq!(byte_size(4_500_000_000_000), "4.5 TB");
    }

    /// The rate a whole file adds up to needs both of its numbers, so a timeline
    /// the container never stated leaves the panel without one.
    #[test]
    fn a_bitrate_needs_a_size_and_a_length() {
        assert_eq!(
            bitrate_text(1_000_000, Duration::from_secs(2)).as_deref(),
            Some("4.0 Mb/s")
        );
        assert_eq!(
            bitrate_text(16_000, Duration::from_secs(1)).as_deref(),
            Some("128 kb/s")
        );
        assert_eq!(bitrate_text(16_000, Duration::ZERO), None);
    }

    /// The size is read as the item opens, from the path it came by; one the
    /// file system no longer answers for is left out of the panel.
    #[test]
    fn a_size_comes_from_the_path_and_nothing_else() {
        let directory = scratch("fvid-player-info-size", &[]);
        let path = directory.join("size.bin");
        std::fs::write(&path, b"twelve bytes").unwrap();
        assert_eq!(file_size(&path), Some(12));
        assert_eq!(file_size(&directory.join("absent.bin")), None);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The panel says what the open item is in the player's own words: the
    /// picture names the codec its container tagged, the size it is stored at
    /// and the rate it runs at; a track names its place in the file's own list;
    /// the file names its size and the rate that size adds up to over its
    /// timeline.
    /// `tests/fixtures/tracks/named.mp4`: 64×64 H.264 at 25 fps over less than a
    /// second, three audio tracks and two subtitle tracks.
    #[test]
    fn the_panel_lists_what_the_open_item_carries() {
        let directory = scratch("fvid-player-info-panel", &[]);
        let path = directory.join("named.mp4");
        std::fs::write(&path, include_bytes!("../tests/fixtures/tracks/named.mp4")).unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert!(!player.info, "the panel starts closed");
        player.apply(Control::Info);
        // On macOS the H.264 in this fixture opens a VideoToolbox session, and
        // the panel says so; which of the two this machine gets is the flag's
        // own answer, so the panel is held to it rather than to one build.
        let decoder = if player.hardware {
            " · VideoToolbox"
        } else {
            ""
        };
        assert_eq!(
            player.info_lines(),
            [
                format!("Video: H.264 · 64×64 · 25 fps{decoder}"),
                "Sound: 1/3 · AAC · Первая · 1 ch 48000 Hz".to_owned(),
                "Subtitles: 1/2 · Титры".to_owned(),
                "File: 21 kB · 0:00 · 413 kb/s".to_owned(),
            ],
            "{:?}",
            player.info_lines()
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A Matroska item names the codec its own track types carry, and the parts
    /// its container lists for it come last.
    /// `tests/fixtures/chapters/chapters.mkv`: 16×16 VP9 at four frames a second
    /// over four seconds, no sound, three named parts.
    #[test]
    fn a_matroska_item_names_its_own_codec_and_parts() {
        let directory = scratch("fvid-player-info-matroska", &[]);
        let path = directory.join("chapters.mkv");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/chapters/chapters.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(
            player.info_lines(),
            [
                "Video: VP9 · 16×16 · 4 fps",
                "File: 1.2 kB · 0:04 · 2 kb/s",
                "Chapters: 3",
            ]
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The name a container gives the file as a whole leads the panel, in both
    /// families and for an item that is nothing but sound: the same walk that
    /// finds the file's parts finds its name.
    /// ```text
    /// ffmpeg -f lavfi -i testsrc=size=16x16:rate=4:duration=1 \
    ///   -metadata title="Полное имя элемента" -c:v libx264 -pix_fmt yuv420p -an \
    ///   tests/fixtures/tags/title.mp4
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000:duration=1 \
    ///   -metadata title="Песня без картинки" -c:a flac -vn tests/fixtures/tags/song.mkv
    /// ```
    /// The first is 16×16 H.264 at four frames a second over one second, named by
    /// its `moov/udta`; the second is one second of FLAC with no picture at all,
    /// named the same way. `tests/fixtures/tags/title.mkv` — the same picture in
    /// VP9, named by its information block — is the file `container::webm`
    /// documents and reads.
    #[test]
    fn the_panel_leads_with_the_name_the_container_gives_the_file() {
        let directory = scratch("fvid-player-info-name", &[]);
        let panel = |name: &str, bytes: &[u8]| -> Vec<String> {
            let path = directory.join(name);
            std::fs::write(&path, bytes).unwrap();
            let mut player = Player {
                queue: vec![path],
                ..Default::default()
            };
            player.play_index(0);
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            let lines = player.info_lines();
            drop(player);
            lines
        };
        for (name, bytes, title) in [
            (
                "title.mp4",
                &include_bytes!("../tests/fixtures/tags/title.mp4")[..],
                "Полное имя элемента",
            ),
            (
                "title.mkv",
                &include_bytes!("../tests/fixtures/tags/title.mkv")[..],
                "Имя из контейнера",
            ),
            (
                "song.mkv",
                &include_bytes!("../tests/fixtures/tags/song.mkv")[..],
                "Песня без картинки",
            ),
        ] {
            assert_eq!(panel(name, bytes)[0], format!("Name: {title}"), "{name}");
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The rest of what a container states about the file as a whole stands in
    /// the panel beside its name, in the order the dialog these facts come by
    /// lists them — in both families, whose writers spell the same six facts
    /// their own way.
    /// ```text
    /// ffmpeg -f lavfi -i testsrc=size=16x16:rate=4:duration=1 \
    ///   -metadata title=T -metadata artist=A -metadata album=B \
    ///   -metadata genre=G -metadata date=2026 -metadata comment=C \
    ///   -c:v libx264 -pix_fmt yuv420p -an tests/fixtures/tags/tags.mp4
    /// ```
    /// `tests/fixtures/tags/tags.mkv` carries the same six facts on the same
    /// picture in VP9, by the command `container::webm` documents.
    #[test]
    fn the_panel_keeps_every_fact_the_container_states_about_the_file() {
        let directory = scratch("fvid-player-info-tags", &[]);
        for (name, bytes) in [
            (
                "tags.mp4",
                &include_bytes!("../tests/fixtures/tags/tags.mp4")[..],
            ),
            (
                "tags.mkv",
                &include_bytes!("../tests/fixtures/tags/tags.mkv")[..],
            ),
        ] {
            let path = directory.join(name);
            std::fs::write(&path, bytes).unwrap();
            let mut player = Player {
                queue: vec![path],
                ..Default::default()
            };
            player.play_index(0);
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            let lines = player.info_lines();
            assert_eq!(
                lines[..6],
                [
                    "Name: T",
                    "Artist: A",
                    "Album: B",
                    "Genre: G",
                    "Date: 2026",
                    "Comment: C",
                ],
                "{name}"
            );
            drop(player);
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A file whose container names nothing has no line to lead with: the panel
    /// starts at its picture, and the name the file carries where it lies stays
    /// where the player has always shown it, above the panel.
    /// `tests/fixtures/chapters/chapters.mkv` states no title at all.
    #[test]
    fn an_unnamed_item_opens_its_panel_with_its_picture() {
        let directory = scratch("fvid-player-info-unnamed", &[]);
        let path = directory.join("chapters.mkv");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/chapters/chapters.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.file_tags, FileTags::default());
        assert_eq!(player.info_lines()[0], "Video: VP9 · 16×16 · 4 fps");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A file muxed to be drawn twice as wide as it is stored says so on its own
    /// line: the shape comes from the container and follows the stored size the
    /// pixels are measured in.
    /// `tests/fixtures/display/par-2x1.webm`: 64×64 VP9 at 25 fps, pixels 2:1.
    #[test]
    fn an_anamorphic_item_names_the_shape_of_its_pixels() {
        let directory = scratch("fvid-player-info-anamorphic", &[]);
        let path = directory.join("par-2x1.webm");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/display/par-2x1.webm"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(
            player.info_lines()[0],
            "Video: VP9 · 64×64 · 25 fps · pixels 2:1"
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The panel is not a message that fades away: it holds the corner it is
    /// drawn in open for as long as it is up, the way a paused item does, and the
    /// same key that opened it closes it.
    #[test]
    fn the_panel_stays_until_the_key_closes_it() {
        let directory = scratch("fvid-player-info-keeps", &[]);
        let path = directory.join("named.mp4");
        std::fs::write(&path, include_bytes!("../tests/fixtures/tracks/named.mp4")).unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        player.apply(Control::Info);
        player.activity = Instant::now() - HIDE_AFTER;
        assert!(player.controls_visible(), "an open panel holds the corner");
        player.apply(Control::Info);
        assert!(!player.info);
        assert!(!player.controls_visible(), "a closed panel lets it fade");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// An item with nothing to show has no picture to name, so the panel starts
    /// at the sound it is playing and stops at the file around it.
    /// `tests/fixtures/audio/flac-stereo.mkv`: two channels at 44.1 kHz over two
    /// seconds, and no video track at all.
    #[test]
    fn a_sound_only_item_is_described_by_its_track() {
        let directory = scratch("fvid-player-info-sound-only", &[]);
        let path = directory.join("flac-stereo.mkv");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/audio/flac-stereo.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.video_codec, "");
        let lines = player.info_lines();
        assert_eq!(
            lines,
            [
                "Sound: 1/1 · FLAC · 2 ch 44100 Hz",
                "File: 105 kB · 0:02 · 415 kb/s",
            ]
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// One coding, however many containers spell it: the table gives the name a
    /// viewer expects for every tag this build has a decoder for, and a coding the
    /// table does not know keeps the name its own file gave it.
    #[test]
    fn every_sound_this_build_decodes_has_the_name_a_viewer_gives_it() {
        for (tag, named) in [
            ("mp4a", "AAC"),
            ("A_VORBIS", "Vorbis"),
            ("A_FLAC", "FLAC"),
            ("A_MPEG/L3", "MP3"),
            ("A_MPEG/L2", "MP2"),
            ("A_ALAC", "ALAC"),
            ("alac", "ALAC"),
            ("A_AC3", "Dolby Digital"),
            ("ac-3", "Dolby Digital"),
            ("A_PCM/INT/LIT", "PCM"),
            ("A_PCM/INT/BIG", "PCM"),
            ("A_PCM/FLOAT/IEEE", "PCM"),
            ("sowt", "PCM"),
            ("twos", "PCM"),
            ("fl32", "PCM"),
            ("fl64", "PCM"),
            ("pcm_alaw", "G.711 (a-law)"),
            ("pcm_mulaw", "G.711 (mu-law)"),
            ("adpcm_ms", "ADPCM (MS)"),
            ("adpcm_ima_wav", "ADPCM (IMA)"),
            ("adpcm_ima_qt", "ADPCM (IMA)"),
            ("midi", "MIDI"),
            ("xm", "XM"),
            (
                "A_THE_PLACE_NO_READER_COMES_FROM",
                "A_THE_PLACE_NO_READER_COMES_FROM",
            ),
        ] {
            assert_eq!(sound_codec(tag), named, "{tag}");
        }
    }

    /// The panel names the coding the sound on screen is actually reading, and
    /// the track key changes that name together with the sound.
    /// `tests/fixtures/audio/ac3-flac-mp3.mkv`: an AC-3 track, then FLAC and MP3.
    /// All three are in the list now that the reader hands `A_AC3` to the decoder,
    /// so the key walks the panel from Dolby Digital through the two lossy ones and
    /// the count it prints is the three the file offers.
    #[test]
    fn the_panel_names_the_codec_and_the_track_key_keeps_it_current() {
        let directory = scratch("fvid-player-sound-codec", &[]);
        let path = directory.join("ac3-flac-mp3.mkv");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/audio/ac3-flac-mp3.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        let sound = |player: &Player| {
            player
                .info_lines()
                .into_iter()
                .find(|line| line.starts_with("Sound: "))
                .unwrap_or_default()
        };
        assert!(
            sound(&player).starts_with("Sound: 1/3 · Dolby Digital · "),
            "{}",
            sound(&player)
        );
        player.cycle_audio_track(true);
        assert_eq!(
            (player.audio_track, player.sound_codec.as_str()),
            (1, "FLAC")
        );
        player.cycle_audio_track(true);
        assert_eq!(
            (player.audio_track, player.sound_codec.as_str()),
            (2, "MP3")
        );
        assert!(
            sound(&player).starts_with("Sound: 3/3 · MP3 · "),
            "{}",
            sound(&player)
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// One fact, two writings: ffmpeg puts `track=3/12` into MP4 as a binary
    /// `trkn` pair and into Matroska as the text `3/12` under `PART_NUMBER`,
    /// and the panel answers both the same way, with the track standing right
    /// after the album it is a place in.
    /// ```text
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000:duration=1 \
    ///   -metadata title=T -metadata artist=A -metadata album=B -metadata track=3/12 \
    ///   -c:a aac tests/fixtures/tags/track.mp4
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000:duration=1 \
    ///   -metadata title=T -metadata artist=A -metadata album=B -metadata track=3/12 \
    ///   -c:a flac tests/fixtures/tags/track.mkv
    /// ```
    #[test]
    fn the_panel_lists_the_place_a_file_names_in_its_album() {
        for (name, bytes) in [
            (
                "track.mp4",
                &include_bytes!("../tests/fixtures/tags/track.mp4")[..],
            ),
            (
                "track.mkv",
                &include_bytes!("../tests/fixtures/tags/track.mkv")[..],
            ),
        ] {
            let directory = scratch("fvid-player-info-track", &[]);
            let path = directory.join(name);
            std::fs::write(&path, bytes).unwrap();
            let mut player = Player {
                queue: vec![path],
                ..Default::default()
            };
            player.play_index(0);
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            let lines = player.info_lines();
            assert_eq!(
                lines[..4],
                ["Name: T", "Artist: A", "Album: B", "Track: 3/12",],
                "{name}"
            );
            drop(player);
            std::fs::remove_dir_all(&directory).unwrap();
        }
    }

    /// One round of ffmpeg's `-metadata` with the album's own facts, the two
    /// containers parting ways exactly as their formats do:
    /// ```text
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000:duration=1 \
    ///   -metadata title=T -metadata album_artist=AA -metadata disc=2/10 \
    ///   -metadata publisher=PB -c:a aac tests/fixtures/tags/band.mp4
    /// ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000:duration=1 \
    ///   -metadata title=T -metadata album_artist=AA -metadata disc=2/10 \
    ///   -metadata publisher=PB -c:a flac tests/fixtures/tags/band.mkv
    /// ```
    /// MP4 has no box for a publisher and the muxer drops that fact on the
    /// floor; Matroska keeps every word it was handed. The disc's number-pair
    /// arrives in both, the binary writing in one file and the text in the
    /// other, and the panel places each fact between the lines it belongs to.
    #[test]
    fn the_panel_names_the_album_its_artist_its_disc_and_its_publisher() {
        for (name, bytes, facts) in [
            (
                "band.mp4",
                &include_bytes!("../tests/fixtures/tags/band.mp4")[..],
                ["Name: T", "Album artist: AA", "Disc: 2/10"].as_slice(),
            ),
            (
                "band.mkv",
                &include_bytes!("../tests/fixtures/tags/band.mkv")[..],
                ["Name: T", "Album artist: AA", "Disc: 2/10", "Publisher: PB"].as_slice(),
            ),
        ] {
            let directory = scratch("fvid-player-info-album", &[]);
            let path = directory.join(name);
            std::fs::write(&path, bytes).unwrap();
            let mut player = Player {
                queue: vec![path],
                ..Default::default()
            };
            player.play_index(0);
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            let lines = player.info_lines();
            for fact in facts {
                assert!(lines.contains(&fact.to_string()), "{name}: {lines:?}");
            }
            if name.ends_with(".mp4") {
                assert!(
                    !lines.iter().any(|l| l.starts_with("Publisher")),
                    "a fact the writer dropped cannot be shown: {lines:?}"
                );
            }
            drop(player);
            std::fs::remove_dir_all(&directory).unwrap();
        }
    }

    /// Two facts one writer states and the other forgets: ffmpeg answers
    /// `-metadata copyright` and `-metadata description` in both containers,
    /// filing them in MP4 as the unmarked `cprt` and `desc` atoms and in
    /// Matroska under their plain names, and the panel shows each where the
    /// file stated it.
    /// `tests/fixtures/tags/rights.mp4` and `rights.mkv`:
    /// ffmpeg -f lavfi -i anullsrc=r=8000:cl=mono -t 0.2 \
    ///   -metadata title=T -metadata copyright='2026 The Holder' \
    ///   -metadata description='A note' -c:a aac tests/fixtures/tags/rights.mp4
    /// (the same line with `-c:a flac` writes the `.mkv`), and ffprobe reads
    /// both facts back from each.
    #[test]
    fn the_panel_shows_the_rights_and_the_note_where_the_file_stated_them() {
        for (name, bytes) in [
            (
                "rights.mp4",
                &include_bytes!("../tests/fixtures/tags/rights.mp4")[..],
            ),
            (
                "rights.mkv",
                &include_bytes!("../tests/fixtures/tags/rights.mkv")[..],
            ),
        ] {
            let directory = scratch("fvid-player-info-rights", &[]);
            let path = directory.join(name);
            std::fs::write(&path, bytes).unwrap();
            let mut player = Player {
                queue: vec![path],
                ..Default::default()
            };
            player.play_index(0);
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            let lines = player.info_lines();
            for fact in [
                "Name: T",
                "Copyright: 2026 The Holder",
                "Description: A note",
            ] {
                assert!(lines.contains(&fact.to_string()), "{name}: {lines:?}");
            }
            drop(player);
            std::fs::remove_dir_all(&directory).unwrap();
        }
    }

    /// The one fact only one of the two containers carries: ffmpeg answers
    /// `-metadata rating` in Matroska under that plain name and drops it from
    /// an MP4 without a word, and the panel shows the rating where the file
    /// kept it and not otherwise. `tests/fixtures/tags/rated.mkv` and
    /// `rated.mp4`:
    /// ffmpeg -f lavfi -i anullsrc=r=8000:cl=mono -t 0.2 \
    ///   -metadata title=T -metadata rating=5 -c:a flac tests/fixtures/tags/rated.mkv
    /// (the same line with `-c:a aac` writes the `.mp4`), and ffprobe reads the
    /// rating back from the Matroska file and finds no trace of it in the MP4.
    #[test]
    fn the_panel_shows_the_rating_the_matroska_file_was_given() {
        for (name, bytes, stated) in [
            (
                "rated.mkv",
                &include_bytes!("../tests/fixtures/tags/rated.mkv")[..],
                true,
            ),
            (
                "rated.mp4",
                &include_bytes!("../tests/fixtures/tags/rated.mp4")[..],
                false,
            ),
        ] {
            let directory = scratch("fvid-player-info-rating", &[]);
            let path = directory.join(name);
            std::fs::write(&path, bytes).unwrap();
            let mut player = Player {
                queue: vec![path],
                ..Default::default()
            };
            player.play_index(0);
            assert!(player.error.is_none(), "{name}: {:?}", player.error);
            let lines = player.info_lines();
            assert!(lines.contains(&"Name: T".to_string()), "{name}: {lines:?}");
            assert_eq!(
                lines.contains(&"Rating: 5".to_string()),
                stated,
                "{name}: {lines:?}"
            );
            drop(player);
            std::fs::remove_dir_all(&directory).unwrap();
        }
    }

    /// With no item on screen there is nothing to describe, so the key says so
    /// instead of opening an empty panel over the stage.
    #[test]
    fn the_information_key_has_nothing_to_name() {
        let mut player = Player::default();
        player.apply(Control::Info);
        assert!(!player.info);
        assert_eq!(player.osd.as_ref().unwrap().0, "Nothing open");
    }

    fn subtitled() -> Player {
        Player {
            cues: subtitles::parse(
                "1\n00:00:01,000 --> 00:00:02,000\n<i>Hi</i>\n\n2\n00:00:03,000 --> 00:00:04,000\nThere\n",
            ),
            ..Default::default()
        }
    }

    fn seconds(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn cues_are_live_inside_their_span() {
        let player = subtitled();
        assert_eq!(player.subtitle_line(seconds(1_000)), Some("Hi"));
        assert_eq!(player.subtitle_line(seconds(1_999)), Some("Hi"));
        assert_eq!(player.subtitle_line(seconds(2_000)), None);
        assert_eq!(player.subtitle_line(seconds(3_500)), Some("There"));
    }

    /// The delay key moves the whole track, so a later cue is asked for what
    /// the file held earlier; pressing it back past zero shows nothing before
    /// the first cue rather than wrapping around.
    #[test]
    fn the_delay_key_shifts_the_cue_timeline() {
        let mut player = subtitled();
        player.apply(Control::SubtitleDelay(true));
        assert_eq!(player.subtitle_delay_ms, 50);
        assert_eq!(player.subtitle_line(seconds(1_000)), None);
        assert_eq!(player.subtitle_line(seconds(1_050)), Some("Hi"));

        player.apply(Control::SubtitleDelay(false));
        player.apply(Control::SubtitleDelay(false));
        assert_eq!(player.subtitle_delay_ms, -50);
        assert_eq!(player.subtitle_line(seconds(950)), Some("Hi"));

        player.apply(Control::SubtitleDelay(true));
        player.apply(Control::SubtitleDelay(true));
        assert_eq!(player.subtitle_delay_ms, 50);
        assert_eq!(player.subtitle_line(seconds(0)), None);
    }

    #[test]
    fn subtitles_toggle_and_survive_an_empty_track() {
        let mut player = Player::default();
        player.apply(Control::Subtitles);
        assert_eq!(player.osd.as_ref().unwrap().0, "No subtitles");
        assert!(player.subtitle_shown);

        player = subtitled();
        player.apply(Control::Subtitles);
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles off");
        assert_eq!(player.subtitle_line(seconds(1_000)), None);
        player.apply(Control::Subtitles);
        assert_eq!(player.subtitle_line(seconds(1_000)), Some("Hi"));
    }

    /// The delay key is bounded at plus and minus ten seconds, so a track
    /// pushed past its end stays where it is rather than running away.
    #[test]
    fn the_delay_key_is_bounded_at_ten_seconds() {
        assert_eq!(delay_step(0, true), 50);
        assert_eq!(delay_step(0, false), -50);
        assert_eq!(delay_step(10_000, true), 10_000);
        assert_eq!(delay_step(-10_000, false), -10_000);
    }

    /// A file beside the video is found on its own, without being asked for.
    #[test]
    fn a_video_picks_up_the_sidecar_beside_it() {
        let directory = std::env::temp_dir().join("fvid-player-subtitle-sidecar");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("clip.webm"), b"not really a video").unwrap();
        std::fs::write(
            directory.join("clip.srt"),
            "1\n00:00:01,000 --> 00:00:02,000\nHello\n",
        )
        .unwrap();

        let mut player = Player::default();
        player.load_subtitles(&directory.join("clip.webm"));
        assert_eq!(player.subtitle_name, "clip.srt");
        assert_eq!(
            player.subtitle_line(Duration::from_millis(1_500)),
            Some("Hello")
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A file the viewer names joins the captions the video came with and is
    /// turned to at once, since it was asked for after everything else.
    #[test]
    fn a_named_file_joins_the_captions_of_the_open_item() {
        let directory = scratch("fvid-player-subtitle-load", &[]);
        std::fs::write(
            directory.join("first.srt"),
            "1\n00:00:01,000 --> 00:00:02,000\nEarly\n",
        )
        .unwrap();
        std::fs::write(
            directory.join("later.srt"),
            "1\n00:00:05,000 --> 00:00:06,000\nLate\n",
        )
        .unwrap();

        let mut player = Player::default();
        player.subtitle_sources.push(SubtitleSource {
            label: "first.srt".to_owned(),
            cues: subtitles::parse("1\n00:00:01,000 --> 00:00:02,000\nEarly\n"),
        });
        player.apply_subtitle_source();
        player.add_subtitle_file(directory.join("later.srt"));
        assert_eq!(
            player
                .subtitle_sources
                .iter()
                .map(|source| source.label.as_str())
                .collect::<Vec<_>>(),
            ["first.srt", "later.srt"]
        );
        assert_eq!(
            (player.subtitle_source, player.subtitle_name.as_str()),
            (1, "later.srt")
        );
        assert!(player.subtitle_shown);
        assert_eq!(player.subtitle_line(seconds(5_500)), Some("Late"));
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles: later.srt");
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A file nothing can be read from leaves the captions as they were and
    /// says so, the way a `--sub-file` that yields nothing does.
    #[test]
    fn a_file_that_captions_nothing_leaves_the_captions_alone() {
        let mut player = subtitled();
        player.subtitle_sources.push(SubtitleSource {
            label: "the-file.srt".to_owned(),
            cues: player.cues.clone(),
        });
        player.apply_subtitle_source();
        let before = player.subtitle_name.clone();
        player.add_subtitle_file(std::env::temp_dir().join("fvid-no-such-subtitles.srt"));
        assert_eq!(player.subtitle_sources.len(), 1);
        assert_eq!(player.subtitle_name, before);
        assert_eq!(player.subtitle_line(seconds(1_500)), Some("Hi"));
        assert!(
            player
                .osd
                .as_ref()
                .unwrap()
                .0
                .starts_with("Cannot read subtitles"),
            "{:?}",
            player.osd
        );
    }

    /// An item that is not open has nothing to be captioned by, so the load
    /// key says so rather than opening a picker whose answer would be dropped.
    #[test]
    fn the_load_key_has_nothing_to_caption() {
        let mut player = Player::default();
        player.apply(Control::LoadSubtitles);
        assert!(player.dialog.is_none());
        assert_eq!(player.osd.as_ref().unwrap().0, "Nothing to caption");
    }

    #[test]
    fn subtitle_size_and_margin_have_ends() {
        let mut player = subtitled();
        for _ in 0..40 {
            player.apply(Control::SubtitleSize(true));
            player.apply(Control::SubtitleMargin(true));
        }
        assert_eq!(player.subtitle_font, 72.0);
        assert_eq!(player.subtitle_margin, 8.0);
        for _ in 0..80 {
            player.apply(Control::SubtitleSize(false));
            player.apply(Control::SubtitleMargin(false));
        }
        assert_eq!(player.subtitle_font, 10.0);
        assert_eq!(player.subtitle_margin, 400.0);
    }

    /// A temporary directory holding `files`, for the list tests.
    fn scratch(name: &str, files: &[&str]) -> PathBuf {
        let directory = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        for file in files {
            std::fs::write(directory.join(file), b"x").unwrap();
        }
        directory
    }

    #[test]
    fn the_list_advances_until_the_end_and_then_per_repetition() {
        // Three items: forward stops past the last one unless the list repeats.
        assert_eq!(advance(0, 3, Repeat::Off), Some(1));
        assert_eq!(advance(2, 3, Repeat::Off), None);
        assert_eq!(advance(2, 3, Repeat::All), Some(0));
        assert_eq!(advance(0, 0, Repeat::All), None);
        assert_eq!(retreat(1, 3, Repeat::Off), Some(0));
        assert_eq!(retreat(0, 3, Repeat::Off), None);
        assert_eq!(retreat(0, 3, Repeat::All), Some(2));
        assert_eq!(playlist_osd(0, 3), "1/3");
        assert_eq!(playlist_osd(2, 3), "3/3");
    }

    /// One dealt cycle holds every item of the list but the one on screen, in
    /// some order; the same seed always deals the same order and another seed
    /// deals another, while a list of one has nothing to deal at all.
    #[test]
    fn a_cycle_is_dealt_over_the_whole_list_once() {
        let mut seed = 0x243f_6a88_85a3_08d3;
        let cycle = deal_cycle(5, 2, &mut seed);
        let mut sorted = cycle.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, [0, 1, 3, 4]);
        let mut same = 0x243f_6a88_85a3_08d3;
        assert_eq!(deal_cycle(5, 2, &mut same), cycle);
        let mut other = 0x8def_8291_d712_b3bf;
        assert_ne!(deal_cycle(5, 2, &mut other), cycle);
        let mut empty = 7;
        assert!(deal_cycle(1, 0, &mut empty).is_empty());
        assert!(deal_cycle(0, 0, &mut empty).is_empty());
    }

    /// With Random on the next key walks the drawn cycle: every other item of
    /// the list plays exactly once, and a list that does not repeat then says
    /// it is at the end. The previous key returns along the walked steps, and
    /// the item stepped past stands open for the draw again. A repeating list
    /// deals a fresh cycle where the old one ran out, and turning Random off
    /// leaves the plain order to resume from the item on screen.
    #[test]
    fn the_shuffled_list_walks_its_cycle_and_returns_its_steps() {
        let directory = scratch("fvid-player-shuffle", &[]);
        for name in ["one.y4m", "two.y4m", "three.y4m", "four.y4m"] {
            std::fs::write(directory.join(name), y4m(1)).unwrap();
        }
        let mut player = Player {
            queue: ["one", "two", "three", "four"]
                .map(|name| directory.join(format!("{name}.y4m")))
                .into_iter()
                .collect(),
            ..Default::default()
        };
        player.play_index(0);
        assert_eq!(player.name, "one.y4m");
        player.apply(Control::Shuffle);
        assert_eq!(player.osd.as_ref().unwrap().0, "Shuffle on");
        let mut drawn = Vec::new();
        for _ in 0..3 {
            player.apply(Control::Next);
            drawn.push(player.name.clone());
        }
        let mut seen = drawn.clone();
        seen.push("one.y4m".to_owned());
        seen.sort();
        assert_eq!(
            seen,
            ["four.y4m", "one.y4m", "three.y4m", "two.y4m"],
            "{drawn:?}"
        );
        player.apply(Control::Next);
        assert_eq!(player.osd.as_ref().unwrap().0, "End of playlist");
        player.apply(Control::Previous);
        assert_eq!(player.name, drawn[1]);
        player.apply(Control::Next);
        assert_eq!(player.name, drawn[2]);
        player.repeat = Repeat::All;
        player.apply(Control::Next);
        assert_ne!(player.name, drawn[2]);
        player.apply(Control::Shuffle);
        assert_eq!(player.osd.as_ref().unwrap().0, "Shuffle off");
        let before = player.index;
        player.apply(Control::Next);
        assert_eq!(player.index, (before + 1) % 4);
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A shuffled list continues by itself along the cycle: an item running
    /// out starts the next draw, not the next number, and the list ends when
    /// the cycle has played everything once.
    #[test]
    fn a_shuffled_item_running_out_walks_the_cycle() {
        let directory = scratch("fvid-player-shuffle-auto", &[]);
        for name in ["one.y4m", "two.y4m", "three.y4m"] {
            std::fs::write(directory.join(name), y4m(1)).unwrap();
        }
        let mut player = Player {
            queue: ["one", "two", "three"]
                .map(|name| directory.join(format!("{name}.y4m")))
                .into_iter()
                .collect(),
            ..Default::default()
        };
        player.play_index(0);
        player.shuffle = true;
        player.upcoming = deal_cycle(3, 0, &mut 99);
        player.walked = vec![0];
        player.continue_queue();
        let first = player.index;
        assert_ne!(first, 0);
        player.continue_queue();
        let second = player.index;
        assert_ne!(second, first);
        let mut seen = [0, first, second];
        seen.sort_unstable();
        assert_eq!(seen, [0, 1, 2]);
        player.continue_queue();
        assert!(player.ended);
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A session started out with `--random` holds the cycle dealt to the
    /// queue the command line named: the walk begins at the item on screen and
    /// every other item stands open for a draw before the first step is taken.
    #[test]
    fn a_random_start_holds_the_cycle_the_queue_was_dealt() {
        let directory = scratch("fvid-player-random-start", &[]);
        for name in ["one.y4m", "two.y4m", "three.y4m"] {
            std::fs::write(directory.join(name), y4m(1)).unwrap();
        }
        let mut player = Player {
            queue: ["one", "two", "three"]
                .map(|name| directory.join(format!("{name}.y4m")))
                .into_iter()
                .collect(),
            shuffle: true,
            ..Default::default()
        };
        player.start_shuffle();
        assert_eq!(player.walked, [0]);
        assert_eq!(player.upcoming.len(), 2);
        player.play_index(0);
        player.apply(Control::Next);
        assert_ne!(player.index, 0);
        let mut seen = vec![0, player.index];
        player.apply(Control::Next);
        seen.push(player.index);
        seen.sort_unstable();
        assert_eq!(seen, [0, 1, 2]);
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn repeat_cycles_in_vlc_order() {
        let mut repeat = Repeat::default();
        let mut seen = Vec::new();
        for _ in 0..4 {
            repeat = cycle_repeat(repeat);
            seen.push(repeat);
        }
        assert_eq!(seen, [Repeat::All, Repeat::One, Repeat::Off, Repeat::All]);
        assert_eq!(repeat_osd(Repeat::One), "Repeat one");
    }

    /// A list is only as good as the files it points at: comments, notes,
    /// network entries and missing files are passed over, and everything else
    /// is looked for beside the list rather than beside the player.
    #[test]
    fn a_list_names_the_files_that_are_actually_there() {
        let directory = scratch("fvid-player-playlist", &["a.mp4", "b.webm"]);
        let list = directory.join("queue.m3u");
        std::fs::write(
            &list,
            "#EXTM3U\n#EXTINF:10,a\na.mp4\n#EXTINF:20,b\nmissing.mp4\nhttps://example.test/b.webm\n",
        )
        .unwrap();
        let names: Vec<String> = expand_inputs(&[list])
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.mp4"]);

        // A PLS keeps its paths behind numbered keys, next to titles that are
        // not paths at all.
        let pls = directory.join("queue.pls");
        std::fs::write(
            &pls,
            "[playlist]\nFile1=b.webm\nTitle1=B\nNumberOfEntries=1\n",
        )
        .unwrap();
        let entries = expand_inputs(&[pls]);
        assert_eq!(entries, [directory.join("b.webm")]);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn an_unusable_list_is_opened_as_itself_so_the_error_says_why() {
        let directory = scratch("fvid-player-empty-playlist", &[]);
        let list = directory.join("empty.m3u");
        std::fs::write(&list, "#EXTM3U\n").unwrap();
        let inputs = vec![list];
        assert_eq!(expand_inputs(&inputs), inputs);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// Stepping onto a file that will not open leaves the picture where it was,
    /// rather than a blank window with a stale index.
    #[test]
    fn a_step_onto_a_broken_item_keeps_the_picture() {
        let directory = scratch("fvid-player-broken-playlist", &["a.mp4"]);
        let mut player = Player {
            queue: vec![directory.join("a.mp4"), directory.join("gone.mp4")],
            ..Default::default()
        };
        player.play_index(1);
        assert_eq!(player.index, 0);
        assert!(player.error.is_some());

        player.step_queue(false);
        assert_eq!(player.index, 0);

        player.step_queue(true);
        assert_eq!(player.osd.as_ref().unwrap().0, "First item");
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// Two frames of 2×2 4:2:0: the smallest thing the reader accepts.
    fn y4m(frames: usize) -> Vec<u8> {
        let mut bytes = b"YUV4MPEG2 W2 H2 F25:1 Ip C420jpeg\n".to_vec();
        for _ in 0..frames {
            bytes.extend_from_slice(b"FRAME\n");
            bytes.extend_from_slice(&[0u8; 6]);
        }
        bytes
    }

    /// Reaching the end of an item is what moves the list on; the last item of
    /// a list that does not repeat stops on its own picture.
    #[test]
    fn the_end_of_an_item_starts_the_next_one() {
        let directory = scratch("fvid-player-queue", &[]);
        for name in ["one.y4m", "two.y4m"] {
            std::fs::write(directory.join(name), y4m(2)).unwrap();
        }
        let mut player = Player {
            queue: vec![directory.join("one.y4m"), directory.join("two.y4m")],
            ..Default::default()
        };
        player.play_index(0);
        assert_eq!(player.name, "one.y4m");

        player.continue_queue();
        assert_eq!((player.name.as_str(), player.index), ("two.y4m", 1));
        assert!(!player.ended);

        player.continue_queue();
        assert_eq!(player.name, "two.y4m");
        assert!(player.ended);
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn repeating_one_item_reopens_it() {
        let directory = scratch("fvid-player-repeat-one", &[]);
        std::fs::write(directory.join("only.y4m"), y4m(1)).unwrap();
        let mut player = Player {
            queue: vec![directory.join("only.y4m")],
            repeat: Repeat::One,
            ..Default::default()
        };
        player.play_index(0);
        player.continue_queue();
        assert_eq!(
            (player.name.as_str(), player.index, player.ended),
            ("only.y4m", 0, false)
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A file with two AAC tracks behind one video: the list comes from the
    /// container, the key moves the sound onto the other track, and the picture
    /// is left alone. Reaching the end of the list wraps back to the first track.
    #[test]
    fn a_file_with_two_audio_tracks_walks_between_them() {
        let directory = scratch("fvid-player-audio-tracks", &[]);
        let path = directory.join("two-audio.mp4");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/audio/two-audio.mp4"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!((player.audio_tracks.len(), player.audio_track), (2, 0));

        player.apply(Control::AudioTrack(true));
        assert_eq!(player.audio_track, 1);
        assert_eq!(player.osd.as_ref().unwrap().0, "Audio 2/2 1 ch 32000 Hz");
        assert_eq!(player.name, "two-audio.mp4");

        player.apply(Control::AudioTrack(true));
        assert_eq!(player.audio_track, 0);
        assert_eq!(player.osd.as_ref().unwrap().0, "Audio 1/2 1 ch 48000 Hz");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The named MP4, whose three audio tracks say different amounts about
    /// themselves: the key quotes the title for the first, the language for the
    /// second and nothing but the layout for the third, and the subtitle list
    /// does the same for the one track the muxer titled.
    /// `tests/fixtures/tracks/named.mp4`, whose command `tests/mp4.rs` records.
    #[test]
    fn the_audio_list_says_what_the_file_says_about_each_track() {
        let directory = scratch("fvid-player-named-tracks", &[]);
        let path = directory.join("named.mp4");
        std::fs::write(&path, include_bytes!("../tests/fixtures/tracks/named.mp4")).unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.audio_tracks.len(), 3);

        player.apply(Control::AudioTrack(true));
        assert_eq!(
            player.osd.as_ref().unwrap().0,
            "Audio 2/3 fre · 1 ch 32000 Hz"
        );
        player.apply(Control::AudioTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Audio 3/3 1 ch 44100 Hz");
        player.apply(Control::AudioTrack(true));
        assert_eq!(
            player.osd.as_ref().unwrap().0,
            "Audio 1/3 Первая · 1 ch 48000 Hz"
        );

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 2/2 mov_text");
        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 1/2 Титры");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The same two lists out of a Matroska file, where the title and the
    /// language are elements of their own and the picture track, which states
    /// only that nothing was said, is left out of both.
    /// `tests/fixtures/tracks/named.mkv`, whose command the container's own test
    /// records.
    #[test]
    fn the_matroska_lists_say_what_the_file_says_about_each_track() {
        let directory = scratch("fvid-player-named-matroska-tracks", &[]);
        let path = directory.join("named.mkv");
        std::fs::write(&path, include_bytes!("../tests/fixtures/tracks/named.mkv")).unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(
            (player.audio_tracks.len(), player.subtitle_sources.len()),
            (2, 2)
        );

        player.apply(Control::AudioTrack(true));
        assert_eq!(
            player.osd.as_ref().unwrap().0,
            "Audio 2/2 fre · 1 ch 32000 Hz"
        );
        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 2/2 UTF-8");
        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 1/2 Титры");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// Two text tracks behind one picture: opening the file finds both, the key
    /// moves the text onto the other one, and the list wraps at either end.
    #[test]
    fn a_file_with_two_subtitle_tracks_walks_between_them() {
        let directory = scratch("fvid-player-subtitle-tracks", &[]);
        let path = directory.join("text-tracks.mkv");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/subtitles/text-tracks.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.subtitle_sources.len(), 2);
        assert_eq!(player.subtitle_line(seconds(700)), Some("plain first"));

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.subtitle_source, 1);
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 2/2 UTF-8");
        assert_eq!(player.subtitle_line(seconds(700)), Some("other track"));
        assert_eq!(player.subtitle_line(seconds(1_400)), Some("tagged words"));

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.subtitle_source, 0);
        player.apply(Control::SubtitleTrack(false));
        assert_eq!(player.subtitle_source, 1);
        assert_eq!(player.name, "text-tracks.mkv");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// An ASS track of the file itself, in the window: the list shows it under
    /// the title the file gives it, the line painted is the text of its blocks
    /// with the tags left out, and the key walks it beside the SubRip track of
    /// the same file. `tests/fixtures/subtitles/ass-track.mkv`, whose command
    /// the reader's own test records.
    #[test]
    fn an_ass_track_of_the_file_is_listed_painted_and_walked() {
        let directory = scratch("fvid-player-ass-track", &[]);
        let path = directory.join("ass-track.mkv");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/subtitles/ass-track.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.subtitle_sources.len(), 2);
        // The ASS track is first in container order, so it is also the default.
        assert_eq!(
            player.subtitle_line(seconds(700)),
            Some("First\nsecond line")
        );
        assert_eq!(
            player.subtitle_line(seconds(2_500)),
            Some("Hello, world with commas")
        );

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.subtitle_source, 1);
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 2/2 UTF-8");
        assert_eq!(player.subtitle_line(seconds(1_500)), Some("One, two"));

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.subtitle_source, 0);
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 1/2 События");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The same list built out of an MP4, which the Matroska reader cannot open:
    /// the file's first bytes pick the other reader, and from there the tracks,
    /// the labels and the key behave as they do for a Matroska file.
    #[test]
    fn a_file_with_two_mov_text_tracks_walks_between_them() {
        let directory = scratch("fvid-player-mov-text-tracks", &[]);
        let path = directory.join("mov-text-tracks.mp4");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/subtitles/mov-text-tracks.mp4"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.subtitle_sources.len(), 2);
        assert_eq!(player.subtitle_line(seconds(150)), Some("first line"));

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.subtitle_source, 1);
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 2/2 mov_text");
        // The other track holds nothing at 150 ms and its own two lines later,
        // the last of them stripped of the markup the encoder moved into a box.
        assert_eq!(player.subtitle_line(seconds(150)), None);
        assert_eq!(player.subtitle_line(seconds(250)), Some("other track"));
        assert_eq!(player.subtitle_line(seconds(450)), Some("tagged words"));

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.subtitle_source, 0);
        assert_eq!(player.name, "mov-text-tracks.mp4");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A sidecar the user placed is the first choice; the file's own tracks stay
    /// behind it in the list rather than replacing it.
    #[test]
    fn a_sidecar_sits_in_front_of_the_tracks_in_the_file() {
        let directory = scratch("fvid-player-subtitle-sidecar-and-tracks", &["clip.srt"]);
        std::fs::write(
            directory.join("clip.mkv"),
            include_bytes!("../tests/fixtures/subtitles/text-tracks.mkv"),
        )
        .unwrap();
        std::fs::write(
            directory.join("clip.srt"),
            "1\n00:00:00,600 --> 00:00:00,800\nfrom the sidecar\n",
        )
        .unwrap();
        let mut player = Player {
            queue: vec![directory.join("clip.mkv")],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.subtitle_sources.len(), 3);
        assert_eq!(player.subtitle_name, "clip.srt");
        assert_eq!(player.subtitle_line(seconds(700)), Some("from the sidecar"));

        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 2/3 UTF-8");
        assert_eq!(player.subtitle_line(seconds(700)), Some("plain first"));
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A SAMI file — the Windows-era caption, where every line carries its own
    /// time in the tag that opens it — is a sidecar like the others, so the line
    /// reaches the screen at the milliseconds its `Begin` and `End` name.
    #[test]
    fn a_sami_sidecar_captions_the_item_beside_which_it_lies() {
        let directory = scratch("fvid-player-sami-sidecar", &[]);
        std::fs::write(
            directory.join("clip.mkv"),
            include_bytes!("../tests/fixtures/subtitles/text-tracks.mkv"),
        )
        .unwrap();
        std::fs::write(
            directory.join("clip.smi"),
            "<SAMI>\n<HEAD><TITLE>Clip</TITLE></HEAD>\n<BODY>\n\
             <p class=eng Begin=500 End=1500>from the sami file\n",
        )
        .unwrap();
        let mut player = Player {
            queue: vec![directory.join("clip.mkv")],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert_eq!(player.subtitle_name, "clip.smi");
        assert_eq!(
            player.subtitle_line(seconds(1_000)),
            Some("from the sami file")
        );
        assert_eq!(
            player.subtitle_line(seconds(2_000)),
            None,
            "the caption's own end is what takes it away"
        );
        // The file's two text tracks are still there, behind it.
        assert_eq!(player.subtitle_sources.len(), 3);
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn the_subtitle_key_needs_a_choice_to_do_anything() {
        let mut player = Player {
            cues: subtitles::parse("1\n00:00:00,500 --> 00:00:01,000\none track\n"),
            subtitle_name: "clip.srt".to_owned(),
            subtitle_sources: vec![SubtitleSource {
                label: "clip.srt".to_owned(),
                cues: subtitles::parse("1\n00:00:00,500 --> 00:00:01,000\none track\n"),
            }],
            ..Default::default()
        };
        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Subtitles 1/1");
        assert_eq!(player.subtitle_source, 0);
        assert_eq!(player.subtitle_line(seconds(700)), Some("one track"));

        let mut player = Player::default();
        player.apply(Control::SubtitleTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "No subtitles");
    }

    #[test]
    fn a_snapshot_is_named_by_the_time_it_was_taken() {
        let video = PathBuf::from("/tmp/clip.mp4");
        assert_eq!(
            snapshot_name(&video, seconds(3_723_456)),
            PathBuf::from("/tmp/clip-01h02m03s456.png")
        );
        assert_eq!(
            snapshot_name(&video, Duration::ZERO),
            PathBuf::from("/tmp/clip-00h00m00s000.png")
        );
    }

    #[test]
    fn a_snapshot_writes_the_picture_on_screen() {
        let directory = scratch("fvid-player-snapshot", &[]);
        let video = directory.join("clip.mp4");
        std::fs::write(&video, b"an mp4 as far as the snapshot cares").unwrap();
        let mut player = Player {
            opened: Some(video),
            presented: Some(Frame {
                pixels: Pixels::Rgb((1..=12).collect()),
                dimensions: [2, 2],
                period: Duration::from_millis(40),
                interval: None,
                pts: None,
                generation: 0,
                serial: 1,
            }),
            ..Default::default()
        };
        player.apply(Control::Snapshot);
        assert_eq!(
            player.osd.as_ref().unwrap().0,
            "Saved clip-00h00m00s000.png"
        );
        let file = std::fs::read(directory.join("clip-00h00m00s000.png")).expect("written");
        assert_eq!(
            &file[..8],
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
        );
        // IHDR starts at the tenth byte with the two 32-bit dimensions.
        assert_eq!(&file[16..24], &[0, 0, 0, 2, 0, 0, 0, 2]);
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_snapshot_converts_planes_before_writing_them() {
        let directory = scratch("fvid-player-snapshot-planes", &[]);
        let video = directory.join("clip.mp4");
        std::fs::write(&video, b"an mp4 as far as the snapshot cares").unwrap();
        let planes = Planar8 {
            width: 2,
            height: 2,
            chroma_width: 1,
            chroma_height: 1,
            y: vec![16, 128, 200, 235],
            cb: vec![128],
            cr: vec![128],
            colour: Default::default(),
        };
        let player = Player {
            opened: Some(video),
            presented: Some(Frame {
                pixels: Pixels::Planar(Arc::new(planes)),
                dimensions: [2, 2],
                period: Duration::from_millis(40),
                interval: None,
                pts: None,
                generation: 0,
                serial: 1,
            }),
            ..Default::default()
        };
        let file = player.picture().expect("planes become a picture");
        assert_eq!(&file[16..24], &[0, 0, 0, 2, 0, 0, 0, 2]);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The snapshot key is the one place this tool writes colour out as a file,
    /// so the picture it saves has to be the graded one. A two-node cube that
    /// turns green end for end is read off text, handed to the thread that shows
    /// a synthetic stream, and the frame that thread shows is saved: the saved
    /// file's own scanlines come out byte for byte as that frame, and that
    /// frame's green is what the cube's text says of the ungraded picture. The
    /// file's own header is asked as well, because a stored, unfiltered PNG
    /// carries one flat run of bytes and a picture encoded at the wrong size
    /// still reads back as the same pixels. Three mutations die here: rows
    /// labelled with a filter that was never applied, byte tables built without
    /// the cube, and dimensions swapped on the way into the encoder.
    #[test]
    fn a_saved_picture_is_the_one_the_grade_made() {
        const INVERT_GREEN: &str = "LUT_1D_SIZE 2
0.0 1.0 0.0
1.0 0.0 1.0
";
        fn stream() -> Vec<u8> {
            let mut bytes = b"YUV4MPEG2 W4 H2 F60:1 Ip C420jpeg\n".to_vec();
            for frame in 0..2u8 {
                bytes.extend_from_slice(b"FRAME\n");
                for y in 0..2u8 {
                    for x in 0..4u8 {
                        bytes.push(16 + x * 40 + y * 7 + frame * 3);
                    }
                }
                bytes.push(64 + frame);
                bytes.push(190 - frame);
            }
            bytes
        }
        fn reader() -> NativeReader<std::io::Cursor<Vec<u8>>> {
            let mut reader = NativeReader::without_memory_limit(std::io::Cursor::new(stream()))
                .expect("a written stream reads");
            assert!(reader.read_frame().expect("a first frame"));
            reader
        }
        fn shown(player: &mut Playback) -> Frame {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                assert!(Instant::now() < deadline, "playback stalled");
                match player.poll() {
                    Some(Event::Frame(frame)) => return frame,
                    Some(Event::Error(error)) => panic!("{error}"),
                    Some(Event::Ended(_)) => panic!("the stream ended before its first frame"),
                    None => std::thread::sleep(Duration::from_millis(1)),
                }
            }
        }
        fn rgb(pixels: &Pixels, budget: usize) -> Vec<u8> {
            match pixels {
                Pixels::Rgb(rgb) => rgb.clone(),
                Pixels::Planar(planes) => {
                    let mut rgb = Vec::new();
                    crate::playback_native::planar8_to_rgb(planes, &mut rgb, budget)
                        .expect("converted");
                    rgb
                }
            }
        }
        /// The scanlines a stored-block PNG carries, filter bytes removed.
        fn scanlines(file: &[u8]) -> Vec<u8> {
            let (width, height) = (
                usize::try_from(u32::from_be_bytes(file[16..20].try_into().unwrap())).unwrap(),
                usize::try_from(u32::from_be_bytes(file[20..24].try_into().unwrap())).unwrap(),
            );
            let row = width * 3 + 1;
            let mut at = 8;
            let idat = loop {
                let len = u32::from_be_bytes(file[at..at + 4].try_into().unwrap()) as usize;
                if &file[at + 4..at + 8] == b"IDAT" {
                    break &file[at + 8..at + 8 + len];
                }
                at += 12 + len;
            };
            assert_eq!(&idat[..2], &[0x78, 0x01], "stored deflate, no dictionary");
            // The stream ends with its four-byte Adler checksum.
            let rows = &idat[7..idat.len() - 4];
            assert_eq!(rows.len(), row * height, "more than one stored block");
            let mut rgb = Vec::with_capacity(width * height * 3);
            for (line, number) in rows.chunks_exact(row).zip(0..height) {
                assert_eq!(line[0], 0, "row {number} was filtered");
                rgb.extend_from_slice(&line[1..]);
            }
            rgb
        }

        let signal = ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        let settings = crate::color::Settings::video(crate::color::DisplayTarget::sdr(240.0));
        assert!(
            crate::color::Grade::new(signal, &HdrMetadata::default(), settings, None).is_identity(),
            "these settings already move this stream on their own, so a cube would not be the only change"
        );
        let grade = crate::color::Grade::new(
            signal,
            &HdrMetadata::default(),
            settings,
            Some(Lut::from_cube(INVERT_GREEN).expect("a written cube is a cube")),
        );
        let budget = reader().rgb_budget();
        let mut plain = Playback::start(reader(), None);
        let mut graded = Playback::start(reader(), Some(grade));
        let untouched = rgb(&shown(&mut plain).pixels, budget);
        let frame = shown(&mut graded);
        let dimensions = frame.dimensions;
        let painted = rgb(&frame.pixels, budget);
        assert_ne!(untouched, painted, "the cube reached no pixels");
        assert_eq!(dimensions, [4, 2]);
        for (pixel, out) in untouched
            .as_chunks::<3>()
            .0
            .iter()
            .zip(painted.as_chunks::<3>().0)
        {
            for channel in 0..3 {
                let want = if channel == 1 {
                    255 - i16::from(pixel[1])
                } else {
                    i16::from(pixel[channel])
                };
                let step = (i16::from(out[channel]) - want).abs();
                assert!(
                    step <= 1,
                    "channel {channel}: {} shown as {}, the cube's own text says {want}",
                    pixel[channel],
                    out[channel]
                );
            }
        }

        let directory = scratch("fvid-player-snapshot-graded", &[]);
        let video = directory.join("clip.mp4");
        std::fs::write(&video, b"an mp4 as far as the snapshot cares").unwrap();
        let mut player = Player {
            opened: Some(video),
            presented: Some(frame),
            ..Default::default()
        };
        player.apply(Control::Snapshot);
        let saved = player.osd.as_ref().unwrap().0.clone();
        let name = saved
            .strip_prefix("Saved ")
            .unwrap_or_else(|| panic!("the snapshot key said {saved}"));
        let file = std::fs::read(directory.join(name)).expect("written");
        // A stored, unfiltered PNG carries its pixels as one flat run, so a file
        // written for the wrong size still hands back the same bytes: the header
        // has to be asked what picture it says, not only the scanlines.
        let saved = [
            usize::try_from(u32::from_be_bytes(file[16..20].try_into().unwrap())).unwrap(),
            usize::try_from(u32::from_be_bytes(file[20..24].try_into().unwrap())).unwrap(),
        ];
        assert_eq!(
            saved, dimensions,
            "the file says a different size from the picture shown"
        );
        assert_eq!(
            scanlines(&file),
            painted,
            "the file holds a different picture from the one shown"
        );
        assert_ne!(
            scanlines(&file),
            untouched,
            "the file holds the ungraded picture"
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_snapshot_without_a_picture_says_so() {
        let mut player = Player::default();
        player.apply(Control::Snapshot);
        assert_eq!(player.osd.as_ref().unwrap().0, "Nothing to save");

        let mut player = Player {
            opened: Some(PathBuf::from("/tmp/clip.mp4")),
            ..Default::default()
        };
        player.apply(Control::Snapshot);
        assert_eq!(player.osd.as_ref().unwrap().0, "Nothing to save");
    }

    /// Arguments are strings, so a queue of two files also has to survive the
    /// options written between them.
    #[test]
    fn play_options_read_the_queue_and_their_own_values() {
        let parsed = play_args(&[
            "a.mp4",
            "--start-time",
            "1:30",
            "--stop-time=95.5",
            "--rate",
            "2.5",
            "b.webm",
        ])
        .expect("options and inputs mix");
        assert_eq!(
            parsed.paths,
            [PathBuf::from("a.mp4"), PathBuf::from("b.webm")]
        );
        assert_eq!(parsed.start, Some(seconds(90_000)));
        assert_eq!(parsed.stop, Some(seconds(95_500)));
        assert_eq!(parsed.rate_milli, 2_500);

        let parsed = play_args(&["clip.mp4"]).expect("one input, no options");
        assert_eq!(parsed.paths, [PathBuf::from("clip.mp4")]);
        assert_eq!(
            (parsed.start, parsed.stop, parsed.rate_milli),
            (None, None, 1_000)
        );
    }

    #[test]
    fn play_options_refuse_what_they_cannot_honour() {
        // A flag without anything after it, a rate outside VLC's ladder, a clock
        // with too many parts, and bounds that cross over.
        assert!(play_args(&["--start-time"]).is_err());
        assert!(play_args(&["a.mp4", "--rate"]).is_err());
        assert!(play_args(&["--rate", "9"]).is_err());
        assert!(play_args(&["--start-time", "1:2:3:4"]).is_err());
        assert!(play_args(&["--start-time", "70", "--stop-time", "60"]).is_err());
        assert!(play_args(&["--toggle", "a.mp4"]).is_err());
        assert_eq!(
            play_args(&["a=b.mp4"])
                .expect("a name with a sign")
                .paths
                .len(),
            1
        );
    }

    #[test]
    fn clocks_are_seconds_or_a_ladder_of_them() {
        assert_eq!(parse_clock("90").expect("plain seconds"), seconds(90_000));
        assert_eq!(parse_clock("1:30").expect("minutes"), seconds(90_000));
        assert_eq!(
            parse_clock("01:02:03.5").expect("hours"),
            seconds(3_723_500)
        );
        assert!(parse_clock("-5").is_err());
        assert!(parse_clock("").is_err());
        assert!(parse_clock("abc").is_err());
    }

    #[test]
    fn the_stop_time_ends_the_item_it_was_given_for() {
        let directory = scratch("fvid-player-stop-time", &[]);
        let path = directory.join("clip.mp4");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/audio/two-audio.mp4"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            bounds: PlayBounds {
                start: Some(seconds(500)),
                stop: Some(seconds(1_500)),
            },
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        // The start bound is spent on this open, the stop bound waits for the picture.
        assert_eq!(player.bounds, PlayBounds::default());
        assert_eq!(player.stop_time, Some(seconds(1_500)));

        // Before the picture has a position there is nothing to compare to.
        player.check_stop();
        assert!(!player.ended);

        player.seek_time(seconds(1_600));
        player.check_stop();
        assert!(player.ended, "a queue of one stops where it was told to");
        assert_eq!(player.stop_time, None, "the bound is spent either way");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn an_empty_list_has_nothing_to_continue_into() {
        let mut player = Player::default();
        player.continue_queue();
        assert!(player.ended);
    }

    /// VLC's loop key presses A, then B, then clears the pair. A press before
    /// the mark already set cannot loop backwards, so it opens a new region.
    #[test]
    fn the_loop_key_marks_a_then_b_then_clears() {
        assert_eq!(
            loop_press(seconds(2_000), None, None),
            LoopMark::MarkedA(seconds(2_000))
        );
        assert_eq!(
            loop_press(seconds(5_000), Some(seconds(2_000)), None),
            LoopMark::MarkedB(seconds(5_000))
        );
        assert_eq!(
            loop_press(seconds(9_000), Some(seconds(2_000)), Some(seconds(5_000))),
            LoopMark::Cleared
        );
        assert_eq!(
            loop_press(seconds(1_000), Some(seconds(2_000)), None),
            LoopMark::MarkedA(seconds(1_000))
        );
    }

    /// Only a closed region rewinds, and only once the picture has reached B.
    #[test]
    fn a_closed_region_rewinds_at_its_end() {
        let (a, b) = (seconds(2_000), seconds(5_000));
        assert_eq!(loop_rewind(seconds(4_999), Some(a), Some(b)), None);
        assert_eq!(loop_rewind(seconds(5_000), Some(a), Some(b)), Some(a));
        assert_eq!(loop_rewind(seconds(9_000), Some(a), None), None);
        assert_eq!(loop_rewind(seconds(9_000), None, Some(b)), None);
    }

    /// Rewinding is a seek, so a track without a sample index cannot loop, and
    /// the marks of the previous item do not carry into the next one.
    #[test]
    fn only_a_seekable_track_loops() {
        let directory = scratch("fvid-player-loop", &[]);
        std::fs::write(directory.join("clip.y4m"), y4m(2)).unwrap();
        let mut player = Player {
            queue: vec![directory.join("clip.y4m")],
            loop_a: Some(seconds(2_000)),
            loop_b: Some(seconds(5_000)),
            ..Default::default()
        };
        player.play_index(0);
        assert_eq!((player.loop_a, player.loop_b), (None, None));
        assert!(!player.seekable);
        player.apply(Control::Loop);
        assert_eq!(player.osd.as_ref().unwrap().0, "Loop needs seeking");
        assert_eq!((player.loop_a, player.loop_b), (None, None));
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A positive delay is what a late voice track needs: the gate sees the
    /// audio clock as further ahead, so the picture runs on by that much.
    #[test]
    fn a_positive_audio_delay_lets_the_picture_run_ahead() {
        assert_eq!(delayed_clock(seconds(1_000), 0), seconds(1_000));
        assert_eq!(delayed_clock(seconds(1_000), 250), seconds(1_250));
        assert_eq!(delayed_clock(seconds(1_000), -250), seconds(750));
        // The clock cannot go before the start of the file.
        assert_eq!(delayed_clock(seconds(100), -250), Duration::ZERO);
    }

    /// The track key wraps within the tracks the container actually offered.
    #[test]
    fn audio_tracks_wrap_around_the_list() {
        assert_eq!(track_step(0, 3, true), 1);
        assert_eq!(track_step(2, 3, true), 0);
        assert_eq!(track_step(0, 3, false), 2);
        assert_eq!(track_step(1, 2, false), 0);
    }

    /// With nothing to choose between, the key says so rather than restarting
    /// the sound that is already playing.
    #[test]
    fn the_track_key_needs_a_choice_to_do_anything() {
        let mut player = Player::default();
        player.apply(Control::AudioTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "No audio");
        player.audio_tracks = vec![crate::audio::AudioTrack {
            sample_rate: 48_000,
            channels: 2,
            name: String::new(),
            language: String::new(),
        }];
        player.apply(Control::AudioTrack(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Audio 1/1");
        assert!(player.audio.is_none());
    }

    #[test]
    fn the_audio_delay_key_is_bounded_at_ten_seconds() {
        let mut player = Player::default();
        player.apply(Control::AudioDelay(true));
        assert_eq!(player.audio_delay_ms, 50);
        assert_eq!(player.osd.as_ref().unwrap().0, "Audio delay 50 ms");
        for _ in 0..300 {
            player.apply(Control::AudioDelay(true));
        }
        assert_eq!(player.audio_delay_ms, 10_000);
        for _ in 0..500 {
            player.apply(Control::AudioDelay(false));
        }
        assert_eq!(player.audio_delay_ms, -10_000);
    }

    /// The chapter keys answer with the starts the list carries: the first one
    /// ahead going forward, the last one behind going back, and nothing to walk
    /// once the position stands at either end of the film.
    #[test]
    fn the_chapter_keys_walk_the_list_and_stop_at_either_end() {
        let starts = [seconds(0), seconds(1_000), seconds(3_000)];
        assert_eq!(chapter_ahead(&starts, seconds(0), true), Some(1));
        assert_eq!(chapter_ahead(&starts, seconds(500), true), Some(1));
        assert_eq!(chapter_ahead(&starts, seconds(1_000), true), Some(2));
        assert_eq!(chapter_ahead(&starts, seconds(3_000), true), None);
        assert_eq!(chapter_ahead(&starts, seconds(3_500), false), Some(2));
        assert_eq!(chapter_ahead(&starts, seconds(1_000), false), Some(0));
        assert_eq!(chapter_ahead(&starts, seconds(0), false), None);
    }

    /// The Matroska chapter atoms and the `chpl` of the MP4 muxed from the same
    /// film hand the player one and the same list; a file that names no parts,
    /// or one there is no reader for, leaves nothing to walk.
    #[test]
    fn both_containers_hand_the_player_the_same_chapter_list() {
        let directory = scratch("fvid-player-chapters", &[]);
        let matroska = directory.join("chapters.mkv");
        let movie = directory.join("chapters.mp4");
        std::fs::write(
            &matroska,
            include_bytes!("../tests/fixtures/chapters/chapters.mkv"),
        )
        .unwrap();
        std::fs::write(
            &movie,
            include_bytes!("../tests/fixtures/chapters/chapters.mp4"),
        )
        .unwrap();
        let expected = [
            (Duration::ZERO, "Opening".to_string()),
            (Duration::from_secs(1), "Глава 2".to_string()),
            (Duration::from_secs(3), "End".to_string()),
        ];
        let shown = |marks: &[ChapterMark]| -> Vec<(Duration, String)> {
            marks
                .iter()
                .map(|mark| (mark.start, mark.title.clone()))
                .collect()
        };
        assert_eq!(shown(&container_facts(&matroska).chapters), expected);
        assert_eq!(shown(&container_facts(&movie).chapters), expected);
        assert!(
            container_facts(&directory.join("no-such-file.mkv"))
                .chapters
                .is_empty()
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A file that names its parts: the key moves the picture to the start of
    /// the next one and says which, the end of the list says so instead of
    /// seeking, and the back key returns to the part just left.
    #[test]
    fn a_file_that_names_its_parts_jumps_between_them() {
        let directory = scratch("fvid-player-chapter-walk", &[]);
        let path = directory.join("chapters.mkv");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/chapters/chapters.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert!(player.seekable);
        assert_eq!(player.chapters.len(), 3);
        // The walk starts from where the picture stands, so put the first frame
        // of the film on the screen: a quarter of a second on the nanosecond
        // clock Matroska counts in.
        player.interval = Some((0, 250_000_000, 1_000_000_000));

        player.apply(Control::Chapter(true));
        assert_eq!(player.seek_target, Some(Duration::from_secs(1)));
        assert_eq!(
            player.osd.as_ref().unwrap().0,
            "Chapter 2/3 Глава 2",
            "{:?}",
            player.error
        );

        player.apply(Control::Chapter(true));
        assert_eq!(player.seek_target, Some(Duration::from_secs(3)));
        assert_eq!(player.osd.as_ref().unwrap().0, "Chapter 3/3 End");

        player.apply(Control::Chapter(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Last chapter");
        assert_eq!(player.seek_target, Some(Duration::from_secs(3)));

        player.apply(Control::Chapter(false));
        assert_eq!(player.seek_target, Some(Duration::from_secs(1)));
        assert_eq!(player.osd.as_ref().unwrap().0, "Chapter 2/3 Глава 2");
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// With no list in the file there is nothing to walk, and the key says so
    /// rather than restarting the position it cannot move.
    #[test]
    fn the_chapter_keys_say_when_the_file_names_no_parts() {
        let mut player = Player::default();
        player.apply(Control::Chapter(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "No chapters");
        player.chapters = vec![ChapterMark {
            start: Duration::from_secs(1),
            title: String::new(),
        }];
        player.apply(Control::Chapter(true));
        assert_eq!(player.osd.as_ref().unwrap().0, "Cannot seek");
    }

    /// The switches and the two track numbers, read in either value form. A
    /// track is counted from one, because that is how the player's own messages
    /// count them, and zero is not a track anywhere.
    #[test]
    fn the_start_switches_and_the_track_numbers_count_from_one() {
        let parsed = play_args(&[
            "a.mp4",
            "--start-paused",
            "--no-audio",
            "--audio-track",
            "2",
            "--subtitle-track=3",
        ])
        .expect("the switches and both numbers");
        assert!(parsed.start_paused);
        assert!(parsed.no_audio);
        assert_eq!(
            (parsed.audio_track, parsed.subtitle_track),
            (Some(1), Some(2))
        );
        assert_eq!(parsed.paths.len(), 1);

        let plain = play_args(&["a.mp4"]).expect("no options at all");
        assert_eq!(
            (
                plain.start_paused,
                plain.no_audio,
                plain.audio_track,
                plain.subtitle_track
            ),
            (false, false, None, None)
        );
        assert!(
            play_args(&["--audio-track", "0"]).is_err(),
            "zero names no track"
        );
        assert!(play_args(&["--audio-track", "b"]).is_err());
        assert!(play_args(&["--subtitle-track"]).is_err());
        assert!(
            play_args(&["--start-paused=now"]).is_err(),
            "a switch takes no value"
        );
        assert!(play_args(&["--no-audio=yes"]).is_err());
    }

    /// `--start-paused` leaves the first picture on the screen and the item
    /// waiting, so the window is not a blank one; the play key lets it go.
    #[test]
    fn a_paused_start_shows_its_first_picture_and_waits() {
        let directory = scratch("fvid-player-start-paused", &[]);
        let path = directory.join("two-audio.mp4");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/audio/two-audio.mp4"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path.clone()],
            start_paused: true,
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        assert!(player.paused);
        assert_eq!(player.step, 1, "one picture is asked for");
        assert!(player.buffering);
        player.toggle_pause();
        assert!(!player.paused);

        let mut running = Player {
            queue: vec![path],
            ..Default::default()
        };
        running.play_index(0);
        assert!(!running.paused);
        assert_eq!(running.step, 0);
        drop(player);
        drop(running);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The sound of the file, or none of it: `--no-audio` leaves the track list
    /// empty, and a track number picks the one the container offers. A number
    /// past the end of the list leaves the track there is to hear.
    #[test]
    fn the_command_line_can_pick_the_sound_or_leave_it_out() {
        let directory = scratch("fvid-player-audio-choice", &[]);
        let path = directory.join("two-audio.mp4");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/audio/two-audio.mp4"),
        )
        .unwrap();

        let mut silent = Player {
            queue: vec![path.clone()],
            no_audio: true,
            ..Default::default()
        };
        silent.play_index(0);
        assert!(silent.error.is_none(), "{:?}", silent.error);
        assert!(silent.audio.is_none());
        assert!(silent.audio_tracks.is_empty());
        assert_eq!(silent.name, "two-audio.mp4");

        let mut second = Player {
            queue: vec![path.clone()],
            preferred_audio: Some(1),
            ..Default::default()
        };
        second.play_index(0);
        assert_eq!((second.audio_tracks.len(), second.audio_track), (2, 1));
        assert!(second.audio.is_some());

        let mut past_the_end = Player {
            queue: vec![path],
            preferred_audio: Some(9),
            ..Default::default()
        };
        past_the_end.play_index(0);
        assert_eq!(
            past_the_end.audio_track, 0,
            "a track there is not is left out"
        );
        drop(silent);
        drop(second);
        drop(past_the_end);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A file with nothing to show is played by its sound, so a session that
    /// refuses sound has nothing left to open. The reason says which option said
    /// so, rather than repeating the picture's complaint.
    #[test]
    fn a_file_with_nothing_to_show_and_no_sound_allowed_refuses() {
        let directory = scratch("fvid-player-no-audio-sound-only", &[]);
        let path = directory.join("flac-stereo.mkv");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/audio/flac-stereo.mkv"),
        )
        .unwrap();

        let mut heard = Player {
            queue: vec![path.clone()],
            ..Default::default()
        };
        heard.play_index(0);
        assert!(heard.error.is_none(), "{:?}", heard.error);
        assert!(heard.audio.is_some());
        assert!(heard.playback.is_none());

        let mut refused = Player {
            queue: vec![path],
            no_audio: true,
            ..Default::default()
        };
        refused.play_index(0);
        let message = refused.error.take().expect("the refusal is reported");
        assert!(
            message.contains("--no-audio"),
            "the refusal names the option: {message}"
        );
        drop(heard);
        drop(refused);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A Dolby Digital file on its own is sound the player can hear: the frame
    /// walk in [`crate::playback_ac3`] lists its packets, so the item opens as
    /// sound with the panel naming the coding the frames state. The surround
    /// fixture is a 5.1 stream, and the count the panel shows is the frame's own
    /// native layout rather than a container's claim.
    #[test]
    fn a_bare_dolby_digital_file_opens_as_sound_with_its_own_geometry() {
        let directory = scratch("fvid-player-elementary-ac3", &[]);
        let path = directory.join("ac3-51.ac3");
        std::fs::write(&path, include_bytes!("../tests/fixtures/audio/ac3-51.ac3")).unwrap();

        let mut heard = Player {
            queue: vec![path.clone()],
            ..Default::default()
        };
        heard.play_index(0);
        assert!(heard.error.is_none(), "{:?}", heard.error);
        assert!(heard.playback.is_none(), "nothing to show");
        assert!(heard.audio.is_some(), "the sound carries the item");
        assert_eq!(heard.sound_codec, "Dolby Digital");
        assert_eq!(
            heard.audio_tracks.len(),
            1,
            "an elementary stream is one track"
        );
        assert_eq!(
            (
                heard.audio_tracks[0].sample_rate,
                heard.audio_tracks[0].channels
            ),
            (48_000, 6)
        );
        // Eight frames of 1536 samples at 48 kHz, which is what ffprobe reports.
        assert_eq!(heard.duration, Some(Duration::from_micros(256_000)));
        assert!(heard.seekable, "a walked stream has a seek table");
        drop(heard);

        // A track key past the one the file holds leaves it playing: the list the
        // stream itself reported is the bound, and the first track is the one there
        // is to hear.
        let mut asked_for_two = Player {
            queue: vec![path],
            preferred_audio: Some(1),
            ..Default::default()
        };
        asked_for_two.play_index(0);
        assert!(asked_for_two.error.is_none(), "{:?}", asked_for_two.error);
        assert_eq!(asked_for_two.audio_track, 0);
        assert_eq!(asked_for_two.sound_codec, "Dolby Digital");
        drop(asked_for_two);

        // The rate a stream's own frames state is the rate the item runs on: this
        // fixture is mono at 32 kHz over six frames, and no option says so.
        let mono = directory.join("ac3-mono-32k.ac3");
        std::fs::write(
            &mono,
            include_bytes!("../tests/fixtures/audio/ac3-mono-32k.ac3"),
        )
        .unwrap();
        let mut low = Player {
            queue: vec![mono],
            ..Default::default()
        };
        low.play_index(0);
        assert!(low.error.is_none(), "{:?}", low.error);
        assert_eq!(
            (
                low.audio_tracks[0].sample_rate,
                low.audio_tracks[0].channels
            ),
            (32_000, 1)
        );
        assert_eq!(low.duration, Some(Duration::from_millis(288)));
        drop(low);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// An ADTS file on its own is sound the same way, with its geometry read out
    /// of the frames rather than a sample entry: the fixture is AAC-LC at 48 kHz
    /// over thirteen frames of 1024 samples. The panel names the coding the frames
    /// state even though this reader has to write the decoder's setup block
    /// itself, which is the one piece of an MP4 sample entry an elementary file
    /// holds nowhere.
    #[test]
    fn a_bare_adts_file_opens_as_sound_with_the_config_its_frames_state() {
        let directory = scratch("fvid-player-elementary-aac", &[]);
        let path = directory.join("aac-stereo.aac");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/audio/aac-stereo.aac"),
        )
        .unwrap();

        let mut heard = Player {
            queue: vec![path.clone()],
            ..Default::default()
        };
        heard.play_index(0);
        assert!(heard.error.is_none(), "{:?}", heard.error);
        assert!(heard.playback.is_none(), "nothing to show");
        assert!(heard.audio.is_some(), "the sound carries the item");
        assert_eq!(heard.sound_codec, "AAC");
        assert_eq!(
            heard.audio_tracks.len(),
            1,
            "an elementary stream is one track"
        );
        assert_eq!(
            (
                heard.audio_tracks[0].sample_rate,
                heard.audio_tracks[0].channels
            ),
            (48_000, 2)
        );
        // Thirteen frames of 1024 samples at 48 kHz: twelve of audio plus the
        // encoder's tag frame, which the reference demuxer counts as a packet too.
        assert_eq!(heard.duration.map(|at| at.as_micros()), Some(277_333));
        assert!(heard.seekable, "a walked stream has a seek table");
        drop(heard);

        // A second track key has nothing to land on, as with the other elementary
        // readers, and the one track the frames describe keeps playing.
        let mut asked_for_two = Player {
            queue: vec![path],
            preferred_audio: Some(1),
            ..Default::default()
        };
        asked_for_two.play_index(0);
        assert!(asked_for_two.error.is_none(), "{:?}", asked_for_two.error);
        assert_eq!(asked_for_two.audio_track, 0);
        assert_eq!(asked_for_two.sound_codec, "AAC");
        drop(asked_for_two);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The text the command line asked for is on screen from the first frame,
    /// and a source the file does not have leaves the one it does.
    #[test]
    fn the_subtitle_option_lands_on_the_track_it_names() {
        let directory = scratch("fvid-player-subtitle-choice", &[]);
        let path = directory.join("text-tracks.mkv");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/subtitles/text-tracks.mkv"),
        )
        .unwrap();

        let mut second = Player {
            queue: vec![path.clone()],
            preferred_subtitle: Some(1),
            ..Default::default()
        };
        second.play_index(0);
        assert_eq!(
            (second.subtitle_sources.len(), second.subtitle_source),
            (2, 1)
        );
        assert_eq!(second.subtitle_line(seconds(700)), Some("other track"));

        let mut first = Player {
            queue: vec![path.clone()],
            preferred_subtitle: Some(9),
            ..Default::default()
        };
        first.play_index(0);
        assert_eq!(first.subtitle_source, 0);

        let mut none = Player {
            queue: vec![path],
            ..Default::default()
        };
        none.play_index(0);
        assert_eq!(none.subtitle_source, 0);
        drop(second);
        drop(first);
        drop(none);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// The level, the mute switch, what the queue does at its end and both
    /// clock shifts, read in the units the player's own messages use: VLC's
    /// percent for the level and milliseconds for the delays, which the delay
    /// keys hold to ±10 s and so are held here too.
    #[test]
    fn the_level_the_repeat_and_both_delays_start_where_the_keys_hold_them() {
        let parsed = play_args(&[
            "a.mp4",
            "--volume",
            "150",
            "--mute",
            "--loop",
            "--random",
            "--audio-delay=-250",
            "--subtitle-delay",
            "12000",
            "--sub-file",
            "subs.srt",
        ])
        .expect("every start option");
        assert_eq!(parsed.volume_milli, 1_500);
        assert!(parsed.muted);
        assert_eq!(parsed.repeat, Repeat::All);
        assert!(parsed.shuffle, "a queue that walks a drawn order");
        assert_eq!(
            (parsed.audio_delay_ms, parsed.subtitle_delay_ms),
            (-250, 10_000)
        );
        assert_eq!(parsed.subtitle_file, Some(PathBuf::from("subs.srt")));

        let plain = play_args(&["a.mp4"]).expect("no options at all");
        assert_eq!(
            (
                plain.volume_milli,
                plain.muted,
                plain.repeat,
                plain.shuffle,
                plain.audio_delay_ms,
                plain.subtitle_delay_ms,
                plain.subtitle_file
            ),
            (1_000, false, Repeat::Off, false, 0, 0, None)
        );

        // The last of the two queue switches wins, the way the repeat key
        // replaces the mode it found rather than adding to it.
        assert_eq!(
            play_args(&["a.mp4", "--loop", "--repeat"])
                .expect("both queue switches")
                .repeat,
            Repeat::One
        );

        assert!(play_args(&["--volume", "201"]).is_err(), "past the slider");
        assert!(play_args(&["--volume", "b"]).is_err());
        assert!(play_args(&["--volume"]).is_err());
        assert!(
            play_args(&["--mute=yes"]).is_err(),
            "a switch takes no value"
        );
        assert!(play_args(&["--loop=1"]).is_err());
        assert!(play_args(&["--audio-delay", "x"]).is_err());
        assert!(play_args(&["--subtitle-delay"]).is_err());
        assert!(play_args(&["--sub-file"]).is_err());
    }

    /// A subtitle file the command line names leads the list the player guesses
    /// from the video's own name, and a name nothing can be read from says so
    /// instead of falling back unheard.
    #[test]
    fn a_named_subtitle_file_leads_the_list_the_player_guesses() {
        let directory = scratch("fvid-player-sub-file", &[]);
        let video = directory.join("film.mp4");
        std::fs::write(&video, b"not a container").unwrap();
        std::fs::write(
            directory.join("film.srt"),
            "1\n00:00:01,000 --> 00:00:02,000\nGuessed\n",
        )
        .unwrap();
        let named = directory.join("chosen.srt");
        std::fs::write(&named, "1\n00:00:01,000 --> 00:00:02,000\nNamed\n").unwrap();

        let mut player = Player {
            subtitle_file: Some(named.clone()),
            ..Default::default()
        };
        player.load_subtitles(&video);
        assert_eq!(
            player
                .subtitle_sources
                .iter()
                .map(|source| source.label.as_str())
                .collect::<Vec<_>>(),
            ["chosen.srt", "film.srt"]
        );
        assert_eq!(player.subtitle_name, "chosen.srt");
        assert_eq!(player.cues[0].text, "Named");

        player.subtitle_file = Some(directory.join("missing.srt"));
        player.load_subtitles(&video);
        assert_eq!(player.subtitle_sources[0].label, "film.srt");
        assert_eq!(player.subtitle_name, "film.srt");
        assert!(
            player
                .osd
                .as_ref()
                .unwrap()
                .0
                .starts_with("Cannot read subtitles"),
            "{:?}",
            player.osd
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A two-node cube that hands every code back: the look a test asks for when
    /// it wants a LUT file without wanting a colour change.
    const IDENTITY_CUBE: &str = "\
LUT_3D_SIZE 2
0 0 0
1 0 0
0 1 0
1 1 0
0 0 1
1 0 1
0 1 1
1 1 1
";

    /// The three colour options name what the session grades with. A curve and a
    /// compression are matched to fvid's own spellings whatever the case, while a
    /// LUT has to be a file that actually parses: a word on the list, a readable
    /// grid.
    #[test]
    fn the_command_line_names_the_colour_grade_to_start_with() {
        let parsed = play_args(&["--log", "SLog3", "--tonemap=Hable", "c"]).unwrap();
        assert_eq!(parsed.grading.log, Some(Log::SLog3));
        assert_eq!(parsed.grading.tone_map, Some(ToneMap::Hable));
        assert_eq!(parsed.grading.lut, None);
        let parsed = play_args(&["--log", " logc4 ", "c"]).unwrap();
        assert_eq!(parsed.grading.log, Some(Log::LogC4));
        assert!(play_args(&["c"]).unwrap().grading.silent());
        for words in [
            vec!["--log", "slog4"],
            vec!["--log", "s-log3"],
            vec!["--log", ""],
            vec!["--log"],
            vec!["--tonemap", "mobius2"],
            vec!["--tonemap", "bt244"],
            vec!["--tonemap"],
            vec!["--lut", "no-such-file.cube"],
        ] {
            assert!(
                play_args(&[words.as_slice(), &["c"]].concat()).is_err(),
                "{words:?} is not a grade"
            );
        }
        // A refusal names what is on offer, so the caller sees the spellings
        // that would have worked.
        let error = play_args(&["--log", "slog4", "c"])
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("slog3") && error.contains("logc4"),
            "{error}"
        );
        let error = play_args(&["--tonemap", "bt244", "c"])
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("reinhard") && error.contains("mobius"),
            "{error}"
        );
    }

    /// `--gamut` names the working gamut the bytes are read in, which is how a
    /// container that labels them wrongly gets corrected: the flag outranks the
    /// primaries the file states and the triangle a `--log` curve lends, and it
    /// is the only route to the gamuts no curve lends — S-Gamut3.Cine, D-Gamut
    /// and F-Gamut C among them. Spellings fold case and separators, so
    /// `S-Gamut3.Cine`, `sgamut3_cine` and a vendor's own `AWG 4` all land on a
    /// triangle, and a word that names none stops the command at the door with
    /// the spellings that would have worked.
    #[test]
    fn the_command_line_names_the_gamut_the_bytes_are_read_in() {
        let parsed = play_args(&["--gamut", "S-Gamut3.Cine", "c"]).unwrap();
        assert_eq!(parsed.grading.gamut, Some(Primaries::S_GAMUT3_CINE));
        assert_eq!(
            play_args(&["--gamut=sgamut3_cine", "c"])
                .unwrap()
                .grading
                .gamut,
            Some(Primaries::S_GAMUT3_CINE)
        );
        assert_eq!(
            play_args(&["--gamut", " AWG 4 ", "c"])
                .unwrap()
                .grading
                .gamut,
            Some(Primaries::ALEX3_EXPANDED)
        );
        assert_eq!(
            play_args(&["--gamut", "rec709", "c"])
                .unwrap()
                .grading
                .gamut,
            Some(Primaries::BT709)
        );
        assert!(play_args(&["c"]).unwrap().grading.gamut.is_none());
        for words in [
            vec!["--gamut", "sgamut"],
            vec!["--gamut", "awg3"],
            vec!["--gamut", ""],
            vec!["--gamut"],
        ] {
            assert!(
                play_args(&[words.as_slice(), &["c"]].concat()).is_err(),
                "{words:?} names no gamut"
            );
        }
        let error = play_args(&["--gamut", "sgamut", "c"])
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("S-Gamut3.Cine") && error.contains("F-Gamut C"),
            "{error}"
        );
    }

    /// `--display` names the panel the session grades for, and a panel is a
    /// request in its own right: it moves where the picture's light lands
    /// whether the item states BT.2100 or nothing at all.
    #[test]
    fn a_named_panel_moves_where_the_picture_lands() {
        let empty = HdrMetadata::default();
        let video = ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        let hdr10 = ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: false,
        };
        // A brighter SDR panel has a higher white, so the same 100 cd/m² of the
        // master is a darker code on the screen that reaches 600.
        let panel = |nits| Grading {
            panel: Some(Panel::Sdr(nits)),
            ..Default::default()
        };
        let lid = Grading::default().grade_for(hdr10, &empty).unwrap();
        let bright = panel(600.0).grade_for(hdr10, &empty).unwrap();
        assert_eq!(lid.plan().target, Some(DisplayTarget::sdr(PANEL_NITS)));
        assert_eq!(bright.plan().target, Some(DisplayTarget::sdr(600.0)));
        assert!(lid.rgb([0.5; 3])[0] > 0.9, "on a 100-nit lid: {lid:?}");
        assert!(
            bright.rgb([0.5; 3])[0] < 0.45,
            "on a 600-nit panel: {:?}",
            bright.rgb([0.5; 3])
        );
        // A BT.2100 destination writes BT.2100 codes over BT.2020 and runs no
        // shoulder, which on HDR10 material is the whole point: the bytes the
        // file holds are the picture the screen wants.
        let hdr = Grading {
            panel: Some(Panel::Pq(1_000.0)),
            ..Default::default()
        }
        .grade_for(hdr10, &empty)
        .unwrap();
        let plan = hdr.plan();
        assert_eq!(
            (plan.to, plan.dest, plan.tone_map),
            (Transfer::Pq, Primaries::BT2020, None)
        );
        assert!(hdr.is_identity(), "HDR10 onto HDR10 changes nothing");
        // An SDR item asked for a PQ destination is the same picture in codes
        // that state its light: white leaves at the panel's diffuse white, not
        // at the top of PQ's 10 000-nit scale.
        let up = Grading {
            panel: Some(Panel::Pq(1_000.0)),
            ..Default::default()
        }
        .grade_for(video, &empty)
        .unwrap();
        assert!(!up.is_identity());
        let nits = Transfer::Pq.to_nits(up.rgb([1.0; 3])[0], 262.0).unwrap();
        assert!(
            (nits - 262.0).abs() < 6.0,
            "SDR white reached {nits} cd/m² on a 1 000-nit PQ panel"
        );
        // The peak a caller names is what an HLG code means, so two panels of
        // different capability cannot share the grade.
        let hlg = |peak| Grading {
            panel: Some(Panel::Hlg(peak)),
            ..Default::default()
        };
        let small = hlg(400.0).grade_for(hdr10, &empty).unwrap();
        let big = hlg(1_000.0).grade_for(hdr10, &empty).unwrap();
        assert_eq!(
            (small.plan().to, small.plan().dest),
            (Transfer::Hlg, Primaries::BT2020)
        );
        assert_eq!(small.plan().target, Some(DisplayTarget::hdr(400.0)));
        assert_ne!(small.rgb([0.5; 3]), big.rgb([0.5; 3]));
    }

    /// `--display` takes a kind and optionally the cd/m² it reaches, in either
    /// spelling of the option; anything else stops the command at the door.
    #[test]
    fn the_command_line_names_the_panel_the_picture_is_graded_for() {
        let parsed = |args: &[&str]| play_args(args).unwrap().grading.panel;
        assert_eq!(parsed(&["--display=pq", "c"]), Some(Panel::Pq(1_000.0)));
        assert_eq!(
            parsed(&["--display", "hdr:600", "c"]),
            Some(Panel::Pq(600.0))
        );
        assert_eq!(
            parsed(&["--display", " SDR : 240 ", "c"]),
            Some(Panel::Sdr(240.0))
        );
        assert_eq!(parsed(&["--display=hlg", "c"]), Some(Panel::Hlg(1_000.0)));
        assert_eq!(
            parsed(&["--display=sdr", "c"]),
            Some(Panel::Sdr(PANEL_NITS))
        );
        assert!(parsed(&["c"]).is_none());
        for bad in [
            "--display=bt2020",
            "--display=sdr:0",
            "--display=pq:99999",
            "--display=hlg:high",
            "--display=pq:",
        ] {
            let error = play_args(&[bad, "c"]).err().unwrap().to_string();
            assert!(error.contains("--display"), "{bad} said {error:?}");
        }
        // A kind fvid writes no codes for is answered with the kinds it does.
        let error = play_args(&["--display=bt2020", "c"])
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("sdr[:nits]") && error.contains("hlg[:nits]"),
            "{error}"
        );
        // With no value at all the message is the one every option of this
        // command gives, which names the option rather than its grammar.
        assert!(
            play_args(&["--display", "c"])
                .err()
                .unwrap()
                .to_string()
                .contains("--display")
        );
    }

    /// Naming a gamut is a request in its own right, not a modifier: an SDR item
    /// that states BT.709 has to be re-read as the gamut the caller names, and
    /// written back in the signal the file states for itself. The picture is not
    /// the one the label implied, so nothing here is an identity.
    #[test]
    fn a_named_gamut_grades_an_item_that_asked_for_nothing_else() {
        let video = ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        let grade = Grading {
            gamut: Some(Primaries::S_GAMUT3_CINE),
            ..Default::default()
        }
        .grade_for(video, &HdrMetadata::default())
        .expect("a named gamut is a request");
        let plan = grade.plan();
        assert_eq!(plan.source, Primaries::S_GAMUT3_CINE);
        assert_eq!(
            (plan.from, plan.to, plan.dest),
            (Transfer::Bt709, Transfer::Bt709, Primaries::BT709)
        );
        assert!(!grade.is_identity());
        // With a curve, the name still wins over the triangle the profile lends.
        let graded = Grading {
            gamut: Some(Primaries::D_GAMUT),
            log: Some(Log::VLog),
            ..Default::default()
        }
        .grade_for(ColourDescription::default(), &HdrMetadata::default())
        .unwrap();
        assert_eq!(graded.plan().source, Primaries::D_GAMUT);
    }

    /// A cube's edge and the way its nodes are joined say how finely the colour
    /// decision is baked, so they move the picture a request produces; naming
    /// either of them alone asks for no picture at all.
    #[test]
    fn a_named_grid_and_reading_change_how_the_picture_is_baked() {
        let empty = HdrMetadata::default();
        let hdr10 = ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: false,
        };
        let video = ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        let baked = |grid, interp| {
            Grading {
                grid,
                interp,
                ..Default::default()
            }
            .grade_for(hdr10, &empty)
            .unwrap()
        };
        assert_eq!(baked(None, None).plan().size, 33);
        let coarse = baked(Some(2), None);
        let fine = baked(Some(64), None);
        assert_eq!((coarse.plan().size, fine.plan().size), (2, 64));
        assert_ne!(coarse.rgb([0.5; 3]), fine.rgb([0.5; 3]));
        let stepped = baked(Some(2), Some(Interpolation::Nearest));
        assert_ne!(stepped.rgb([0.5; 3]), coarse.rgb([0.5; 3]));
        // The same reading is what a look from a file gets: a two-node grid whose
        // top corner is off-white blends towards it when the box around a point
        // is read, and keeps its step when only the nearest corner is.
        let look = Lut::from_cube(
            "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 0\n",
        )
        .unwrap();
        let with = |interp| {
            Grading {
                lut: Some(look.clone()),
                interp,
                ..Default::default()
            }
            .grade_for(video, &empty)
            .unwrap()
        };
        assert_ne!(
            with(None).rgb([0.5; 3]),
            with(Some(Interpolation::Nearest)).rgb([0.5; 3])
        );
        for modifier in [
            Grading {
                grid: Some(64),
                ..Default::default()
            },
            Grading {
                interp: Some(Interpolation::Nearest),
                ..Default::default()
            },
        ] {
            assert!(modifier.grade_for(video, &empty).is_none());
        }
    }

    /// `--grid` takes the node count the conversion is baked at and `--interp`
    /// one of the readings fvid knows, in either spelling of the option; the
    /// bound and the set of readings are both in the answer a mistake gets.
    #[test]
    fn the_command_line_names_the_cube_and_how_its_nodes_are_joined() {
        let grid = |args: &[&str]| play_args(args).unwrap().grading.grid;
        let interp = |args: &[&str]| play_args(args).unwrap().grading.interp;
        assert_eq!(grid(&["--grid=64", "c"]), Some(64));
        assert_eq!(grid(&["--grid", " 12 ", "c"]), Some(12));
        assert_eq!(grid(&["--grid", "2", "c"]), Some(2));
        assert_eq!(grid(&["--grid", "128", "c"]), Some(128));
        assert!(grid(&["c"]).is_none());
        assert_eq!(
            interp(&["--interp=nearest", "c"]),
            Some(Interpolation::Nearest)
        );
        assert_eq!(
            interp(&["--interp", " TRILINEAR ", "c"]),
            Some(Interpolation::Trilinear)
        );
        assert_eq!(
            interp(&["--interp=linear", "c"]),
            Some(Interpolation::Trilinear)
        );
        assert_eq!(
            interp(&["--interp", "tetrahedral", "c"]),
            Some(Interpolation::Tetrahedral)
        );
        assert!(interp(&["c"]).is_none());
        for bad in [
            "--grid=1",
            "--grid=129",
            "--grid=deep",
            "--grid=-4",
            "--interp=bilinear",
            "--interp=",
        ] {
            let option = if bad.starts_with("--grid") {
                "--grid"
            } else {
                "--interp"
            };
            let error = play_args(&[bad, "c"]).err().unwrap().to_string();
            assert!(error.contains(option), "{bad} said {error:?}");
        }
        assert!(
            play_args(&["--grid=129", "c"])
                .err()
                .unwrap()
                .to_string()
                .contains("2..=128")
        );
        let error = play_args(&["--interp=bilinear", "c"])
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("nearest")
                && error.contains("trilinear")
                && error.contains("tetrahedral"),
            "{error}"
        );
        // With no value at all the message names the option, as every option of
        // this command does.
        assert!(
            play_args(&["--grid", "c"])
                .err()
                .unwrap()
                .to_string()
                .contains("--grid")
        );
    }

    /// The panel says the three things a screen cannot show on its own: what the
    /// item states its colour to be, what light it carries, and what the grade
    /// does with the two.
    #[test]
    fn the_panel_says_what_a_picture_is_and_what_is_done_to_it() {
        let bt709 = ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        // The signal every screen expects is not worth a line of its own.
        assert!(colour_line(bt709).is_none());
        // Neither is a file that wrote no colour at all: three zeros are
        // H.273's way of saying unspecified, and calling matrix 0 "RGB" would
        // report a shape the file never claimed.
        assert!(colour_line(ColourDescription::default()).is_none());
        // The same triangle over the full range is, since the grade reads its
        // codes by a different rule.
        assert_eq!(
            colour_line(ColourDescription {
                full_range: true,
                ..bt709
            })
            .unwrap(),
            "Colour: BT.709 · BT.709 · BT.709 · full"
        );
        let hdr10 = ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: false,
        };
        assert_eq!(
            colour_line(hdr10).unwrap(),
            "Colour: BT.2020 · PQ · BT.2020 NCL · limited"
        );
        // A part the container leaves unstated is named as unstated, because it
        // is the part the grade has to answer for.
        assert_eq!(
            colour_line(ColourDescription {
                primaries: 2,
                transfer: 2,
                matrix: 2,
                full_range: false,
            })
            .unwrap(),
            "Colour: not stated · unknown · not stated · limited"
        );
        // A part the file does state, by a number this module cannot resolve, is
        // reported by that number: "not stated" would blame the file for saying
        // nothing when it said something fvid has no table for. The other two
        // halves of the same triple are named, because their numbers do land.
        assert_eq!(
            colour_line(ColourDescription {
                primaries: 13,
                transfer: 13,
                matrix: 13,
                full_range: true,
            })
            .unwrap(),
            "Colour: code 13 · sRGB · chroma-derived CL · full"
        );
        let display = MasteringDisplay::from_corners(
            (0.708, 0.292),
            (0.170, 0.797),
            (0.131, 0.046),
            (0.3127, 0.3290),
            1_000.0,
            0.005,
        )
        .unwrap();
        let hdr = HdrMetadata {
            mastering: Some(display),
            light: ContentLight {
                max_cll: 1_000.0,
                max_fall: 400.0,
            },
        };
        assert!(light_line(&HdrMetadata::default()).is_none());
        assert_eq!(
            light_line(&hdr).unwrap(),
            "HDR: BT.2020 · 0.005–1000 cd/m² · MaxCLL 1000 cd/m² · MaxFALL 400 cd/m²"
        );
        // A limit the file does not state is left off the line rather than
        // invented for it.
        assert_eq!(
            light_line(&HdrMetadata {
                mastering: Some(display),
                light: ContentLight::default(),
            })
            .unwrap(),
            "HDR: BT.2020 · 0.005–1000 cd/m²"
        );
        // A volume no table rounds onto is another panel, and the line says so
        // instead of naming the nearest one it half matches.
        let projector = MasteringDisplay::from_corners(
            (0.680, 0.320),
            (0.265, 0.690),
            (0.150, 0.060),
            (0.314, 0.351),
            500.0,
            0.001,
        )
        .unwrap();
        assert_eq!(
            light_line(&HdrMetadata {
                mastering: Some(projector),
                light: ContentLight::default(),
            })
            .unwrap(),
            "HDR: DCI-P3 · 0.001–500 cd/m²"
        );
        let odd = MasteringDisplay::from_corners(
            (0.680, 0.320),
            (0.200, 0.750),
            (0.150, 0.060),
            (0.3127, 0.3290),
            4_000.0,
            0.0001,
        )
        .unwrap();
        assert_eq!(
            light_line(&HdrMetadata {
                mastering: Some(odd),
                light: ContentLight::default(),
            })
            .unwrap(),
            "HDR: custom · 0.0001–4000 cd/m²"
        );
        // The grade line tells what is being done, which is not only what was
        // asked: HDR10 on the panel every session starts with compresses its
        // highlights although no shoulder was named, and the one picked is on
        // the line along with the grid it is baked on.
        let grade = Grading::default().grade_for(hdr10, &hdr).unwrap();
        let text = grade_text(&grade.plan(), grade.interpolation(), None);
        assert!(
            text.contains("PQ → BT.709")
                && text.contains("BT.2020 → BT.709")
                && text.contains("33³ tetrahedral"),
            "{text}"
        );
        // A look the session was given is on the same line, by the shape and
        // node count it was read as: a file's title does not survive the parse,
        // and how coarse the grid under the picture is what a viewer weighs.
        let looked = Grading {
            lut: Some(Lut::from_cube(IDENTITY_CUBE).unwrap()),
            ..Default::default()
        }
        .grade_for(hdr10, &hdr)
        .unwrap();
        let looked_text = grade_text(&looked.plan(), looked.interpolation(), looked.lut());
        assert!(looked_text.contains("look 2³"), "{looked_text}");
        let lifted = Grading {
            lut: Some(Lut::from_cube("LUT_1D_SIZE 2\n0 0 0\n1 1 1\n").unwrap()),
            ..Default::default()
        }
        .grade_for(hdr10, &hdr)
        .unwrap();
        let lifted_text = grade_text(&lifted.plan(), lifted.interpolation(), lifted.lut());
        assert!(lifted_text.contains("look 1D 2"), "{lifted_text}");
        // A log curve named at the door is the first word of the line, because
        // the grade reads the bytes as that curve rather than as the curve the
        // file states — the thing a viewer blames when a picture comes out flat.
        // It is the same word the option takes, so the panel and the command
        // line say the curve one way.
        let logged = Grading {
            log: Some(Log::SLog3),
            ..Default::default()
        }
        .grade_for(hdr10, &hdr)
        .unwrap();
        let logged_text = grade_text(&logged.plan(), logged.interpolation(), logged.lut());
        assert!(logged_text.starts_with("slog3 → BT.709"), "{logged_text}");
        // The sample file writes no colour element and asks for no grade, so the
        // panel shows only the picture's own line; the three lines are then put
        // on by hand to check the order the panel keeps them in.
        let directory = scratch("fvid-player-info-colour", &[]);
        let path = directory.join("chapters.mkv");
        std::fs::write(
            &path,
            include_bytes!("../tests/fixtures/chapters/chapters.mkv"),
        )
        .unwrap();
        let mut player = Player {
            queue: vec![path],
            ..Default::default()
        };
        player.play_index(0);
        assert!(player.error.is_none(), "{:?}", player.error);
        // The absence on the panel comes from the file stating nothing, not from
        // the helper dropping a signal the reader carried.
        assert_eq!(player.signal, ColourDescription::default());
        assert!(player.hdr.is_empty());
        assert!(player.grade_line.is_none());
        let plain = player.info_lines();
        assert_eq!(plain[0], "Video: VP9 · 16×16 · 4 fps");
        assert!(
            !plain
                .iter()
                .any(|line| line.starts_with("Colour:") || line.starts_with("Grade:")),
            "{plain:?}"
        );
        player.signal = hdr10;
        player.hdr = hdr;
        player.grade_line = Some(format!("Grade: {text}"));
        let lines = player.info_lines();
        assert_eq!(
            lines[1..4],
            [
                "Colour: BT.2020 · PQ · BT.2020 NCL · limited",
                "HDR: BT.2020 · 0.005–1000 cd/m² · MaxCLL 1000 cd/m² · MaxFALL 400 cd/m²",
                format!("Grade: {text}").as_str(),
            ]
        );
        drop(player);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// `--lut` reads the look at startup, from a file named either way round:
    /// `.cube` and `.3dl` are told apart by their contents, not their suffix, and
    /// a file that holds neither stops the command before a window opens.
    #[test]
    fn the_command_line_reads_the_lut_before_anything_is_opened() {
        let directory = std::env::temp_dir().join("fvid-player-lut-option");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let cube = directory.join("identity.cube");
        std::fs::write(&cube, IDENTITY_CUBE).unwrap();
        let parsed = play_args(&["--lut", cube.to_str().unwrap(), "c"]).unwrap();
        assert_eq!(
            parsed.grading.lut,
            Some(Lut::from_cube(IDENTITY_CUBE).unwrap())
        );
        // The same bytes under the wrong name are still a cube.
        let mislabelled = directory.join("identity.3dl");
        std::fs::write(&mislabelled, IDENTITY_CUBE).unwrap();
        assert!(
            play_args(&["--lut", mislabelled.to_str().unwrap(), "c"])
                .unwrap()
                .grading
                .lut
                .is_some()
        );
        let broken = directory.join("broken.cube");
        std::fs::write(&broken, "LUT_3D_SIZE 2\n0 0 0\n").unwrap();
        let error = play_args(&["--lut", broken.to_str().unwrap(), "c"])
            .err()
            .unwrap();
        // The file's own name is in the message, since that is what the caller
        // has to go and fix.
        assert!(error.to_string().contains("broken.cube"), "{error}");
        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// One request, several files: each item is graded from the signal it states
    /// for itself. BT.2100 material is compressed for the screen even when the
    /// session asked for nothing, a log file is read the way the camera recorded
    /// it, and a look over a stated curve is the look alone.
    #[test]
    fn each_item_is_graded_from_the_signal_it_states() {
        let empty = HdrMetadata::default();
        let video = ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        };
        // Nothing asked and nothing to undo: the bytes go to the screen as read.
        assert!(Grading::default().grade_for(video, &empty).is_none());
        assert!(
            Grading::default()
                .grade_for(ColourDescription::default(), &empty)
                .is_none()
        );
        // HDR10 states BT.2100 codes over BT.2020 primaries, which no desktop
        // panel shows unmapped, so the grade is on without being asked for.
        let hdr10 = ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: false,
        };
        let grade = Grading::default().grade_for(hdr10, &empty).unwrap();
        let plan = grade.plan();
        assert_eq!((plan.from, plan.to), (Transfer::Pq, Transfer::Bt709));
        assert_eq!(
            (plan.source, plan.dest),
            (Primaries::BT2020, Primaries::BT709)
        );
        assert_eq!(plan.tone_map, Some(ToneMap::Clip));
        // `--tonemap` replaces the curve the plan would have picked.
        let hable = Grading {
            tone_map: Some(ToneMap::Hable),
            ..Default::default()
        }
        .grade_for(hdr10, &empty)
        .unwrap();
        assert_eq!(hable.plan().tone_map, Some(ToneMap::Hable));
        // `--log` reads the codes as the camera wrote them and hands the panel
        // finished video instead; the file's own primaries still win over the
        // profile's working gamut, because they describe these bytes.
        let logged = Grading {
            log: Some(Log::VLog),
            ..Default::default()
        }
        .grade_for(video, &empty)
        .unwrap();
        let plan = logged.plan();
        assert_eq!(plan.log, Some(Log::VLog));
        assert_eq!((plan.from, plan.to), (Transfer::Linear, Transfer::Bt709));
        assert_eq!(plan.source, Primaries::BT709);
        assert!(plan.tone_map.is_none());
        // A look is asked for on its own, so the plan writes the item's own
        // signal back and the file's own curve becomes the destination. Nothing
        // in the grid moves: an identity look on any file is no change at all.
        let look = Grading {
            lut: Some(Lut::from_cube(IDENTITY_CUBE).unwrap()),
            ..Default::default()
        };
        let grade = look.grade_for(video, &empty).unwrap();
        let plan = grade.plan();
        assert_eq!((plan.from, plan.to), (Transfer::Bt709, Transfer::Bt709));
        assert_eq!(plan.source, plan.dest);
        assert!(plan.tone_map.is_none());
        assert!(grade.is_identity());
        // A file that names no curve is read as the codes come out of the
        // converter, so a look over it is still the look alone.
        let grade = look
            .grade_for(ColourDescription::default(), &empty)
            .unwrap();
        assert_eq!(grade.plan().from, grade.plan().to);
        assert!(grade.is_identity());
    }

    /// The chain end to end on a real file: this clip's light exists only as
    /// in-band SEI messages, which the decoder reads out of the configuration
    /// record as it is built, so a session that was told nothing still grades the
    /// file against the 1 000 cd/m² peak the encoder stated rather than a blind
    /// guess. That peak is also what the default curve is chosen from: the panel
    /// reaches 100 cd/m² and the master was authored at ten times that, so the
    /// session rolls its highlights instead of folding them to white, which a
    /// grade that never read the peak cannot show.
    #[test]
    fn a_files_in_band_light_reaches_the_grade_the_session_bakes() {
        use crate::playback_native::NativeReader;
        let data = include_bytes!("../tests/fixtures/hevc/hdr10.mp4").to_vec();
        let reader = NativeReader::without_memory_limit(std::io::Cursor::new(data)).unwrap();
        let (signal, hdr) = (reader.colour(), reader.hdr());
        assert!(signal.is_hdr());
        let grade = Grading::default()
            .grade_for(signal, &hdr)
            .expect("BT.2100 material is graded for the panel");
        assert_eq!(grade.plan().content.max_cll, 1_000.0);
        // Asked for by name, the same peak steers the curve that reads it.
        let reinhard = Grading {
            tone_map: Some(ToneMap::Reinhard),
            ..Default::default()
        }
        .grade_for(signal, &hdr)
        .unwrap();
        assert_eq!(reinhard.plan().content.max_cll, 1_000.0);
        let highlight = [1.0, 0.85, 0.7];
        let blind = Grading {
            tone_map: Some(ToneMap::Reinhard),
            ..Default::default()
        }
        .grade_for(signal, &HdrMetadata::default())
        .unwrap();
        assert_ne!(reinhard.rgb(highlight), blind.rgb(highlight));
        // The default reads the peak too, and that is the difference a viewer
        // sees: a code above the panel's own reach keeps its place in the
        // shoulder instead of folding into white.
        let blind_default = Grading::default()
            .grade_for(signal, &HdrMetadata::default())
            .unwrap();
        assert_eq!(grade.plan().tone_map, Some(ToneMap::Mobius));
        assert_eq!(blind_default.plan().tone_map, Some(ToneMap::Clip));
        // PQ's 0.62 is about 190 cd/m², which a 100-nit panel has no number for
        // until the master's own peak says how far above it to roll.
        let kept = grade.rgb([0.62; 3])[0];
        let burnt = blind_default.rgb([0.62; 3])[0];
        assert!(burnt > 0.999, "{burnt}");
        assert!((0.85..0.96).contains(&kept), "{kept}");
        // And a code the panel shows outright is untouched by the choice.
        assert_eq!(grade.rgb([0.3; 3]), blind_default.rgb([0.3; 3]));
        // The panel words the same decision, from the file's own bytes: the
        // corners are BT.2020 in the payload's multiples of 0.00002, which the
        // line names as a panel rather than leaving as an exact-compare miss.
        assert_eq!(
            colour_line(signal).unwrap(),
            "Colour: BT.2020 · PQ · BT.2020 NCL · limited"
        );
        assert_eq!(
            light_line(&hdr).unwrap(),
            "HDR: BT.2020 · 0.0001–1000 cd/m² · MaxCLL 1000 cd/m² · MaxFALL 400 cd/m²"
        );
        assert_eq!(
            grade_text(&grade.plan(), grade.interpolation(), grade.lut()),
            "PQ → BT.709 · BT.2020 → BT.709 · mobius · 100 cd/m² · 33³ tetrahedral"
        );
    }

    /// The same chain on an H.264 clip that states HLG in the only place the
    /// coding has for it: the VUI of the sequence parameter set inside the
    /// `avcC` record, with no `colr` atom in the file at all. The triple has to
    /// answer when the file opens, because that is when the grade is baked — a
    /// reader that waited for a picture to select its parameter set would show
    /// this clip as if its codes were BT.709.
    #[test]
    fn an_avc_records_vui_is_graded_at_open() {
        use crate::playback_native::NativeReader;
        let data = include_bytes!("../tests/fixtures/avc/hlg-vui-only.mp4").to_vec();
        let reader = NativeReader::without_memory_limit(std::io::Cursor::new(data)).unwrap();
        let (signal, hdr) = (reader.colour(), reader.hdr());
        assert!(signal.is_hdr());
        assert!(hdr.is_empty());
        let grade = Grading::default()
            .grade_for(signal, &hdr)
            .expect("BT.2100 material is graded for the panel");
        let plan = grade.plan();
        assert_eq!((plan.from, plan.to), (Transfer::Hlg, Transfer::Bt709));
        assert_eq!(plan.tone_map, Some(ToneMap::Clip));
        assert_eq!(
            colour_line(signal).unwrap(),
            "Colour: BT.2020 · HLG · BT.2020 NCL · limited"
        );
        assert_eq!(
            grade_text(&plan, grade.interpolation(), grade.lut()),
            "HLG → BT.709 · BT.2020 → BT.709 · clip · 100 cd/m² · 33³ tetrahedral"
        );
    }

    /// The same chain on an AV1 file that writes its signal in no container
    /// element at all: SVT-AV1 stated the BT.2020 triple in its sequence header
    /// and the HDR10 volume and a 1 234/567 light in metadata OBUs inside the
    /// first packet, and the muxer's `Colour` element holds one range flag. The
    /// grade a session bakes before showing a picture still reads the encoder's
    /// own peak, because the reader unpacks that packet while the file opens.
    /// And the coding's range flag replaces the container's, since a lone range
    /// with no triple beside it states no signal for the composition to keep.
    #[test]
    fn an_av1_tracks_in_band_signal_is_graded_at_open() {
        use crate::playback_native::NativeReader;
        let data = include_bytes!("../tests/fixtures/av1/hdr-in-band.mkv").to_vec();
        let reader = NativeReader::without_memory_limit(std::io::Cursor::new(data)).unwrap();
        let (signal, hdr) = (reader.colour(), reader.hdr());
        assert_eq!(
            signal,
            ColourDescription {
                primaries: 9,
                transfer: 16,
                matrix: 9,
                full_range: true,
            }
        );
        assert!(signal.is_hdr());
        assert_eq!(hdr.content_light(PANEL_NITS).max_cll, 1_234.0);
        let grade = Grading::default()
            .grade_for(signal, &hdr)
            .expect("BT.2100 material is graded for the panel");
        let plan = grade.plan();
        assert_eq!((plan.from, plan.to), (Transfer::Pq, Transfer::Bt709));
        assert_eq!(plan.tone_map, Some(ToneMap::Mobius));
        assert_eq!(plan.content.max_cll, 1_234.0);
        assert_eq!(
            colour_line(signal).unwrap(),
            "Colour: BT.2020 · PQ · BT.2020 NCL · full"
        );
        assert_eq!(
            light_line(&hdr).unwrap(),
            // The floor is AV1's own number, not ST 2086's: the volume writes
            // luminance in 18.14 fixed point, so the black this encoder means
            // arrives as 2/16 384 rather than the 0.0001 a `mdcv` box carries.
            // The name above is unaffected — the corners and the 1 000 cd/m²
            // peak decide it, and no floor is compared — and the line states
            // what the file says rather than rounding it toward the other
            // standard's quantum.
            "HDR: BT.2020 · 0.00012207031–1000 cd/m² · MaxCLL 1234 cd/m² · MaxFALL 567 cd/m²"
        );
        assert_eq!(
            grade_text(&plan, grade.interpolation(), grade.lut()),
            "PQ → BT.709 · BT.2020 → BT.709 · mobius · 100 cd/m² · 33³ tetrahedral"
        );
    }

    /// The same chain on a file that states a curve and no light at all: an HLG
    /// clip. Nothing was asked from the command line, yet BT.2100 material still
    /// gets the display-domain pass [`Grade::new`] picks — and because the panel's
    /// own peak is the only peak this file states, the headroom rule answers
    /// "nothing to roll", which for HLG is not a compromise but what the format
    /// means, its scene light being normalised to whatever display shows it. So
    /// the top of the code scale stays separated on the way to the screen instead
    /// of folding into white, and mid-code grey arrives as grey.
    #[test]
    fn an_hlg_item_is_graded_at_the_panels_own_peak() {
        use crate::playback_native::NativeReader;
        let data = include_bytes!("../tests/fixtures/hevc/hlg.mp4").to_vec();
        let reader = NativeReader::without_memory_limit(std::io::Cursor::new(data)).unwrap();
        let (signal, hdr) = (reader.colour(), reader.hdr());
        assert!(signal.is_hdr());
        assert!(hdr.is_empty());
        let grade = Grading::default()
            .grade_for(signal, &hdr)
            .expect("BT.2100 material is graded for the panel");
        let plan = grade.plan();
        assert_eq!(plan.from, Transfer::Hlg);
        assert_eq!(plan.to, Transfer::Bt709);
        assert_eq!(plan.tone_map, Some(ToneMap::Clip));
        assert_eq!(plan.content.max_cll, PANEL_NITS);
        // Four codes that a 1 000-cd/m² reading of this signal would have
        // crushed together into white stay apart, in order, with mid grey below
        // half the scale.
        let steps: Vec<f32> = [0.5f32, 0.65, 0.8, 1.0]
            .iter()
            .map(|c| grade.rgb([*c; 3])[0])
            .collect();
        assert!(steps[0] < 0.5, "{steps:?}");
        assert!(steps.windows(2).all(|w| w[1] > w[0] + 0.05), "{steps:?}");
        assert!((steps[3] - 1.0).abs() < 1e-3, "{steps:?}");
        // The panel's words for this file: HLG states no light, so only the
        // signal it does state is on the panel, and the grade line says which
        // display-domain pass it took and against what peak.
        assert_eq!(
            colour_line(signal).unwrap(),
            "Colour: BT.2020 · HLG · BT.2020 NCL · limited"
        );
        assert!(light_line(&hdr).is_none());
        assert_eq!(
            grade_text(&plan, grade.interpolation(), grade.lut()),
            "HLG → BT.709 · BT.2020 → BT.709 · clip · 100 cd/m² · 33³ tetrahedral"
        );
    }
}
