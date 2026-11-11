//! Window presentation for FVid's own video reader.
//!
//! The chrome follows the minimalist player design: a near-black window, the
//! picture in a rounded dark frame, a title block top-left, one large play
//! button in the middle while paused, and a thin progress line with round
//! controls along the bottom. Controls fade out while the video plays and
//! the pointer rests; any movement brings them back.

pub mod gpu;
mod captions;
mod clock;
mod config;
mod playlist;
mod projection;
mod session;
mod skin;
mod ui;

use skin as player_skin;

use crate::audio_thread::DolbyPlaybackPolicy;
use crate::color::{ColourDescription, Grade, HdrMetadata};
use crate::container::FileTags;
use crate::playback_native::Planar8;
use crate::playback_spool::SpoolHandle;
use crate::playback_thread::{Event, Frame, Playback};
use crate::subtitles::Cue;
#[cfg(test)]
use crate::subtitles;
#[cfg(test)]
use crate::playback_native::NativeReader;
#[cfg(test)]
use crate::playback_thread::Pixels;

use captions::*;
use clock::*;
use config::*;
use playlist::*;
use projection::*;
#[cfg(test)]
pub(crate) use projection::adjust_rgb;
pub use clock::AUDIO_SYNC_SLACK;
use session::*;
use ui::*;

use eframe::egui::{self, Vec2};
#[cfg(test)]
use eframe::egui::{Color32, FontId, Pos2, Rect};
use std::future::Future;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

type Dialog = Pin<Box<dyn Future<Output = Option<rfd::FileHandle>>>>;

pub fn run(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    run_parsed(parse_play_args(&args)?)
}
/// Whether the legacy CLI request is completely handled by the native parser.
pub fn accepts_media_play(args: &[String]) -> bool { parse_media_play_args(args).is_ok() }
/// Native entry for `media play`, retaining its zero-based track indices.
pub fn run_media(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    run_parsed(parse_media_play_args(&args)?)
}

