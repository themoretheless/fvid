use super::{
    ChapterMark, PlayBounds, Player, SubtitleSource,
    captions::container_facts,
    clock::delayed_clock,
    config::{Grading, grade_text},
    playlist::track_step,
};
use crate::audio_thread::DolbyPlaybackPolicy;
use crate::color::{ColourDescription, HdrMetadata};
use crate::container::FileTags;
use crate::playback_native::NativeReader;
use crate::playback_spool::SpoolHandle;
use crate::playback_thread::Playback;
use crate::subtitles::Cue;
use eframe::egui;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub(super) struct NetworkDownloadCancel(pub(super) fvid_media::CancelFlag);
impl Drop for NetworkDownloadCancel {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Self-contained per-media-item session state built on a background worker and
/// installed atomically into [`Player`] without a dummy `Player::default()` or
/// field-by-field `std::mem::swap` chains.
pub(super) struct MediaSession {
    pub(super) cache_packets: Vec<(u64, u64, Duration, Duration)>,
    pub(super) playback: Option<Playback>,
    pub(super) audio: Option<crate::audio_thread::AudioPlayback>,
    pub(super) audio_ended: bool,
    pub(super) duration: Option<Duration>,
    pub(super) seekable: bool,
    pub(super) hardware: bool,
    pub(super) video_codec: &'static str,
    pub(super) bytes: Option<u64>,
    pub(super) dimensions: [usize; 2],
    pub(super) pixel_aspect: (u32, u32),
    pub(super) container_insets: [u32; 4],
    pub(super) period: Duration,
    pub(super) signal: ColourDescription,
    pub(super) hdr: HdrMetadata,
    pub(super) grade_line: Option<String>,
    pub(super) spool: Option<SpoolHandle>,
    pub(super) spool_wait: Option<Instant>,
    pub(super) opened: Option<PathBuf>,
    pub(super) audio_tracks: Vec<crate::audio::AudioTrack>,
    pub(super) audio_track: usize,
    pub(super) sound_codec: String,
    pub(super) cues: Vec<Cue>,
    pub(super) subtitle_name: String,
    pub(super) subtitle_sources: Vec<SubtitleSource>,
    pub(super) subtitle_source: usize,
    pub(super) chapters: Vec<ChapterMark>,
    pub(super) file_tags: FileTags,
    pub(super) name: String,
    pub(super) error: Option<String>,
    pub(super) paused: bool,
    pub(super) step: u32,
    pub(super) ended: bool,
    pub(super) stop_time: Option<Duration>,
    pub(super) buffering: bool,
    pub(super) network_input: Option<Arc<fvid_media::owned_http::DownloadedInput>>,
}

struct SessionOpenWorker {
    network_cancel: Option<NetworkDownloadCancel>,
    preferred_embedded_subtitle: Option<usize>,
    preferred_output: Option<String>,
    dolby_policy: DolbyPlaybackPolicy,
    grading: Grading,
    bounds: PlayBounds,
    no_audio: bool,
    preferred_audio: Option<usize>,
    preferred_subtitle: Option<usize>,
    subtitle_file: Option<PathBuf>,
    start_paused: bool,
    volume_milli: u32,
    muted: bool,
    rate_milli: u32,
    prepared_source: Option<std::sync::mpsc::Receiver<PreparedSource>>,
    open_progress: std::sync::mpsc::Sender<OpenProgress>,
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    shared_surfaces: bool,
}

impl SessionOpenWorker {
    fn open_session(self, path: PathBuf) -> crate::Result<MediaSession> {
        let mut host = Player {
            network_cancel: self.network_cancel,
            preferred_embedded_subtitle: self.preferred_embedded_subtitle,
            preferred_output: self.preferred_output,
            dolby_policy: self.dolby_policy,
            grading: self.grading,
            bounds: self.bounds,
            no_audio: self.no_audio,
            preferred_audio: self.preferred_audio,
            preferred_subtitle: self.preferred_subtitle,
            subtitle_file: self.subtitle_file,
            start_paused: self.start_paused,
            volume_milli: self.volume_milli,
            muted: self.muted,
            rate_milli: self.rate_milli,
            prepared_source: self.prepared_source,
            open_progress: Some(self.open_progress),
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            shared_surfaces: self.shared_surfaces,
            ..Default::default()
        };
        host.open(path)?;
        Ok(host.extract_session())
    }
}

pub(super) enum OpenProgress {
    Stage(&'static str),
    Download(u64),
    Buffer(Arc<std::sync::atomic::AtomicUsize>),
    Spool(Option<SpoolHandle>),
    Ready(crate::Result<MediaSession>),
}

pub(super) type PreparedSource = std::io::Result<(crate::playback_spool::Source, Option<SpoolHandle>)>;

pub(super) struct NextFile {
    pub(super) path: PathBuf,
    pub(super) source: std::sync::mpsc::Receiver<PreparedSource>,
    pub(super) cancelled: Arc<std::sync::atomic::AtomicBool>,
}

pub(super) fn prepare_source(path: &Path, cancelled: &std::sync::atomic::AtomicBool) -> PreparedSource {
    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        return Err(std::io::ErrorKind::Interrupted.into());
    }
    // Large files get a bounded local lead even on a fast source.
    let spool = crate::playback_spool::Spool::adaptive(path)?;
    let handle = spool.handle();
    handle.prioritize(0);
    Ok((crate::playback_spool::Source::Spooled(spool), Some(handle)))
}

/// How many bytes a path takes up, once the file system has answered for it.
pub(super) fn file_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|meta| meta.len())
}

