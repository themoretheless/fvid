use super::{
    Adjust, Aspect, NO_CROP, Repeat, SphericalView, VOLUME_MAX,
    skin as player_skin,
};
use crate::codec::ac3_decoder::CompressionMode;
use crate::color::{
    Chromaticity, ColourDescription, CubePlan, DisplayTarget, Grade, HdrMetadata, Interpolation,
    Log, Lut, MasteringDisplay, MatrixCoeff, Primaries, Settings, ToneMap, Transfer,
};
use eframe::egui;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

/// Open an empty player or a supported local Y4M, MP4/AVC or WebM/VP9/AV1 file.
/// Several inputs, or a playlist that names them, queue up: the player moves to
/// the next item when the picture runs out.
/// The bounds `--start-time` and `--stop-time` put on an item. They belong to
/// the first one only, so opening the next takes them out and leaves this.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(super) struct PlayBounds {
    pub(super) start: Option<Duration>,
    pub(super) stop: Option<Duration>,
}

/// The panel a session defaults to grading for: a desktop screen's diffuse
/// white, which is what an SDR player is asked to fill. `--display` replaces it
/// for a screen that reaches higher, or one that takes BT.2100 codes at all.
pub(super) const PANEL_NITS: f32 = 100.0;

/// The destinations `--display` offers, each with the codes its panel is fed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Panel {
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
    pub(super) fn target(self) -> DisplayTarget {
        match self {
            Self::Sdr(nits) => DisplayTarget::sdr(nits),
            Self::Pq(nits) | Self::Hlg(nits) => DisplayTarget::hdr(nits),
        }
    }

    /// The codes to write, when the panel's own kind decides them rather than
    /// the item.
    pub(super) fn destination(self) -> Option<(Transfer, Primaries)> {
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
pub(super) struct Grading {
    pub(super) content_light: Option<crate::color::ContentLight>,
    pub(super) mastering_luminance: Option<(f32, f32)>,
    /// Camera log curve the coded values carry.
    pub(super) log: Option<Log>,
    /// Highlight compression to run in place of the one the plan picks for the
    /// item.
    pub(super) tone_map: Option<ToneMap>,
    /// Grading LUT read out of the named file, applied after the conversion the
    /// other two ask for.
    pub(super) lut: Option<Lut>,
    /// The working gamut the coded values are read in, named instead of taken
    /// from the file or the curve.
    pub(super) gamut: Option<Primaries>,
    /// The panel the picture is graded for, named instead of assumed.
    pub(super) panel: Option<Panel>,
    /// Edge length of the grid the conversion is baked on, named instead of the
    /// 33 nodes every session starts with. A modifier of how a grade is computed
    /// rather than a request to change a picture, so it is not in [`Self::silent`].
    pub(super) grid: Option<usize>,
    /// How that grid and a `--lut` file are read between their nodes. Also a
    /// modifier: `nearest` on a coarse look keeps its steps instead of blending
    /// them away.
    pub(super) interp: Option<Interpolation>,
}

impl Grading {
    pub(super) fn metadata_for(&self, signal: ColourDescription, hdr: &HdrMetadata) -> HdrMetadata {
        let mut metadata = hdr.clone();
        if let Some(light) = self.content_light { metadata.light = light; }
        if let Some((min_luminance, max_luminance)) = self.mastering_luminance {
            let primaries = signal.primary_set().unwrap_or(Primaries::BT709);
            let mut volume = metadata.mastering.unwrap_or(MasteringDisplay {
                red: primaries.r, green: primaries.g, blue: primaries.b, white: primaries.white,
                min_luminance, max_luminance,
            });
            volume.min_luminance = min_luminance;
            volume.max_luminance = max_luminance;
            metadata.mastering = Some(volume);
        }
        metadata
    }

    /// Whether the command line named no colour change at all.
    pub(super) fn silent(&self) -> bool {
        self.log.is_none()
            && self.gamut.is_none()
            && self.panel.is_none()
            && self.tone_map.is_none()
            && self.lut.is_none()
            && self.content_light.is_none()
            && self.mastering_luminance.is_none()
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
    pub(super) fn grade_for(&self, signal: ColourDescription, hdr: &HdrMetadata) -> Option<Grade> {
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
        let metadata = self.metadata_for(signal, hdr);
        Some(Grade::new(signal, &metadata, settings, self.lut.clone()))
    }
}


/// Inputs and startup controls for the native player.
pub(super) struct PlayArgs {
    pub(super) spherical: SphericalView,
    pub(super) stereo3d: fvid_media::owned_spherical::PlayStereo3D,
    pub(super) no_subtitles: bool,
    pub(super) embedded_subtitle: Option<usize>,
    pub(super) audio_device: Option<String>,
    pub(super) dolby_compression: CompressionMode,
    pub(super) dolby_dialogue_normalization: bool,
    pub(super) eac3_stereo: Option<crate::codec::eac3_decoder::StereoDownmixMode>,
    pub(super) ac3_stereo: Option<crate::codec::ac3_decoder::StereoDownmixMode>,
    pub(super) list_audio_devices: bool,
    pub(super) fullscreen: bool,
    pub(super) on_top: bool,
    pub(super) quit_at_end: bool,
    pub(super) snapshot_dir: Option<PathBuf>,
    pub(super) gpu_backend: crate::Backend,
    pub(super) gpu_device: usize,
    pub(super) shader: Option<crate::player_gpu::ColorShader>,
    pub(super) skin: player_skin::Skin,
    pub(super) paths: Vec<PathBuf>,
    pub(super) start: Option<Duration>,
    pub(super) stop: Option<Duration>,
    pub(super) rate_milli: u32,
    pub(super) start_paused: bool,
    pub(super) no_audio: bool,
    /// Which audio track of each item to hear, counted from zero.
    pub(super) audio_track: Option<usize>,
    /// Which of each item's subtitle sources to read, counted from zero.
    pub(super) subtitle_track: Option<usize>,
    /// Output gain in thousandths the session starts at.
    pub(super) volume_milli: u32,
    pub(super) muted: bool,
    /// What happens when the queue runs out, as the command line asked.
    pub(super) repeat: Repeat,
    /// Whether the queue walks in a drawn order from the first frame, as
    /// `--random` asks for it.
    pub(super) shuffle: bool,
    /// Signed shifts in milliseconds the clocks start with.
    pub(super) audio_delay_ms: i64,
    pub(super) subtitle_delay_ms: i64,
    /// A subtitle file named on the command line, in front of the one the
    /// player would guess from the video's own name.
    pub(super) subtitle_file: Option<PathBuf>,
    /// Magnification in thousandths the session starts at, of the whole picture
    /// fitted into the frame.
    pub(super) zoom_milli: u32,
    /// The shape the session starts cropped to, as VLC's crop option writes it.
    pub(super) crop: (u32, u32),
    /// The shape the session starts drawn at, as VLC's aspect option writes it.
    pub(super) aspect: Aspect,
    /// The picture settings the session starts with. As in VLC, naming one of
    /// them is asking for the filter, so any of the five switches the dialog's
    /// own Enable on; naming none leaves the pixels alone until a key says
    /// otherwise.
    pub(super) adjust: Adjust,
    /// The colour the session starts graded with: the camera log to read the
    /// codes as, the highlight compression to run, and the LUT to apply after
    /// them. Naming any of the three asks for the grade; naming none leaves the
    /// picture as the reader hands it over, except on BT.2100 material, which an
    /// SDR panel cannot show unmapped.
    pub(super) grading: Grading,
}

/// Read the options `fvid play` answers, in either `--flag VALUE` or
/// `--flag=VALUE` form. Everything that is not an option joins the queue.
pub(super) fn parse_play_args(args: &[String]) -> crate::Result<PlayArgs> {
    let mut spherical = SphericalView::default();
    let mut stereo3d = fvid_media::owned_spherical::PlayStereo3D::Off;
    let mut gpu_backend = crate::Backend::Auto;
    let mut gpu_device = 0;
    let mut shader = None;
    let mut skin = player_skin::Skin::default();
    let mut paths = Vec::new();
    let mut start = None;
    let mut stop = None;
    let mut rate_milli = 1_000;
    let mut start_paused = false;
    let mut fullscreen = false;
    let mut on_top = false;
    let mut quit_at_end = false;
    let mut snapshot_dir = None;
    let mut audio_device = None;
    let mut dolby_compression = CompressionMode::Line;
    let mut dolby_dialogue_normalization = false;
    let mut eac3_stereo = Some(crate::codec::eac3_decoder::StereoDownmixMode::Automatic);
    let mut ac3_stereo = Some(crate::codec::ac3_decoder::StereoDownmixMode::Automatic);
    let mut list_audio_devices = false;
    let mut no_subtitles = false;
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
            "--no-subtitles" => { no_value(inline, flag)?; no_subtitles = true; }
            "--dolby-dialogue-normalization" => {
                dolby_dialogue_normalization=match option_value(args,&mut index,flag,inline)?.as_str() {
                    "on"=>true,"off"=>false,
                    _=>return Err(crate::invalid("--dolby-dialogue-normalization requires on or off")),
                };
            }
            "--eac3-stereo" => {
                use crate::codec::eac3_decoder::StereoDownmixMode as Mode;
                eac3_stereo=match option_value(args,&mut index,flag,inline)?.as_str() {
                    "native"=>None,"loro"=>Some(Mode::LoRo),"ltrt"=>Some(Mode::LtRt),"auto"=>Some(Mode::Automatic),
                    _=>return Err(crate::invalid("--eac3-stereo requires native, loro, ltrt or auto")),
                };
            }
            "--ac3-stereo" => {
                use crate::codec::ac3_decoder::StereoDownmixMode as Mode;
                ac3_stereo=match option_value(args,&mut index,flag,inline)?.as_str() {
                    "native"=>None,"loro"=>Some(Mode::LoRo),"ltrt"=>Some(Mode::LtRt),"auto"=>Some(Mode::Automatic),
                    _=>return Err(crate::invalid("--ac3-stereo requires native, loro, ltrt or auto")),
                };
            }
            "--dolby-compression" => {
                dolby_compression = match option_value(args, &mut index, flag, inline)?.as_str() {
                    "line" => CompressionMode::Line,
                    "rf" => CompressionMode::Rf,
                    _ => return Err(crate::invalid("--dolby-compression requires line or rf")),
                };
            }
            "--audio-device" => {
                let name = option_value(args, &mut index, flag, inline)?;
                if name.trim().is_empty() { return Err(crate::invalid("--audio-device requires a nonempty name")); }
                audio_device = Some(name);
            }
            "--list-audio-devices" => { no_value(inline, flag)?; list_audio_devices = true; }
            "--fullscreen" => { no_value(inline, flag)?; fullscreen = true; }
            "--on-top" => { no_value(inline, flag)?; on_top = true; }
            "--play-and-exit" => { no_value(inline, flag)?; quit_at_end = true; }
            "--snapshot-path" => { snapshot_dir = Some(PathBuf::from(option_value(args, &mut index, flag, inline)?)); }
            "--backend" => {
                gpu_backend = option_value(args, &mut index, flag, inline)?.parse()?;
                if matches!(gpu_backend, crate::Backend::Cpu | crate::Backend::Cuda) {
                    return Err(crate::invalid("player renderer requires auto, metal, vulkan, dx12 or gl"));
                }
            }
            "--device" => {
                gpu_device = option_value(args, &mut index, flag, inline)?.parse()
                    .map_err(|_| crate::invalid("invalid player GPU device"))?;
            }
            "--shader" => {
                use std::io::Read;
                if shader.is_some() { return Err(crate::invalid("only one display shader is supported")); }
                let path = option_value(args, &mut index, flag, inline)?;
                let mut source = String::new();
                File::open(path)?.take(64 * 1024 + 1).read_to_string(&mut source)?;
                shader = Some(crate::player_gpu::ColorShader::new(&source)?);
            }
            "--skin" => {
                let value = option_value(args, &mut index, flag, inline)?;
                skin = player_skin::Skin::load(&value)?;
            }
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
            "--sub-file" | "--subtitles" | "--subs" => {
                let value = option_value(args, &mut index, flag, inline)?;
                subtitle_file = Some(PathBuf::from(value));
            }
            "--spherical" | "--360" => {
                no_value(inline, flag)?;
                spherical.enabled = true;
            }
            "--spherical-stereo" => {
                let value = option_value(args, &mut index, flag, inline)?;
                spherical.stereo = fvid_media::owned_spherical::parse_spherical_stereo(&value).map_err(|e| crate::invalid(&e))?;
                spherical.enabled = true;
            }
            "--spherical-projection" => {
                let value = option_value(args, &mut index, flag, inline)?;
                spherical.projection = fvid_media::owned_spherical::parse_spherical_projection(&value).map_err(|e| crate::invalid(&e))?;
                spherical.enabled = true;
            }
            "--yaw" | "--pitch" | "--roll" | "--fov" => {
                let value = option_value(args, &mut index, flag, inline)?;
                let degrees = fvid_media::owned_spherical::parse_degrees_milli(&value).map_err(|e| crate::invalid(&e))?;
                match flag {
                    "--yaw" => spherical.yaw = degrees,
                    "--pitch" => spherical.pitch = degrees,
                    "--roll" => spherical.roll = degrees,
                    _ => spherical.fov = degrees,
                }
            }
            "--play-stereo3d" => {
                let value = option_value(args, &mut index, flag, inline)?;
                stereo3d = fvid_media::owned_spherical::parse_play_stereo3d(&value).map_err(|error| crate::invalid(&error))?;
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
            "--hdr-mastering" => {
                let value = option_value(args, &mut index, flag, inline)?;
                let (min_milli, max_nits) = fvid_media::play_math::parse_hdr_mastering_nits(&value).map_err(|e| crate::invalid(&e))?;
                let min_nits = min_milli as f32 / 1000.0;
                if max_nits as f32 <= min_nits { return Err(crate::invalid("HDR mastering maximum must exceed minimum")); }
                grading.mastering_luminance = Some((min_nits, max_nits as f32));
            }
            "--hdr-nits" => {
                let value = option_value(args, &mut index, flag, inline)?;
                let nits: u32 = value.parse().map_err(|_| crate::invalid("--hdr-nits requires positive integer nits"))?;
                if nits == 0 { return Err(crate::invalid("--hdr-nits requires positive integer nits")); }
                grading.panel = Some(Panel::Sdr(nits as f32));
            }
            "--hdr-maxcll" => {
                let value = option_value(args, &mut index, flag, inline)?;
                let (cll, fall) = fvid_media::play_math::parse_hdr_maxcll_maxfall(&value).map_err(|e| crate::invalid(&e))?;
                grading.content_light = Some(crate::color::ContentLight { max_cll: cll as f32, max_fall: fall as f32 });
            }
            "--display" => {
                let value = option_value(args, &mut index, flag, inline)?;
                grading.panel = Some(parse_panel(&value)?);
            }
            "--hdr-tonemap" | "--tonemap" => {
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
    if gpu_backend == crate::Backend::Auto && gpu_device != 0 {
        return Err(crate::invalid("--device requires an explicit player backend when not zero"));
    }
    Ok(PlayArgs {
        spherical,
        stereo3d,
        no_subtitles,
        embedded_subtitle: None,
        audio_device,
        dolby_compression,
        dolby_dialogue_normalization,
        eac3_stereo,
        ac3_stereo,
        list_audio_devices,
        fullscreen,
        on_top,
        quit_at_end,
        snapshot_dir,
        gpu_backend,
        gpu_device,
        shader,
        skin,
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
pub(super) fn no_value(inline: Option<&str>, flag: &str) -> crate::Result<()> {
    if inline.is_some() {
        return Err(crate::invalid(&format!("{flag} takes no value")));
    }
    Ok(())
}

/// A track number as the player's own messages say it, from one, kept as the
/// index from zero the lists are walked with.
pub(super) fn parse_track(text: &str, flag: &str) -> crate::Result<usize> {
    let invalid = || crate::invalid(&format!("invalid {flag} {text:?}"));
    let stated = text.trim().parse::<usize>().map_err(|_| invalid())?;
    let counted = stated.checked_sub(1).ok_or_else(invalid)?;
    Ok(counted)
}

/// The value of an option: glued to its flag with `=`, or standing after it.
pub(super) fn option_value(
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
pub(super) fn parse_clock(text: &str) -> crate::Result<Duration> {
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
pub(super) fn parse_rate(text: &str) -> crate::Result<u32> {
    let invalid = || crate::invalid(&format!("invalid rate {text:?}"));
    let rate = text.parse::<f64>().map_err(|_| invalid())?;
    if !(0.25..=4.0).contains(&rate) {
        return Err(invalid());
    }
    Ok((rate * 1_000.0).round() as u32)
}

/// `--volume` as VLC states it: a whole percent of the recorded level, up to
/// the 200% its slider reaches, kept as the thousandths the player holds.
pub(super) fn parse_volume(text: &str) -> crate::Result<u32> {
    let invalid = || crate::invalid(&format!("invalid volume {text:?}"));
    let percent = text.trim().parse::<u32>().map_err(|_| invalid())?;
    if percent > VOLUME_MAX / 10 {
        return Err(invalid());
    }
    Ok(percent * 10)
}

/// A clock shift in milliseconds, either way round, held to the ±10 s the
/// delay keys stop at: a start value asks for the same shift a key would leave.
pub(super) fn parse_delay(text: &str, flag: &str) -> crate::Result<i64> {
    let invalid = || crate::invalid(&format!("invalid {flag} {text:?}"));
    let millis = text.trim().parse::<i64>().map_err(|_| invalid())?;
    Ok(millis.clamp(-10_000, 10_000))
}

/// `--zoom` as a multiple of the fitted picture, within the span VLC's own
/// zoom controls cover: its lowest menu rung to the ceiling its scale keys
/// stop at. Kept as thousandths, which is also how the keys hold it.
pub(super) fn parse_zoom(text: &str) -> crate::Result<u32> {
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
pub(super) fn parse_setting(
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
pub(super) fn parse_crop(text: &str) -> crate::Result<(u32, u32)> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("none") {
        return Ok(NO_CROP);
    }
    parse_ratio(text, "crop")
}

/// `--aspect` as one of VLC's aspect-ratio choices: `default` for the shape the
/// file declares, `fill` for the window's own shape (its `Fill Window`), and a
/// shape split by `:` for everything else.
pub(super) fn parse_aspect(text: &str) -> crate::Result<Aspect> {
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
pub(super) fn parse_ratio(text: &str, flag: &str) -> crate::Result<(u32, u32)> {
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
pub(super) fn parse_log(text: &str) -> crate::Result<Log> {
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
pub(super) fn parse_gamut(text: &str) -> crate::Result<Primaries> {
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
pub(super) fn parse_panel(text: &str) -> crate::Result<Panel> {
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
pub(super) fn parse_nits(text: &str) -> crate::Result<f32> {
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
pub(super) fn parse_grid(text: &str) -> crate::Result<usize> {
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
pub(super) fn parse_interpolation(text: &str) -> crate::Result<Interpolation> {
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
pub(super) fn colour_line(signal: ColourDescription) -> Option<String> {
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
pub(super) fn mastered_on(display: &MasteringDisplay) -> &'static str {
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
pub(super) fn light_line(hdr: &HdrMetadata) -> Option<String> {
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
pub(super) fn grade_text(plan: &CubePlan, interpolation: Interpolation, lut: Option<&Lut>) -> String {
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
pub(super) fn parse_tone_map(text: &str) -> crate::Result<ToneMap> {
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
/// in every message about it. The five table formats — `.cube`, `.3dl`, `.dat`,
/// `.spi1d`, `.spi3d` — are told apart by their content, not their suffix.
pub(super) fn read_lut(path: &str) -> crate::Result<Lut> {
    const MAX_LUT_BYTES: usize = 16 * 1024 * 1024;
    let file = File::open(path)
        .map_err(|error| crate::invalid(&format!("cannot read --lut {path:?}: {error}")))?;
    let mut text = String::new();
    file.take((MAX_LUT_BYTES + 1) as u64)
        .read_to_string(&mut text)
        .map_err(|error| crate::invalid(&format!("cannot read --lut {path:?}: {error}")))?;
    if text.len() > MAX_LUT_BYTES {
        return Err(crate::invalid(&format!("bad --lut {path:?}: LUT file is too large")));
    }
    Lut::from_text(&text).map_err(|error| crate::invalid(&format!("bad --lut {path:?}: {error}")))
}

pub(super) fn parse_media_play_args(args: &[String]) -> crate::Result<PlayArgs> {
    let mut native = Vec::new();
    let mut embedded = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let (flag, inline) = arg.split_once('=').map_or((arg.as_str(), None), |(flag, value)| (flag, Some(value)));
        index += 1;
        if flag == "--audio-track" {
            let value = option_value(args, &mut index, flag, inline)?;
            let track = value.parse::<usize>().map_err(|_| crate::invalid("invalid media audio track"))?;
            native.push(format!("--audio-track={}", track.checked_add(1).ok_or_else(|| crate::invalid("media audio track overflow"))?));
        } else if flag == "--subtitle-track" {
            let value = option_value(args, &mut index, flag, inline)?;
            let track = value.parse::<i32>().map_err(|_| crate::invalid("invalid media subtitle track"))?;
            if track == -1 { native.push("--no-subtitles".into()); embedded = None; }
            else if track >= 0 {
                embedded = Some(track as usize);
                native.retain(|arg| arg != "--no-subtitles");
            } else { return Err(crate::invalid("media subtitle track must be -1 or nonnegative")); }
        } else {
            native.push(arg.clone());
            if inline.is_none() && matches!(flag,
                "--hdr-tonemap" | "--hdr-mastering" | "--hdr-nits" | "--hdr-maxcll" | "--spherical-stereo" | "--spherical-projection" | "--yaw" | "--pitch" | "--roll" | "--fov" | "--play-stereo3d" | "--audio-device" | "--dolby-compression" | "--dolby-dialogue-normalization" | "--eac3-stereo" | "--ac3-stereo" | "--snapshot-path" | "--subtitles" | "--sub-file" | "--subs"
                | "--backend" | "--device" | "--shader" | "--skin" | "--start-time" | "--stop-time"
                | "--rate" | "--audio-delay" | "--subtitle-delay" | "--volume" | "--zoom"
                | "--crop" | "--aspect" | "--brightness" | "--gamma" | "--saturation" | "--contrast"
                | "--hue" | "--log" | "--gamut" | "--display" | "--tonemap" | "--lut" | "--grid" | "--interp")
            {
                native.push(option_value(args, &mut index, flag, None)?);
            }
        }
    }
    let mut parsed = parse_play_args(&native)?;
    parsed.embedded_subtitle = embedded;
    Ok(parsed)
}

pub(super) fn play_viewport(fullscreen: bool, on_top: bool) -> egui::ViewportBuilder {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([480.0, 320.0])
        .with_title("FVid")
        .with_title_shown(false)
        .with_titlebar_shown(false)
        .with_fullsize_content_view(true)
        .with_drag_and_drop(true)
        .with_fullscreen(fullscreen);
    if on_top { viewport.with_always_on_top() } else { viewport }
}