fn run_parsed(parsed: PlayArgs) -> Result<(), Box<dyn std::error::Error>> {
    if parsed.list_audio_devices {
        for name in crate::audio::PlatformBackend::output_devices() { println!("{name}"); }
        return Ok(());
    }
    let gpu_configuration = crate::player_gpu::configuration(parsed.gpu_backend, parsed.gpu_device)?;
    let shader = parsed.shader;
    let mut app = Player {
        spherical: parsed.spherical,
        stereo3d: parsed.stereo3d,
        subtitle_shown: !parsed.no_subtitles,
        preferred_embedded_subtitle: parsed.embedded_subtitle,
        preferred_output: parsed.audio_device,
        dolby_policy: DolbyPlaybackPolicy {
            compression:parsed.dolby_compression,normalize_dialogue:parsed.dolby_dialogue_normalization,
            ac3_stereo_output:parsed.ac3_stereo.is_some(),
            ac3_stereo_downmix:parsed.ac3_stereo.map(|mode|crate::codec::ac3_decoder::StereoDownmixPolicy {mode,headroom:true}).unwrap_or_default(),
            eac3_stereo_output:parsed.eac3_stereo.is_some(),
            eac3_stereo_downmix:parsed.eac3_stereo.map(|mode|crate::codec::eac3_decoder::StereoDownmixPolicy {mode,headroom:true}).unwrap_or_default(),
            ..Default::default()
        },
        queue: expand_inputs(&parsed.paths),
        bounds: PlayBounds {
            start: parsed.start,
            stop: parsed.stop,
        },
        quit_at_end: parsed.quit_at_end,
        snapshot_dir: parsed.snapshot_dir,
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
        skin: parsed.skin,
        ..Default::default()
    };
    if app.shuffle {
        // VLC starts shuffling with the queue already read in: the cycle is
        // dealt to the list the command line named, so the first step is a
        // draw rather than the second item.
        app.start_shuffle();
    }
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: gpu_configuration,
        viewport: play_viewport(parsed.fullscreen, parsed.on_top),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "FVid",
        options,
        Box::new(move |cc| {
            if let Some(state) = cc.wgpu_render_state.as_ref() {
                crate::player_gpu::install_with_shader(state, shader.as_ref())?;
                #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
                { app.shared_surfaces = state.device.adapter_info().backend == eframe::wgpu::Backend::Metal
                    && state.device.features().contains(eframe::wgpu::Features::TEXTURE_FORMAT_16BIT_NORM); }
            } else {
                return Err("GPU renderer was not initialized".into());
            }
            app.asynchronous_open = true;
            if let Some(path) = app.queue.first().cloned() { app.try_open(path); }
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

struct Player {
    spherical: SphericalView,
    stereo3d: fvid_media::owned_spherical::PlayStereo3D,
    preferred_embedded_subtitle: Option<usize>,
    preferred_output: Option<String>,
    dolby_policy: DolbyPlaybackPolicy,
    quit_at_end: bool,
    snapshot_dir: Option<PathBuf>,
    next_file: Option<NextFile>,
    cache_pressure: bool,
    memory_check: Option<Instant>,
    prepared_source: Option<std::sync::mpsc::Receiver<PreparedSource>>,
    cache_packets: Vec<(u64, u64, Duration, Duration)>,
    loading_buffer: Option<Arc<std::sync::atomic::AtomicUsize>>,
    asynchronous_open: bool,
    loading_since: Option<Instant>,
    opening_stage: &'static str,
    opening: Option<std::sync::mpsc::Receiver<OpenProgress>>,
    open_progress: Option<std::sync::mpsc::Sender<OpenProgress>>,
    skin: player_skin::Skin,
    /// The decoding thread for the open file.
    playback: Option<Playback>,
    /// Audio playback thread for A/V sync (when audio track is present).
    audio: Option<crate::audio_thread::AudioPlayback>,
    /// Audio reached its end or failed, so it no longer gates presentation.
    audio_ended: bool,
    /// Clock that paced the currently displayed picture.
    picture_audio_clock: bool,
    /// Minimum published video phase and its raw source anchor during audio replacement.
    audio_clock_floor: Option<(Duration, Duration)>,
    /// Media position retained while a silent picture is paused.
    paused_picture_time: Option<Duration>,
    /// Media phase held until missing picture input becomes presentable.
    buffered_picture_time: Option<Duration>,
    /// Explicit terminal position until navigation or another item takes over.
    terminal_clock: Option<Duration>,
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
    rgb_frame: Option<(Arc<Vec<u8>>, [usize; 2])>,
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
    deferred_event: Option<Event>,
    buffering: bool,
    /// The local copy of a source too slow to stream its own item, and when the
    /// wait for its lead started. Nothing while the source keeps up.
    spool: Option<SpoolHandle>,
    spool_wait: Option<Instant>,
    seek_preview: bool,
    seek_target: Option<Duration>,
    /// Frame on screen when it is drawn by the GPU shader (planar), with the
    /// grade it still owes and that shader reads.
    video: Option<(Arc<Planar8>, u64, Option<Arc<Grade>>, u16)>,
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    video_surface: Option<(crate::playback_thread::SurfacePixels, u64)>,
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    shared_surfaces: bool,
    video_packed: Option<(Arc<crate::playback_native::PackedPlanar>, u64, Option<Arc<Grade>>, u16)>,
    /// Frame on screen when it arrived as packed RGB (Y4M, WebM).
    texture: Option<egui::TextureHandle>,
    /// The frame last shown, in whichever layout it came. The snapshot key
    /// writes this, so nothing is copied to keep it: the frame is moved here
    /// once its pixels have reached the screen.
    presented: Option<Frame>,
    presented_source: Option<Frame>,
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
    /// Whether the clock at the right end of the progress line counts what is
    /// left rather than the whole length; clicking it flips the two.
    remaining: bool,
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
    network_input: Option<Arc<fvid_media::owned_http::DownloadedInput>>,
    network_downloaded: Option<u64>,
    network_cancel: Option<NetworkDownloadCancel>,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            network_input: None,
            network_downloaded: None,
            network_cancel: None,
            spherical: SphericalView::default(),
            stereo3d: fvid_media::owned_spherical::PlayStereo3D::Off,
            preferred_embedded_subtitle: None,
            preferred_output: None,
            // This player routes Dolby to a known L/R device through CPAL.
            // Decode the signalled independent downmix rather than folding
            // extended replacement channels a second time in the device router.
            dolby_policy: DolbyPlaybackPolicy {
                ac3_stereo_output:true,
                ac3_stereo_downmix:crate::codec::ac3_decoder::StereoDownmixPolicy {mode:crate::codec::ac3_decoder::StereoDownmixMode::Automatic,headroom:true},
                eac3_stereo_output:true,
                eac3_stereo_downmix:crate::codec::eac3_decoder::StereoDownmixPolicy {mode:crate::codec::eac3_decoder::StereoDownmixMode::Automatic,headroom:true},
                ..Default::default()
            },
            quit_at_end: false,
            snapshot_dir: None,
            skin: player_skin::Skin::default(),
            playback: None,
            audio: None,
            audio_ended: false,
            picture_audio_clock: false,
            audio_clock_floor: None,
            paused_picture_time: None,
            buffered_picture_time: None,
            terminal_clock: None,
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
            deferred_event: None,
            next_file: None,
            cache_pressure: false,
            memory_check: None,
            prepared_source: None,
            cache_packets: Vec::new(),
            loading_buffer: None,
            asynchronous_open: false,
            loading_since: None,
            opening_stage: "Открытие файла",
            opening: None,
            open_progress: None,
            buffering: true,
            spool: None,
            spool_wait: None,
            seek_preview: false,
            seek_target: None,
            video: None,
            video_packed: None,
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            video_surface: None,
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            shared_surfaces: false,
            texture: None,
            presented: None,
            presented_source: None,
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
            remaining: false,
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

#[cfg(test)]
mod tests;