/// The sound of the open item, named the way a viewer names it. Every
/// container files the same coding under its own tag — AAC is `mp4a` in MP4,
/// PCM is `sowt` in QuickTime and `A_PCM/INT/LIT` in Matroska, and Microsoft's
/// ADPCM reaches this build under the decoder's dispatch names — so one word
/// per coding needs the table the picture's `mp4_codec` is for it. A coding
/// none of the readers can produce arrives under the name its own container
/// gave it, because some name on the panel beats a blank one.
pub(super) fn sound_codec(tag: &str) -> String {
    match tag {
        "A_OGG/VORBIS_OPUS" => "Vorbis/Opus",
        "A_VORBIS" => "Vorbis",
        "A_MPEG/L3" => "MP3",
        "A_MPEG/L2" => "MP2",
        "A_FLAC" => "FLAC",
        "wavpack" | "A_WAVPACK4" => "WavPack",
        "A_ALAC" | "alac" => "ALAC",
        "A_AC3" | "A_AC3/BSID9" | "A_AC3/BSID10" | "ac-3" => "Dolby Digital",
        "A_EAC3" | "ec-3" => "Dolby Digital Plus",
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

/// One file format's claim on a path: open its nth audio track, or say the
/// file isn't ours. Each reader below is the whole of its format's arm: the
/// table only states the order they are tried in.
pub(super) struct AudioAttempt {
    pub(super) explicit: fn(&Path,usize)->Option<Box<dyn crate::audio::AudioStream>>,
    pub(super) preferred: Option<fn(&Path)->Option<Box<dyn crate::audio::AudioStream>>>,
}
impl AudioAttempt {
    pub(super) fn open(&self,path:&Path,selected:Option<usize>)->Option<Box<dyn crate::audio::AudioStream>> {
        match (selected,self.preferred) {
            (None,Some(open))=>open(path),
            (selected,_) => (self.explicit)(path,selected.unwrap_or(0)),
        }
    }
}
pub(super) fn preferred_mp4_audio_stream(path:&Path)->Option<Box<dyn crate::audio::AudioStream>> {
    crate::playback_mp4_audio::Mp4AudioReader::open(BufReader::new(crate::playback_spool::open_shared(path).ok()?),crate::container::mp4::Limits::default()).ok().map(boxed_stream)
}
pub(super) fn preferred_webm_audio_stream(path:&Path)->Option<Box<dyn crate::audio::AudioStream>> {
    crate::playback_webm_audio::WebmAudioReader::open(BufReader::new(crate::playback_spool::open_shared(path).ok()?),crate::container::webm::Limits::default()).ok().map(boxed_stream)
}

/// MP4 first: it is the container the video reader already opened for the
/// picture, so its audio track is the one most often asked for.
pub(super) fn mp4_audio_stream(path: &Path, nth: usize) -> Option<Box<dyn crate::audio::AudioStream>> {
    let file = crate::playback_spool::open_shared(path).ok()?;
    crate::playback_mp4_audio::Mp4AudioReader::<std::io::BufReader<crate::playback_spool::SharedFile>>::open_at(
        BufReader::new(file),
        crate::container::mp4::Limits::default(),
        nth,
    )
    .ok()
    .map(boxed_stream)
}

/// A Matroska file names its tracks with the same dispatch tags the
/// audio decoder answers for, and it follows the MP4 one the way the
/// picture's reader order does.
pub(super) fn webm_audio_stream(path: &Path, nth: usize) -> Option<Box<dyn crate::audio::AudioStream>> {
    let file = crate::playback_spool::open_shared(path).ok()?;
    crate::playback_webm_audio::WebmAudioReader::<std::io::BufReader<crate::playback_spool::SharedFile>>::open_at(
        BufReader::new(file),
        crate::container::webm::Limits::default(),
        nth,
    )
    .ok()
    .map(boxed_stream)
}

/// An AVI file keeps one program's audio in a stream of its own among the
/// file's several, so the track key counts those; its reader frames records
/// for the codings whose record counting it has measured, of which the
/// Microsoft-spelled ADPCM blocks are the ones this build decodes.
pub(super) fn avi_audio_stream(path: &Path, nth: usize) -> Option<Box<dyn crate::audio::AudioStream>> {
    let file = crate::playback_spool::open_shared(path).ok()?;
    crate::playback_avi_audio::AviAudioReader::open_at::<std::io::BufReader<crate::playback_spool::SharedFile>>(
        BufReader::new(file),
        crate::container::avi::Limits::default(),
        nth,
    )
    .ok()
    .map(boxed_stream)
}

/// A MIDI performance is one track of its own, so the second and later
/// track keys have nothing to land on.
pub(super) fn smf_audio_stream(path: &Path, nth: usize) -> Option<Box<dyn crate::audio::AudioStream>> {
    if nth > 0 {
        return None;
    }
    let file = crate::playback_spool::open_shared(path).ok()?;
    crate::playback_smf::SmfAudioReader::open::<std::io::BufReader<crate::playback_spool::SharedFile>>(
        BufReader::new(file),
        crate::container::smf::Limits::default(),
    )
    .ok()
    .map(boxed_stream)
}

/// A module is one performance too, and one track of it: every channel is
/// mixed into what is heard rather than offered separately.
pub(super) fn xm_audio_stream(path: &Path, nth: usize) -> Option<Box<dyn crate::audio::AudioStream>> {
    if nth > 0 {
        return None;
    }
    let file = crate::playback_spool::open_shared(path).ok()?;
    crate::playback_xm::XmAudioReader::open::<std::io::BufReader<crate::playback_spool::SharedFile>>(
        BufReader::new(file),
        crate::container::xm::Limits::default(),
    )
    .ok()
    .map(boxed_stream)
}

/// An Ogg file names its codec in the first bytes of its first packet
/// rather than in a field, and one program can be written as several
/// chains after each other, so it is still one track to choose.
pub(super) fn ogg_audio_stream(path: &Path, nth: usize) -> Option<Box<dyn crate::audio::AudioStream>> {
    crate::playback_ogg_catalog::Reader::open(path,nth).ok().map(boxed_stream)
}

/// A Wave file holds one run of samples, so as with the two performances
/// above there is no second track key to land on.
pub(super) fn wav_audio_stream(path: &Path, nth: usize) -> Option<Box<dyn crate::audio::AudioStream>> {
    if nth > 0 {
        return None;
    }
    let file = crate::playback_spool::open_shared(path).ok()?;
    crate::playback_wav::WavAudioReader::open::<std::io::BufReader<crate::playback_spool::SharedFile>>(
        BufReader::new(file),
        crate::playback_wav::Limits::default(),
    )
    .ok()
    .map(boxed_stream)
}

/// An AIFF file holds one run too, with its geometry stated once and in
/// big-endian, and its rate as an 80-bit extended number.
pub(super) fn aiff_audio_stream(path: &Path, nth: usize) -> Option<Box<dyn crate::audio::AudioStream>> {
    if nth > 0 {
        return None;
    }
    let file = crate::playback_spool::open_shared(path).ok()?;
    crate::playback_aiff::AiffAudioReader::open::<std::io::BufReader<crate::playback_spool::SharedFile>>(
        BufReader::new(file),
        crate::playback_aiff::Limits::default(),
    )
    .ok()
    .map(boxed_stream)
}

/// A Sun `.snd` header states only a geometry too, and reads even its
/// width through a table its consumers disagree about.
pub(super) fn au_audio_stream(path: &Path, nth: usize) -> Option<Box<dyn crate::audio::AudioStream>> {
    if nth > 0 {
        return None;
    }
    let file = crate::playback_spool::open_shared(path).ok()?;
    crate::playback_au::AuAudioReader::open::<std::io::BufReader<crate::playback_spool::SharedFile>>(
        BufReader::new(file),
        crate::playback_au::Limits::default(),
    )
    .ok()
    .map(boxed_stream)
}

/// Native headers distinguish WavPack before readers that ingest whole files.
/// A present correction companion is required to validate; errors never select
/// an implicit lossy fallback. Selecting .wvc resolves its .wv main file.
pub(super) fn wavpack_audio_stream(path:&Path,nth:usize)->Option<Box<dyn crate::audio::AudioStream>> {
    if nth!=0 {return None;}
    let main=if path.extension().is_some_and(|e|e.eq_ignore_ascii_case("wvc")) {
        [path.with_extension("wv"),path.with_extension("WV")].into_iter().find(|p|p.is_file())?
    }else{path.to_path_buf()};
    let file=crate::playback_spool::open_shared(&main).ok()?;
    let mut stream=crate::playback_wavpack::WavpackAudioReader::open(BufReader::new(file),Default::default()).ok()?;
    for extension in ["wvc","WVC"] {
        let companion=main.with_extension(extension);
        match std::fs::symlink_metadata(&companion) {
            Ok(_)=>{let file=crate::playback_spool::open_shared(&companion).ok()?;stream=stream.attach_correction(BufReader::new(file)).ok()?;break;},
            Err(e)if e.kind()==std::io::ErrorKind::NotFound=>{},
            Err(_)=>return None,
        }
    }
    Some(boxed_stream(stream))
}

/// A `.flac` file names itself with `fLaC` and states its geometry once,
/// but its frames carry no length, so the walk that lists them proves
/// each end by the checksum the frame holds.
pub(super) fn flac_audio_stream(path: &Path, nth: usize) -> Option<Box<dyn crate::audio::AudioStream>> {
    if nth > 0 {
        return None;
    }
    let file = crate::playback_spool::open_shared(path).ok()?;
    crate::playback_flac::FlacAudioReader::open::<std::io::BufReader<crate::playback_spool::SharedFile>>(
        BufReader::new(file),
        crate::playback_flac::Limits::default(),
    )
    .ok()
    .map(boxed_stream)
}

/// A bare MPEG audio file states nothing before its frames at all, so it
/// opens last: its first bytes are free to look like anything, and every
/// other reader here begins with a magic that says what it is.
pub(super) fn mp3_audio_stream(path: &Path, nth: usize) -> Option<Box<dyn crate::audio::AudioStream>> {
    if nth > 0 {
        return None;
    }
    let file = crate::playback_spool::open_shared(path).ok()?;
    crate::playback_mp3::Mp3AudioReader::open::<std::io::BufReader<crate::playback_spool::SharedFile>>(
        BufReader::new(file),
        crate::playback_mp3::Limits::default(),
    )
    .ok()
    .map(boxed_stream)
}

/// A bare Dolby Digital file states its geometry in every frame too, but
/// its frames open on a syncword no other reader here claims, so it is
/// offered alongside the MPEG one and only takes a file whose whole run of
/// frame lengths is measured out of Table 5.18.
pub(super) fn ac3_audio_stream(path: &Path, nth: usize) -> Option<Box<dyn crate::audio::AudioStream>> {
    let file = crate::playback_spool::open_shared(path).ok()?;
    crate::playback_ac3::Ac3AudioReader::open_track::<std::io::BufReader<crate::playback_spool::SharedFile>>(
        BufReader::new(file),
        crate::playback_ac3::Limits::default(),
        nth,
    )
    .ok()
    .map(boxed_stream)
}

/// An ADTS file names its geometry in its frame headers as the two
/// elementary readers above do, but it keeps no setup block, so its
/// reader synthesizes the one the MP4 decoder asks for. It is offered
/// last because its syncword is the loosest of the three: only a file
/// whose whole run of self-stated lengths fills it is taken.
pub(super) fn adts_audio_stream(path: &Path, nth: usize) -> Option<Box<dyn crate::audio::AudioStream>> {
    if nth > 0 {
        return None;
    }
    let file = crate::playback_spool::open_shared(path).ok()?;
    crate::playback_aac::AacAudioReader::open::<std::io::BufReader<crate::playback_spool::SharedFile>>(
        BufReader::new(file),
        crate::playback_aac::Limits::default(),
    )
    .ok()
    .map(boxed_stream)
}

/// Hand a reader to the player's one stream slot: every reader already
/// implements the trait, only the trait object's type must be stated.
pub(super) fn boxed_stream<R: crate::audio::AudioStream + 'static>(
    reader: R,
) -> Box<dyn crate::audio::AudioStream> {
    Box::new(reader)
}

/// The audio readers the player holds, in the order a file is offered to them.
/// A container that names itself with a magic goes first; the bare elementary
/// streams open last, because their first bytes are free to look like anything.
pub(super) const AUDIO_READERS: &[AudioAttempt] = &[
    AudioAttempt { explicit: wavpack_audio_stream, preferred: None },
    AudioAttempt { explicit: mp4_audio_stream, preferred: Some(preferred_mp4_audio_stream) },
    AudioAttempt { explicit: webm_audio_stream, preferred: Some(preferred_webm_audio_stream) },
    AudioAttempt { explicit: avi_audio_stream, preferred: None },
    AudioAttempt { explicit: smf_audio_stream, preferred: None },
    AudioAttempt { explicit: xm_audio_stream, preferred: None },
    AudioAttempt { explicit: ogg_audio_stream, preferred: None },
    AudioAttempt { explicit: wav_audio_stream, preferred: None },
    AudioAttempt { explicit: aiff_audio_stream, preferred: None },
    AudioAttempt { explicit: au_audio_stream, preferred: None },
    AudioAttempt { explicit: flac_audio_stream, preferred: None },
    AudioAttempt { explicit: mp3_audio_stream, preferred: None },
    AudioAttempt { explicit: ac3_audio_stream, preferred: None },
    AudioAttempt { explicit: adts_audio_stream, preferred: None },
];

impl Player {
    pub(super) fn open(&mut self, path: PathBuf) -> crate::Result<()> {
        let remote = path.to_str().is_some_and(fvid_media::owned_http::recognizes);
        let input = if remote {
            let progress = self.open_progress.clone().map(|send| fvid_media::ProgressHook::new(move |event| {
                let _ = send.send(OpenProgress::Download(event.payload_bytes));
            }));
            let options = fvid_media::owned_http::DownloadOptions { progress, cancel: self.network_cancel.as_ref().map(|cancel|cancel.0.clone()), ..Default::default() };
            Some(fvid_media::owned_http::DownloadedInput::download_media(path.to_str().unwrap(), &options)
                .map_err(|error| crate::invalid(&error))?)
        } else {
            self.network_input.as_ref().filter(|input|input.path() == path).cloned()
        };
        let local = input.as_ref().map_or_else(||path.clone(),|input|input.path().to_owned());
        self.open_local(local)?;
        self.network_input = input;
        if remote {
            self.name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        }
        Ok(())
    }

    pub(super) fn open_stage(&self, stage: &'static str) {
        if let Some(progress) = &self.open_progress { let _ = progress.send(OpenProgress::Stage(stage)); }
    }

    pub(super) fn open_local(&mut self, path: PathBuf) -> crate::Result<()> {
        self.open_stage("Ожидание данных файла");
        // Start every source through a bounded streaming lead. Do not read
        // the whole file or probe an unrelated cold range before decoding.
        let prepared = self.prepared_source.take().and_then(|receiver| receiver.recv().ok());
        let (source, spool) = if let Some(Ok(source)) = prepared {
            source
        } else {
            prepare_source(&path, &std::sync::atomic::AtomicBool::new(false))?
        };
        self.spool_wait = spool.is_some().then(Instant::now);
        self.spool = spool;
        if let Some(progress) = &self.open_progress {
            let _ = progress.send(OpenProgress::Spool(self.spool.clone()));
        }
        self.open_stage("Чтение индекса и первого кадра");
        let has_start_bound = self.bounds.start.is_some();
        let video = NativeReader::without_memory_limit(BufReader::with_capacity(
            crate::playback_spool::READ_AHEAD,
            source,
        ))
        .and_then(|mut reader| {
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            if self.shared_surfaces { reader.enable_shared_surfaces()?; }
            // An indexed start may recover a new independently coded sequence
            // past a damaged old picture. Do not decode frame zero first.
            if has_start_bound && reader.seekable() {
                Ok((reader, None))
            } else if let Some(first) = reader.read_frame_raw()? {
                Ok((reader, Some(first)))
            } else {
                Err(crate::invalid("video has no frames"))
            }
        });
        let (mut reader, first_frame) = match video {
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
        reader.warm_seeking(&path);
        // `--start-time` and `--stop-time` belong to the first item of the queue.
        let bounds = std::mem::take(&mut self.bounds);
        let first_frame = if let Some(start) = bounds.start {
            match reader.seek_raw(start)? {
                Some(first) => first,
                None => {
                    // An empty video track may still accompany playable sound.
                    // Keep its original bounded sound-only fallback.
                    drop(reader);
                    self.bounds = bounds;
                    if self.open_sound_only(&path) { return Ok(()); }
                    return Err(crate::invalid("start-time has no video frame"));
                }
            }
        } else {
            first_frame.ok_or_else(|| crate::invalid("video has no frames"))?
        };
        self.stop_time = bounds.stop;
        self.cache_packets = reader.cache_packets();
        self.prioritize_swap(bounds.start.unwrap_or(Duration::ZERO));
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
        self.picture_audio_clock = false;
        self.terminal_clock = None;
        self.audio_clock_floor = None;
        self.buffered_picture_time = None;
        self.paused_picture_time = None;
        // Drop the old thread before starting the new one.
        self.playback = None;
        self.audio = None;
        self.audio_ended = false;
        self.queued = None;
        self.deferred_event = None;
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
        self.playback = Some(Playback::start_from_frame(reader, first_frame, grade));
        if let Some(progress) = &self.open_progress {
            let _ = progress.send(OpenProgress::Buffer(self.playback.as_ref().unwrap().buffer_counter()));
        }
        // The picture of the item before this one is no longer on screen.
        self.presented = None;
        self.presented_source = None;
        // Try to start audio playback if the file has an audio track.
        self.opened = Some(path.clone());
        // The track list belongs to the file, so it starts empty again here; a
        // failed audio open leaves it empty rather than stale.
        self.audio_tracks = Vec::new();
        self.audio_track = 0;
        self.sound_codec = String::new();
        self.open_stage("Подготовка звука");
        self.open_item_audio(&path);
        // A nonzero start must anchor both clocks before the preroll releases
        // audio; otherwise the picture waits for sound to catch up from zero.
        if let (Some(start), Some(audio)) = (bounds.start, &mut self.audio) {
            audio.seek(start);
        }
        self.apply_audio_controls();
        self.open_stage("Чтение субтитров и метаданных");
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
    pub(super) fn open_sound_only(&mut self, path: &Path) -> bool {
        // The bounds belong to the first item of the queue, as they do for a
        // picture: the sound starts where the command said and stops there too.
        let bounds = std::mem::take(&mut self.bounds);
        self.stop_time = bounds.stop;
        self.playback = None;
        self.audio = None;
        self.seekable = false;
        self.audio_ended = false;
        self.queued = None;
        self.deferred_event = None;
        self.buffering = false;
        self.seek_preview = false;
        self.seek_target = None;
        self.presented = None;
        self.presented_source = None;
        self.video = None;
        self.video_packed = None;
                        #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
                        { self.video_surface = None; }
        self.texture = None;
        self.rgb_frame = None;
        self.interval = None;
        self.picture_audio_clock = false;
        self.terminal_clock = None;
        self.audio_clock_floor = None;
        self.buffered_picture_time = None;
        self.paused_picture_time = None;
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
    pub(super) fn item_open(&self) -> bool {
        self.playback.is_some() || self.audio.is_some()
    }

    /// With the timing switch on, say what the item's timeline turned out to be.
    pub(super) fn report_duration(&self, path: &Path) {
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
    /// thread. The file is offered to every audio reader the table
    /// [`AUDIO_READERS`] holds, in the order it holds them, until one reads a
    /// track out of it: the video reader accepts either main container and
    /// reports no codec information of its own.
    /// False when the file has no such track.
    pub(super) fn try_start_audio(&mut self, path: &Path, nth: usize) -> bool {
        self.try_start_audio_choice(path,Some(nth))
    }
    pub(super) fn try_start_audio_choice(&mut self, path: &Path, selected: Option<usize>) -> bool {
        let output = self.preferred_output.clone();
        self.try_start_audio_choice_with_backend(path, selected, move || {
            Box::new(crate::audio::PlatformBackend::with_output_device(output))
        })
    }

    pub(super) fn try_start_audio_choice_with_backend<F>(&mut self, path: &Path, selected: Option<usize>, backend: F) -> bool
    where F: FnOnce() -> Box<dyn crate::audio::AudioBackend> + Send + 'static {
        let mut stream = None;
        for attempt in AUDIO_READERS {
            if let Some(found) = attempt.open(path, selected) {
                stream = Some(found);
                break;
            }
        }
        let Some(stream) = stream else {
            return false;
        };
        self.start_audio_stream_with_backend(stream, selected, backend);
        true
    }

    pub(super) fn start_audio_stream_with_backend<F>(&mut self, stream: Box<dyn crate::audio::AudioStream>, selected: Option<usize>, backend: F)
    where F: FnOnce() -> Box<dyn crate::audio::AudioBackend> + Send + 'static {
        // The container lists every track it can hand to a decoder, so the track
        // key knows how far to walk without opening the file again.
        self.audio_tracks = stream.audio_tracks();
        // The tag is read from the stream itself rather than from the track
        // list: the decoder was built for it, so this is the coding actually
        // sounding, which is what the panel is asked to name.
        self.sound_codec = sound_codec(stream.codec());
        self.audio_track = selected.unwrap_or_else(||stream.audio_track_index());
        self.audio = Some(crate::audio_thread::AudioPlayback::start_with_dolby_policy(stream, self.dolby_policy, backend));
    }

    /// Start the sound of the item being opened, at the track the command line
    /// asked for. `--no-audio` starts nothing at all, and a number the file's
    /// own list does not reach leaves the first track, because that is the one
    /// there is to hear.
    pub(super) fn open_item_audio(&mut self, path: &Path) -> bool {
        if self.no_audio { return false; }
        match self.preferred_audio {
            Some(nth) => self.try_start_audio(path,nth) || self.try_start_audio(path,0),
            None => self.try_start_audio_choice(path,None),
        }
    }

    /// Hold the item the command line asked to hold. Space lets it go again; the
    /// one-frame step shows its first picture, so a paused start is not a blank
    /// window.
    pub(super) fn hold_start(&mut self) {
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
    pub(super) fn cycle_audio_track(&mut self, forward: bool) {
        let output = self.preferred_output.clone();
        self.cycle_audio_track_with_backend(forward, move || {
            Box::new(crate::audio::PlatformBackend::with_output_device(output))
        });
    }

    pub(super) fn cycle_audio_track_with_backend<F>(&mut self, forward: bool, backend: F)
    where F: FnOnce() -> Box<dyn crate::audio::AudioBackend> + Send + 'static {
        let len = self.audio_tracks.len();
        if len < 2 {
            self.show_osd(if len == 0 { "No audio" } else { "Audio 1/1" });
            return;
        }
        let Some(path) = self.opened.clone() else {
            return;
        };
        let nth = track_step(self.audio_track, len, forward);
        // Acquire the source while its current descriptor/decoder is alive.
        // Failure leaves the playing item intact; successful admission pins
        // the same live file even if its pathname moved or was replaced.
        let Some(stream) = AUDIO_READERS.iter().find_map(|attempt| attempt.open(&path, Some(nth))) else {
            self.show_osd("Audio track unavailable");
            return;
        };
        // Preserve pending/terminal transport intent first. Otherwise keep an
        // active source audio clock unchanged; applying its A/V shift twice
        // would skip or repeat sound. A picture-owned phase needs the inverse
        // shift, while sound-only transport never uses that video delay.
        let picture = self.playback.is_some();
        let inverse_delay = self.audio_delay_ms.saturating_neg();
        let source_phase = |at| if picture { delayed_clock(at, inverse_delay) } else { at };
        let at = self.seek_target
            .or_else(|| self.terminal_clock.map(source_phase))
            .or_else(|| self.audio.as_ref().filter(|_| !picture || !self.audio_ended)
                .map(|audio| {
                    let at = audio.transport_position();
                    self.audio_clock_floor.map_or(at, |(_, floor)| at.max(floor))
                }))
            .or_else(|| self.presentation_clock().map(source_phase));
        let clock_floor = if picture { self.presentation_clock().zip(at) } else { None };
        let playing = !self.paused && !self.ended && !self.buffering && !self.seek_preview;
        // Dropping the old handle stops its thread before the new one starts.
        self.audio = None;
        self.start_audio_stream_with_backend(stream, Some(nth), backend);
        self.audio_ended = false;
        self.audio_clock_floor = clock_floor;
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

    pub(super) fn try_open(&mut self, path: PathBuf) -> bool {
        if self.next_file.as_ref().is_some_and(|next| next.path == path) {
            self.prepared_source = self.next_file.take().map(|next| next.source);
        } else {
            self.clear_prefetch();
        }
        if self.asynchronous_open {
            self.begin_open(path);
            return true;
        }
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

    fn extract_session(&mut self) -> MediaSession {
        MediaSession {
            cache_packets: std::mem::take(&mut self.cache_packets),
            playback: self.playback.take(),
            audio: self.audio.take(),
            audio_ended: self.audio_ended,
            duration: self.duration.take(),
            seekable: self.seekable,
            hardware: self.hardware,
            video_codec: self.video_codec,
            bytes: self.bytes.take(),
            dimensions: self.dimensions,
            pixel_aspect: self.pixel_aspect,
            container_insets: self.container_insets,
            period: self.period,
            signal: self.signal,
            hdr: std::mem::take(&mut self.hdr),
            grade_line: self.grade_line.take(),
            spool: self.spool.take(),
            spool_wait: self.spool_wait.take(),
            opened: self.opened.take(),
            audio_tracks: std::mem::take(&mut self.audio_tracks),
            audio_track: self.audio_track,
            sound_codec: std::mem::take(&mut self.sound_codec),
            cues: std::mem::take(&mut self.cues),
            subtitle_name: std::mem::take(&mut self.subtitle_name),
            subtitle_sources: std::mem::take(&mut self.subtitle_sources),
            subtitle_source: self.subtitle_source,
            chapters: std::mem::take(&mut self.chapters),
            file_tags: std::mem::take(&mut self.file_tags),
            name: std::mem::take(&mut self.name),
            error: self.error.take(),
            paused: self.paused,
            step: self.step,
            ended: self.ended,
            stop_time: self.stop_time.take(),
            buffering: self.buffering,
            network_input: self.network_input.take(),
        }
    }

    fn apply_session(&mut self, session: MediaSession) {
        let MediaSession {
            cache_packets,
            playback,
            audio,
            audio_ended,
            duration,
            seekable,
            hardware,
            video_codec,
            bytes,
            dimensions,
            pixel_aspect,
            container_insets,
            period,
            signal,
            hdr,
            grade_line,
            spool,
            spool_wait,
            opened,
            audio_tracks,
            audio_track,
            sound_codec,
            cues,
            subtitle_name,
            subtitle_sources,
            subtitle_source,
            chapters,
            file_tags,
            name,
            error,
            paused,
            step,
            ended,
            stop_time,
            buffering,
            network_input,
        } = session;
        let stale = (
            std::mem::replace(&mut self.cache_packets, cache_packets),
            std::mem::replace(&mut self.playback, playback),
            std::mem::replace(&mut self.audio, audio),
            std::mem::replace(&mut self.spool, spool),
            std::mem::replace(&mut self.network_input, network_input),
        );
        self.audio_ended = audio_ended;
        self.duration = duration;
        self.seekable = seekable;
        self.hardware = hardware;
        self.video_codec = video_codec;
        self.bytes = bytes;
        self.dimensions = dimensions;
        self.pixel_aspect = pixel_aspect;
        self.container_insets = container_insets;
        self.period = period;
        self.signal = signal;
        self.hdr = hdr;
        self.grade_line = grade_line;
        self.spool_wait = spool_wait;
        self.opened = opened;
        self.network_downloaded = None;
        self.audio_tracks = audio_tracks;
        self.audio_track = audio_track;
        self.sound_codec = sound_codec;
        self.cues = cues;
        self.subtitle_name = subtitle_name;
        self.subtitle_sources = subtitle_sources;
        self.subtitle_source = subtitle_source;
        self.chapters = chapters;
        self.file_tags = file_tags;
        self.name = name;
        self.error = error;
        self.paused = paused;
        self.step = step;
        self.ended = ended;
        self.stop_time = stop_time;
        self.buffering = buffering;
        self.interval = None;
        self.picture_audio_clock = false;
        self.terminal_clock = None;
        self.audio_clock_floor = None;
        self.buffered_picture_time = None;
        self.paused_picture_time = None;
        self.deadline = Instant::now();
        self.apply_audio_controls();
        // Dispose of previous handles off the UI drawing thread.
        std::thread::spawn(move || drop(stale));
    }

    pub(super) fn begin_open(&mut self, path: PathBuf) {
        self.network_cancel = None;
        let network_cancel = NetworkDownloadCancel(fvid_media::CancelFlag::new());
        self.network_cancel = Some(network_cancel.clone());
        let (send, receive) = std::sync::mpsc::channel();
        let worker = SessionOpenWorker {
            network_cancel: Some(network_cancel),
            preferred_embedded_subtitle: self.preferred_embedded_subtitle,
            preferred_output: self.preferred_output.clone(),
            dolby_policy: self.dolby_policy,
            grading: self.grading.clone(),
            bounds: std::mem::take(&mut self.bounds),
            no_audio: self.no_audio,
            preferred_audio: self.preferred_audio,
            preferred_subtitle: self.preferred_subtitle,
            subtitle_file: self.subtitle_file.clone(),
            start_paused: self.start_paused,
            volume_milli: self.volume_milli,
            muted: self.muted,
            rate_milli: self.rate_milli,
            prepared_source: self.prepared_source.take(),
            open_progress: send.clone(),
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            shared_surfaces: self.shared_surfaces,
        };
        let old_video = self.playback.take();
        let old_audio = self.audio.take();
        let old_open = self.opening.take();
        let old_network_input = self.network_input.take();
        let old_spool = self.spool.take();
        self.network_downloaded = None;
        // Joining a reader blocked on a cloud mount belongs off the UI thread.
        std::thread::spawn(move || {
            drop(old_video);
            drop(old_audio);
            drop(old_open);
            drop(old_spool);
            drop(old_network_input);
        });
        self.opening = Some(receive);
        self.loading_since = Some(Instant::now());
        self.opening_stage = "Открытие файла";
        self.loading_buffer = None;
        self.spool = None;
        self.cache_packets.clear();
        self.duration = None;
        self.opened = None;
        self.seekable = false;
        self.file_tags = FileTags::default();
        self.chapters.clear();
        self.cues.clear();
        self.presented = None;
        self.presented_source = None;
        self.video = None;
        self.video_packed = None;
        #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
        {
            self.video_surface = None;
        }
        self.texture = None;
        self.rgb_frame = None;
        self.queued = None;
        self.deferred_event = None;
        self.seek_target = None;
        self.seek_preview = false;
        self.loop_a = None;
        self.loop_b = None;
        self.buffering = true;
        self.ended = false;
        self.error = None;
        self.name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.activity = Instant::now();
        std::thread::spawn(move || {
            let result = worker.open_session(path);
            let _ = send.send(OpenProgress::Ready(result));
        });
    }

    pub(super) fn poll_open(&mut self, ctx: &egui::Context) {
        loop {
            let Some(receiver) = &self.opening else {
                return;
            };
            let Ok(progress) = receiver.try_recv() else {
                ctx.request_repaint_after(Duration::from_millis(16));
                return;
            };
            match progress {
                OpenProgress::Stage(stage) => self.opening_stage = stage,
                OpenProgress::Download(bytes) => self.network_downloaded = Some(bytes),
                OpenProgress::Spool(spool) => {
                    self.spool = spool;
                    self.network_downloaded = None;
                }
                OpenProgress::Buffer(counter) => self.loading_buffer = Some(counter),
                OpenProgress::Ready(result) => {
                    self.network_cancel = None;
                    self.opening = None;
                    self.loading_since = None;
                    self.loading_buffer = None;
                    match result {
                        Ok(session) => self.apply_session(session),
                        Err(error) => {
                            self.error = Some(error.to_string());
                            self.buffering = false;
                        }
                    }
                    return;
                }
            }
        }
    }
}
