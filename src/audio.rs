//! Audio playback backend abstraction.
//!
//! The audio pipeline mirrors the video one: a background thread decodes
//! compressed packets into PCM samples, which are handed to a platform
//! backend for playback. The backend is responsible for clocking samples to
//! the audio hardware and keeping the playback position available for A/V
//! sync.

#[cfg(feature = "player")]
mod cpal_backend;

#[cfg(feature = "player")]
pub use cpal_backend::CpalBackend as PlatformBackend;

use std::time::Duration;

/// PCM sample format produced by decoders and consumed by backends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    /// Signed 16-bit little-endian interleaved samples.
    I16,
    /// Signed 32-bit little-endian interleaved samples.
    I32,
    /// 32-bit IEEE 754 little-endian interleaved samples.
    F32,
}

/// Description of a decoded audio stream.
#[derive(Clone, Copy, Debug)]
pub struct AudioSpec {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of interleaved channels.
    pub channels: u16,
    /// Sample format.
    pub format: SampleFormat,
}

impl AudioSpec {
    /// Bytes per interleaved frame (one sample per channel).
    pub fn frame_size(&self) -> usize {
        let sample = match self.format {
            SampleFormat::I16 => 2,
            SampleFormat::I32 | SampleFormat::F32 => 4,
        };
        sample * self.channels as usize
    }
}

/// A decoded audio packet ready for playback.
pub struct AudioPacket {
    /// PCM samples, interleaved by channel.
    pub data: Vec<u8>,
    /// Presentation timestamp in stream timebase units.
    pub pts: u64,
    /// Stream timebase: `pts * timebase_num / timebase_den` seconds.
    pub timebase_num: u32,
    pub timebase_den: u32,
}

impl AudioPacket {
    /// Presentation timestamp as a `Duration` from the stream start.
    pub fn presentation_time(&self) -> Duration {
        if self.timebase_den == 0 {
            return Duration::ZERO;
        }
        let nanos = (self.pts as u128)
            .saturating_mul(self.timebase_num as u128)
            .saturating_mul(1_000_000_000)
            / (self.timebase_den as u128);
        Duration::from_nanos(nanos as u64)
    }
}

/// Scale one sample by a volume level given in thousandths, where 1000 is the
/// recorded level. Above full scale the sample is clamped; use
/// [`apply_volume_frame`], which normalises the block instead of clipping it.
pub fn apply_volume(sample: f32, milli: u32) -> f32 {
    let scaled = sample * (milli as f32 / 1_000.0);
    if scaled.is_finite() {
        scaled.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Apply a volume level to interleaved frames, in place.
///
/// Cutting and unity are a plain gain, which cannot clip a decoded sample
/// already inside full scale. Boosting instead holds the block's loudest peak at
/// full scale and lifts everything below it, so a 200% request makes quiet
/// passages audible instead of turning loud ones into a square wave.
pub fn apply_volume_frame(samples: &mut [f32], milli: u32) {
    if milli == 1_000 {
        return;
    }
    let gain = milli as f32 / 1_000.0;
    let boost = milli > 1_000;
    let peak = if boost {
        samples
            .iter()
            .fold(0.0f32, |acc, sample| acc.max(sample.abs()))
            * gain
    } else {
        0.0
    };
    // A block whose peak still fits is left as a plain gain; only an overflowing
    // one is pulled back, so boosting never quietly attenuates a file.
    let applied = if peak > 1.0 { 1.0 / peak.max(1.0) } else { 1.0 };
    for sample in samples {
        *sample = if sample.is_finite() {
            (*sample * gain * applied).clamp(-1.0, 1.0)
        } else {
            0.0
        };
    }
}

/// Frames of decoded stream audio one callback of `output_frames` consumes at a
/// playback rate given in thousandths. The remainder stays in `phase` so the
/// average over time is exact, which is what keeps the audio clock in step with
/// the picture instead of drifting by a frame per callback.
pub fn scaled_frames(output_frames: usize, milli: u32, phase: u32) -> (usize, u32) {
    let total = output_frames as u64 * u64::from(milli) + u64::from(phase);
    (
        usize::try_from(total / 1_000).unwrap_or(usize::MAX),
        (total % 1_000) as u32,
    )
}

/// Input frames one segment of the pitch-preserving stretch reads.
const STRETCH_SEGMENT: usize = 512;

/// How many frames a segment hands over, which is also how many the next one
/// overlaps it under the crossfade that joins the two. The rest of a segment is
/// kept only as the reference the next start is fitted to.
const STRETCH_FADE: usize = 256;

/// How far from its nominal place a segment may start looking for the wave phase
/// that fits, in frames.
const STRETCH_TOLERANCE: usize = 256;

/// Step of that search, in frames. It sets what periods can be told apart, and
/// the search costs one correlation per step taken.
const STRETCH_SEARCH_STEP: usize = 4;

/// Time stretch that keeps the pitch.
///
/// The plain rate path makes audio shorter by reading the stream faster, so
/// every pitch rises with the speed. This one cuts the stream into segments of
/// [`STRETCH_FADE`] frames played at the rate they were recorded at — which is
/// why the pitch holds — and changes only the distance between their starts:
/// [`Stretch::hop`] frames apart in the stream to shorten the audio, closer
/// together to lengthen it. A start landing in the middle of a wave would be a
/// click, so each one is moved by up to [`STRETCH_TOLERANCE`] frames to
/// wherever the segment's head continues the tail of the one before, and the
/// joint disappears under that crossfade.
///
/// What the stretch retires is what the playback clock runs on, so it is driven
/// by the rate the caller asked for rather than by the places the segments
/// happen to fit: the playhead moves by [`Stretch::hop`] every segment, and a
/// start moved by the search shifts the wave inside the audio, not the audio
/// along the stream. The retirement stays one tolerance behind that place, which
/// is as far back as the next segment can still reach.
pub struct Stretch {
    /// Interleaved channels of the stream being stretched.
    channels: usize,
    /// Playback rate in thousandths, which sets the distance between segments.
    milli: u32,
    /// Unfinished stream frame of that distance, carried between segments the
    /// way the plain rate path carries it, so the average stays exact.
    phase: u32,
    /// Frame the next segment starts searching around.
    nominal: usize,
    /// Frames retired: everything before them is out of reach of the search.
    retired: usize,
    /// The tail of the last segment as it was cut, which the next is fitted to
    /// and faded out under. Empty before the first segment.
    held: Vec<f32>,
    /// The segment being served, and how much of it the caller has taken.
    segment: Vec<f32>,
    taken: usize,
}

impl Stretch {
    /// A stretch of interleaved `channels` audio at `milli`/1000 of the recorded
    /// rate.
    pub fn new(channels: usize, milli: u32) -> Self {
        Self {
            channels,
            milli,
            phase: 0,
            nominal: 0,
            retired: 0,
            held: Vec::new(),
            segment: Vec::new(),
            taken: STRETCH_FADE,
        }
    }

    /// Move to another rate without disturbing the playhead: only the distance
    /// between segment starts changes.
    pub fn set_rate(&mut self, milli: u32) {
        self.milli = milli;
    }

    /// Stream frames, counted from [`Stretch::retired`], that
    /// [`Stretch::fill`] needs in hand to give out `frames` of output. A shorter
    /// window costs only the segments it cannot cut: the stretch still hands over
    /// what it can.
    pub fn lookahead(&self, frames: usize) -> usize {
        (frames / STRETCH_FADE + 2) * self.hop() + STRETCH_SEGMENT + 2 * STRETCH_TOLERANCE
    }

    /// Stream frames retired so far, counted from the first frame the stretch
    /// was ever given. The caller moves its read position here after each call.
    pub fn retired(&self) -> usize {
        self.retired
    }

    /// How far apart two segment starts are in the stream.
    fn hop(&self) -> usize {
        scaled_frames(STRETCH_FADE, self.milli, 0).0
    }

    /// Fill `output` with stretched frames, reporting how many elements went in.
    ///
    /// `input` is interleaved stream audio whose frame `0` stands at the absolute
    /// frame `base`, which is where [`Stretch::retired`] was when the window was
    /// taken. Output shorter than asked for means the window ran out.
    pub fn fill(&mut self, base: usize, input: &[f32], output: &mut [f32]) -> usize {
        let channels = self.channels;
        let mut written = 0;
        while written < output.len() {
            if self.taken >= STRETCH_FADE && !self.build(base, input) {
                break;
            }
            // Room and segment are counted in frames; a leftover element of an
            // incomplete last frame stays for the caller's next window.
            let take = ((output.len() - written) / channels).min(STRETCH_FADE - self.taken);
            if take == 0 {
                break;
            }
            let from = self.taken * channels;
            let to = from + take * channels;
            output[written..written + take * channels].copy_from_slice(&self.segment[from..to]);
            written += take * channels;
            self.taken += take;
        }
        written
    }

    /// Cut the next segment into `self.segment`, saying whether the window
    /// reached far enough for it.
    fn build(&mut self, base: usize, input: &[f32]) -> bool {
        let channels = self.channels;
        let available = input.len() / channels;
        let first = self.held.is_empty();
        // The first segment has nothing to fit to, so it starts where the window
        // does; every later one is searched around the place the rate asks for.
        let start = if first {
            self.retired.max(base)
        } else {
            match self.align(base, input, available) {
                Some(start) => start,
                None => return false,
            }
        };
        if (start - base) + STRETCH_SEGMENT > available {
            return false;
        }
        let head = (start - base) * channels;
        let end = head + STRETCH_SEGMENT * channels;
        self.segment.resize(STRETCH_FADE * channels, 0.0);
        // Both halves of the joint keep their own samples and only their weights
        // change, so the splice leaves the loudness where it was.
        for frame in 0..STRETCH_FADE {
            let mix = 0.5 - 0.5 * (std::f32::consts::PI * frame as f32 / STRETCH_FADE as f32).cos();
            for channel in 0..channels {
                let at = frame * channels + channel;
                self.segment[at] = if first {
                    input[head + at]
                } else {
                    self.held[at] * (1.0 - mix) + input[head + at] * mix
                };
            }
        }
        // The reference is the unblended tail, so a segment cut at the place the
        // rate asks for hands the stream's own samples back.
        self.held = input[end - STRETCH_FADE * channels..end].to_vec();
        let (hop, carry) = scaled_frames(STRETCH_FADE, self.milli, self.phase);
        self.phase = carry;
        // The next place is the rate's own progression: the start the search
        // found moves the wave inside the audio, not the audio along the stream,
        // and a wave that repeats itself at a distance would otherwise drag the
        // playhead onto its period and change the speed asked for. Only a window
        // beginning beyond the progression pulls it forward.
        self.nominal = self.nominal.max(base) + hop;
        // Frames behind the reach of the next search can go, and the retirement
        // never runs back into material the reader has already played.
        self.retired = self
            .retired
            .max(self.nominal.saturating_sub(STRETCH_TOLERANCE));
        self.taken = 0;
        true
    }

    /// Where the next segment starts, or nothing when the window holds no place
    /// the search could take: within the tolerance of its nominal place, the
    /// start whose head leaves the smallest error against the held tail.
    ///
    /// Candidates are visited outward from the nominal place and only a strict
    /// improvement moves the choice, so a signal that repeats itself at a
    /// distance - a pure tone - keeps the place the rate asked for instead of
    /// sliding to an equally fitting one.
    fn align(&self, base: usize, input: &[f32], available: usize) -> Option<usize> {
        let lowest = self
            .nominal
            .saturating_sub(STRETCH_TOLERANCE)
            .max(self.retired)
            .max(base);
        // A place the search may take is one a whole segment still fits at, so a
        // short window costs the caller only the segments it cannot cut.
        let highest = (self.nominal + STRETCH_TOLERANCE)
            .min(base + available.checked_sub(STRETCH_SEGMENT)?)
            .max(lowest);
        if highest - lowest < STRETCH_SEARCH_STEP {
            return None;
        }
        let wanted = self.nominal.clamp(lowest, highest);
        let mut best = wanted;
        let mut best_error = self.error(wanted, base, input);
        for step in (STRETCH_SEARCH_STEP..=STRETCH_TOLERANCE).step_by(STRETCH_SEARCH_STEP) {
            for candidate in [wanted.wrapping_sub(step), wanted + step] {
                if candidate < lowest || candidate > highest {
                    continue;
                }
                let error = self.error(candidate, base, input);
                if error < best_error {
                    best_error = error;
                    best = candidate;
                }
            }
        }
        Some(best)
    }

    /// How badly the head of a candidate start matches the held tail.
    fn error(&self, candidate: usize, base: usize, input: &[f32]) -> f64 {
        let head = (candidate - base) * self.channels;
        self.held
            .iter()
            .enumerate()
            .map(|(element, held)| {
                let step = f64::from(*held) - f64::from(input[head + element]);
                step * step
            })
            .sum()
    }
}

/// Errors that can occur during audio playback.
#[derive(Debug)]
pub enum AudioError {
    /// The backend failed to initialize (device unavailable, format unsupported).
    InitFailed(String),
    /// The backend encountered an error during playback.
    PlaybackError(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::InitFailed(msg) => write!(f, "audio init failed: {msg}"),
            AudioError::PlaybackError(msg) => write!(f, "audio playback error: {msg}"),
        }
    }
}

impl std::error::Error for AudioError {}

/// Platform audio backend. Implementations manage the audio device and buffer
/// packets internally; the caller pushes packets as they arrive from the
/// decoder and queries the current playback position for sync.
pub trait AudioBackend {
    /// Open the device for the given stream description. Called once before
    /// any packets are pushed. The device does not have to clock samples yet —
    /// `resume` starts the clock, which is how a caller keeps the audio clock
    /// anchored to the moment it starts presenting.
    fn start(&mut self, spec: AudioSpec) -> Result<(), AudioError>;

    /// Push a decoded packet for playback. The backend takes ownership and
    /// schedules the samples according to the packet's presentation time.
    fn push(&mut self, packet: AudioPacket) -> Result<(), AudioError>;

    /// Current playback position on the stream timeline. Used for
    /// A/V sync: the video presentation waits until the audio clock has
    /// reached the frame's PTS.
    fn position(&self) -> Duration;

    /// Discard all buffered packets and re-anchor the clock at `at`. Called
    /// after a seek or rewind, so `position` keeps reporting stream time
    /// rather than the elapsed time since the buffer was drained.
    fn flush(&mut self, at: Duration) -> Result<(), AudioError>;

    /// Pause playback. The backend keeps its buffers but stops clocking
    /// samples.
    fn pause(&mut self) -> Result<(), AudioError>;

    /// Run the device clock from its current position. This is the first start
    /// as well as every restart after a pause: between `start` and here the
    /// clock has not advanced at all.
    fn resume(&mut self) -> Result<(), AudioError>;

    /// Stop playback and release the audio device. The backend may be
    /// restarted with a new `start` call.
    fn stop(&mut self) -> Result<(), AudioError>;

    /// Output gain in thousandths, 1000 being the recorded level. A backend
    /// without a hardware volume control simply keeps silence at the right
    /// level until it can apply one.
    fn set_volume(&mut self, _milli: u32) -> Result<(), AudioError> {
        Ok(())
    }

    /// Consumption rate in thousandths: the device pulls this much stream audio
    /// per unit of its own time. Backends that cannot resample keep playing at
    /// the recorded rate.
    fn set_rate(&mut self, _milli: u32) -> Result<(), AudioError> {
        Ok(())
    }
}

/// A no-op backend used when audio is disabled or unsupported. All operations
/// succeed immediately; `position` follows the pushed packets and the flushes
/// exactly, so it mirrors what a real device clock would report.
pub struct NullBackend {
    position: Duration,
}

impl Default for NullBackend {
    fn default() -> Self {
        Self {
            position: Duration::ZERO,
        }
    }
}

impl AudioBackend for NullBackend {
    fn start(&mut self, _: AudioSpec) -> Result<(), AudioError> {
        Ok(())
    }

    fn push(&mut self, packet: AudioPacket) -> Result<(), AudioError> {
        self.position = packet.presentation_time();
        Ok(())
    }

    fn position(&self) -> Duration {
        self.position
    }

    fn flush(&mut self, at: Duration) -> Result<(), AudioError> {
        self.position = at;
        Ok(())
    }

    fn pause(&mut self) -> Result<(), AudioError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), AudioError> {
        Ok(())
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        Ok(())
    }
}

/// One encoded audio access unit read from a container. Timestamps stay in the
/// source's own timescale, which the stream reports separately.
pub struct EncodedPacket {
    pub data: Vec<u8>,
    pub pts: i64,
    pub duration: i64,
}

/// An audio track a container offers, in the container's own order. Only the
/// tracks the player can decode are listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioTrack {
    pub sample_rate: u32,
    pub channels: u16,
    /// What the container calls the track, empty when it names none.
    pub name: String,
    /// The language the container states, empty when it states none or says
    /// that none was said.
    pub language: String,
}

impl AudioTrack {
    /// How the player names a track on screen: the title the file gives it, or
    /// else its language, and then the layout and rate the track actually
    /// carries. The layout stays in every label because two tracks can share a
    /// name and still be a mono and a 5.1 version of the same thing.
    pub fn label(&self) -> String {
        let layout = format!("{} ch {} Hz", self.channels, self.sample_rate);
        let title = if !self.name.is_empty() {
            &self.name
        } else {
            &self.language
        };
        if title.is_empty() {
            layout
        } else {
            format!("{title} · {layout}")
        }
    }
}

/// Container-side audio source: stream metadata plus a cursor over encoded
/// packets that can be rewound and seeked. Implemented per container so the
/// decode thread stays independent of both the demultiplexer and the codec.
pub trait AudioStream: Send {
    /// Codec tag, e.g. `mp4a` or `A_VORBIS`.
    fn codec(&self) -> &str;
    /// Ticks per second used by packet timestamps.
    fn timescale(&self) -> u32;
    fn sample_rate(&self) -> u32;
    fn channels(&self) -> u16;
    /// Bits per sample as the container declares them, 0 when it says nothing.
    /// Only uncompressed PCM needs it; the compressed codecs carry the depth in
    /// their setup data.
    fn bits_per_sample(&self) -> u16 {
        0
    }
    /// Length of the track, for a listener with no picture to take the timeline
    /// from. `None` when the container states nothing the reader can trust.
    fn duration(&self) -> Option<Duration> {
        None
    }
    /// Optional exact decoded sample window for a container packet. The decoder
    /// must consume the whole access unit before the player trims its PCM tail.
    /// Containers with approximate durations leave this unspecified.
    fn packet_sample_limit(&self, _duration: u64) -> crate::Result<Option<usize>> {
        Ok(None)
    }
    /// Apply container presentation edits after decoding and packet-tail trimming.
    /// `None` consumes preroll without handing samples to the device.
    fn present_decoded(&self, packet: AudioPacket, _source_pts: i64) -> crate::Result<Option<AudioPacket>> {
        Ok(Some(packet))
    }
    /// Codec setup data, already in the form the decoder expects.
    fn extra_data(&self) -> &[u8];
    /// Every audio track of this container the player can decode, in container
    /// order. The track being read is one of them, so a longer list is what
    /// makes the track key worth having.
    fn audio_tracks(&self) -> Vec<AudioTrack>;
    /// Next encoded packet, or `None` at end of stream.
    fn next_packet(&mut self) -> crate::Result<Option<EncodedPacket>>;
    fn rewind(&mut self);
    /// Move the cursor to the last packet at or before `pts` and report the
    /// timestamp actually landed on.
    fn seek_to(&mut self, pts: i64) -> i64;

    /// Wall-clock time of a packet timestamp.
    fn time_of(&self, pts: i64) -> Duration {
        let timescale = self.timescale();
        if timescale == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(pts.max(0) as f64 / f64::from(timescale))
    }
}

/// Decoder half of the audio pipeline: turn encoded packets into PCM.
pub trait AudioDecode: Send {
    /// Decode one access unit. `Ok(None)` means the decoder consumed the packet
    /// without producing output, so the caller should feed it another one.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        duration: u64,
    ) -> crate::Result<Option<AudioPacket>>;

    /// Drop codec state, e.g. after a seek.
    fn reset(&mut self);
}

/// Registry entry for an audio codec: its dispatch tag(s) + name.
pub struct CodecEntry {
    pub tags: &'static [&'static str],
    pub name: &'static str,
}

// Registry of known audio codec tags with their display names.
const CODEC_ENTRIES: &[CodecEntry] = &[
    CodecEntry { tags: &["A_VORBIS"], name: "Vorbis" },
    CodecEntry { tags: &["A_MPEG/L3"], name: "MP3" },
    CodecEntry { tags: &["A_MPEG/L2"], name: "MP2" },
    CodecEntry { tags: &["A_FLAC"], name: "FLAC" },
    CodecEntry { tags: &["alac", "A_ALAC"], name: "ALAC" },
    CodecEntry { tags: &["A_AC3", "ac-3"], name: "Dolby Digital" },
    CodecEntry { tags: &["mp4a"], name: "AAC" },
    CodecEntry { 
        tags: &[
            "A_PCM/INT/LIT", "A_PCM/INT/BIG", "A_PCM/FLOAT/IEEE",
            "sowt", "twos", "fl32", "fl64", "in24", "in32", "raw ",
        ], 
        name: "PCM", 
    },
    CodecEntry { tags: &["pcm_alaw"], name: "G.711 (a-law)" },
    CodecEntry { tags: &["pcm_mulaw"], name: "G.711 (mu-law)" },
    CodecEntry { tags: &["adpcm_ms"], name: "ADPCM (MS)" },
    CodecEntry { tags: &["adpcm_ima_wav", "adpcm_ima_qt"], name: "ADPCM (IMA)" },
    CodecEntry { tags: &["midi"], name: "MIDI" },
    CodecEntry { tags: &["xm"], name: "XM" },
];

/// Look up the display name for a codec tag from the registry.
pub fn codec_name(tag: &str) -> String {
    CODEC_ENTRIES.iter()
        .find(|entry| entry.tags.contains(&tag))
        .map_or_else(|| tag.to_owned(), |entry| entry.name.to_owned())
}


#[cfg(test)]
mod tests {
    use super::{
        STRETCH_SEGMENT, STRETCH_TOLERANCE, Stretch, apply_volume, apply_volume_frame,
        scaled_frames,
    };

    /// A tone of `period` frames to the cycle, `frames` long, the same on every
    /// channel so a stretch cannot hide a channel swap behind a matching pitch.
    fn tone(channels: usize, period: f32, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * channels];
        for frame in 0..frames {
            for channel in 0..channels {
                out[frame * channels + channel] =
                    (2.0 * std::f32::consts::PI * frame as f32 / period).sin();
            }
        }
        out
    }

    /// A tone whose two channels ask for different periods.
    fn two_tones(short: f32, long: f32, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames * 2];
        for frame in 0..frames {
            out[frame * 2] = (2.0 * std::f32::consts::PI * frame as f32 / short).sin();
            out[frame * 2 + 1] = (2.0 * std::f32::consts::PI * frame as f32 / long).sin();
        }
        out
    }

    /// Run a stretch over `input` the way a device pulls it: in blocks of
    /// `callback` frames, each handed a window from the frames not retired yet.
    fn stretch(channels: usize, milli: u32, input: &[f32], callback: usize) -> Vec<f32> {
        let mut stretch = Stretch::new(channels, milli);
        let mut out = Vec::new();
        loop {
            let base = stretch.retired();
            let window = &input[base * channels..];
            let mut piece = vec![0.0f32; callback * channels];
            let given = stretch.fill(base, window, &mut piece);
            if given == 0 {
                return out;
            }
            out.extend_from_slice(&piece[..given]);
        }
    }

    /// The delay at which a signal most resembles itself, searched over the
    /// periods a voice has: the pitch in frames, and the plainest way to see
    /// whether a stretch moved it.
    fn pitch(signal: &[f32], channels: usize) -> usize {
        let frames = signal.len() / channels;
        let window = (frames - 1).min(8_192);
        let mut best = (0.0f64, 0usize);
        for lag in 24..480 {
            let mut fit = 0.0f64;
            let mut left = 0.0f64;
            let mut right = 0.0f64;
            for frame in 0..window - lag {
                for channel in 0..channels {
                    let a = f64::from(signal[frame * channels + channel]);
                    let b = f64::from(signal[(frame + lag) * channels + channel]);
                    fit += a * b;
                    left += a * a;
                    right += b * b;
                }
            }
            let score = fit / (left * right).max(1e-9).sqrt();
            if score > best.0 {
                best = (score, lag);
            }
        }
        best.1
    }

    /// The whole point of the stretch: audio twice as short, still the same
    /// voice. The plain rate path fails the second half of this.
    #[test]
    fn a_stretched_stream_loses_length_and_keeps_its_pitch() {
        let input = tone(1, 100.0, 16_384);
        let fast = stretch(1, 2_000, &input, 441);
        assert!(
            fast.len() < input.len() * 3 / 5,
            "twice the rate gave {} frames",
            fast.len()
        );
        assert!(
            fast.len() > input.len() / 3,
            "twice the rate gave {} frames",
            fast.len()
        );
        assert!(pitch(&fast, 1).abs_diff(100) <= 4, "pitch moved");
        let slow = stretch(1, 500, &input, 441);
        assert!(slow.len() > input.len() * 3 / 2);
        assert!(slow.len() < input.len() * 5 / 2);
        assert!(pitch(&slow, 1).abs_diff(100) <= 4);
    }

    /// At the recorded rate the two halves of every fade are the same samples, so
    /// the stretch has nothing to do but hand the stream back.
    #[test]
    fn a_stretch_at_the_recorded_rate_hands_the_stream_back() {
        let input = tone(1, 97.0, 4_096);
        let same = stretch(1, 1_000, &input, 441);
        assert!(same.len() > input.len() - 2 * 441);
        // Only where the window runs out can a segment stop fitting at the very
        // place the rate asks for, which is the last couple of tolerances.
        let exact = same.len() - 2 * STRETCH_TOLERANCE - STRETCH_SEGMENT;
        for (frame, sample) in same[..exact].iter().enumerate() {
            assert!(
                (sample - input[frame]).abs() < 1e-4,
                "frame {frame} moved to {sample}"
            );
        }
    }

    /// A splice the ear could hear would show up as a step between two frames
    /// that no sine of that period would make on its own.
    #[test]
    fn a_stretched_stream_keeps_its_wave_continuous() {
        let period = 100.0f32;
        let input = tone(2, period, 16_384);
        // The steepest step a whole-amplitude tone of this period makes anyway.
        let natural = 2.0 * (std::f32::consts::PI / period).sin();
        for milli in [250, 500, 1_500, 2_000, 4_000] {
            let out = stretch(2, milli, &input, 256);
            let mut steps = 0.0f32;
            for frame in 1..out.len() / 2 {
                for channel in 0..2 {
                    let at = frame * 2 + channel;
                    steps = steps.max((out[at] - out[at - 2]).abs());
                }
            }
            assert!(steps < natural * 2.0, "{milli}/1000 stepped {steps}");
        }
    }

    /// The clock runs on what the stretch retires, so that count has to stay by
    /// the rate asked of it: the search may borrow, not spend.
    #[test]
    fn the_stretch_retires_what_the_rate_asked_for() {
        let input = tone(1, 100.0, 64 * STRETCH_SEGMENT);
        let mut stretch = Stretch::new(1, 1_750);
        let mut out = vec![0.0f32; 441];
        let mut given = 0usize;
        while given < 20_000 {
            let base = stretch.retired();
            let window = &input[base..];
            if window.len() < stretch.lookahead(441) {
                break;
            }
            let written = stretch.fill(base, window, &mut out);
            given += written;
        }
        let exact = scaled_frames(given, 1_750, 0).0;
        assert!(
            stretch.retired().abs_diff(exact) <= 2 * STRETCH_TOLERANCE,
            "retired {} for a nominal {exact}",
            stretch.retired()
        );
    }

    /// Each channel keeps its own voice: the fade borrows samples from the same
    /// channel only, so a stretch cannot mix the pair together. The two tones
    /// repeat on a common period, which is what the search of a stereo pair has
    /// to land on: it fits the frame as a whole, not each channel on its own.
    #[test]
    fn a_stretched_pair_keeps_each_channel_on_its_own() {
        let input = two_tones(80.0, 160.0, 16_384);
        let out = stretch(2, 2_000, &input, 441);
        let even: Vec<f32> = out.iter().copied().step_by(2).collect();
        let odd: Vec<f32> = out[1..].iter().copied().step_by(2).collect();
        assert!(pitch(&even, 1).abs_diff(80) <= 4);
        assert!(pitch(&odd, 1).abs_diff(160) <= 8);
    }

    /// A window that runs out costs the caller the segments it cannot cut, not
    /// the ones it can, and a rate asked for midway changes only the distance
    /// between the segments after it.
    #[test]
    fn a_short_window_and_a_late_rate_change_cost_nothing_extra() {
        let input = tone(1, 100.0, 4_096);
        let mut stretch = Stretch::new(1, 2_000);
        let mut out = vec![0.0f32; 4_096];
        let written = stretch.fill(0, &input[..64], &mut out);
        assert_eq!(written, 0, "a window of no segment gives no audio");
        let written = stretch.fill(0, &input[..1_024], &mut out);
        assert!(written > 0);
        assert!(written < out.len(), "a short window was served in full");
        stretch.set_rate(500);
        let base = stretch.retired();
        let mut filled = vec![0.0f32; 882];
        let again = stretch.fill(base, &input[base..], &mut filled);
        assert_eq!(again, filled.len(), "the new rate stalled the stretch");
        // A half rate spends about half of what it hands over, and the playhead
        // never runs back to the frames the faster rate already took.
        assert!(
            stretch.retired().abs_diff(base + 441) <= 2 * STRETCH_TOLERANCE,
            "a half rate retired {} for 882 frames",
            stretch.retired().abs_diff(base),
        );
    }

    #[test]
    fn level_scales_linearly_below_full_scale() {
        assert_eq!(apply_volume(0.5, 1_000), 0.5);
        assert_eq!(apply_volume(0.5, 500), 0.25);
        assert_eq!(apply_volume(0.5, 0), 0.0);
        assert_eq!(apply_volume(-0.25, 200), -0.05);
        assert_eq!(apply_volume(1.0, 1_000), 1.0);
    }

    #[test]
    fn non_finite_samples_stay_silent() {
        assert_eq!(apply_volume(f32::NAN, 1_000), 0.0);
        assert_eq!(apply_volume(f32::INFINITY, 4_000), 0.0);
    }

    #[test]
    fn a_full_scale_frame_is_left_alone() {
        let mut samples = [0.25, -0.5, 1.0];
        apply_volume_frame(&mut samples, 1_000);
        assert_eq!(samples, [0.25, -0.5, 1.0]);
    }

    #[test]
    fn cutting_never_clips() {
        let mut samples = [1.0, -1.0, 0.5];
        apply_volume_frame(&mut samples, 400);
        assert_eq!(samples, [0.4, -0.4, 0.2]);
    }

    /// The point of boosting past the recorded level is to hear quiet passages,
    /// not to square-wave the loud ones: the peak lands on full scale and the
    /// body rises underneath it.
    #[test]
    fn boost_lifts_the_body_and_holds_the_peak() {
        let mut samples = [0.45, -0.45, 0.9, 0.05];
        apply_volume_frame(&mut samples, 2_000);
        assert_eq!(samples[2], 1.0, "peak must sit at full scale");
        assert_eq!(samples[1], -samples[0]);
        assert!(samples[0] > 0.45, "quiet part not lifted");
        assert!(samples[3] > 0.05);

        // A block that still fits after the gain keeps the plain amplification.
        let mut quiet = [0.1, -0.2];
        apply_volume_frame(&mut quiet, 2_000);
        assert_eq!(quiet, [0.2, -0.4]);
    }

    #[test]
    fn muted_output_is_silent() {
        let mut samples = [0.75, -0.25];
        apply_volume_frame(&mut samples, 0);
        assert_eq!(samples, [0.0, 0.0]);
    }

    /// The carried fraction is what makes an awkward rate exact over time: a
    /// device pulling 441 frames per callback at 1.5x must consume 661.5 stream
    /// frames, and dropping the half frame every other callback would detune the
    /// clock by a whole frame.
    #[test]
    fn rate_consumes_the_exact_total_over_time() {
        let mut phase = 0;
        let mut consumed = 0;
        for _ in 0..1_000 {
            let (frames, carry) = scaled_frames(441, 1_500, phase);
            consumed += frames;
            phase = carry;
        }
        assert_eq!(consumed, 661_500);
        assert_eq!(phase, 0);
    }

    #[test]
    #[test]
    fn all_known_codecs_have_a_display_name() {
        let tags_and_names = [
            ("A_VORBIS", "Vorbis"),
            ("A_MPEG/L3", "MP3"),
            ("A_MPEG/L2", "MP2"),
            ("A_FLAC", "FLAC"),
            ("alac", "ALAC"),
            ("A_ALAC", "ALAC"),
            ("A_AC3", "Dolby Digital"),
            ("ac-3", "Dolby Digital"),
            ("mp4a", "AAC"),
            ("A_PCM/INT/LIT", "PCM"),
            ("A_PCM/INT/BIG", "PCM"),
            ("sowt", "PCM"),
            ("twos", "PCM"),
            ("pcm_alaw", "G.711 (a-law)"),
            ("pcm_mulaw", "G.711 (mu-law)"),
            ("adpcm_ms", "ADPCM (MS)"),
            ("adpcm_ima_wav", "ADPCM (IMA)"),
            ("midi", "MIDI"),
            ("xm", "XM"),
        ];
        
        for (tag, expected) in tags_and_names {
            assert_eq!(super::codec_name(tag), expected, "{tag}");
        }
        assert_eq!(super::codec_name("unknown"), "unknown");
    }


    fn default_rate_consumes_one_frame_per_frame() {
        for _ in 0..8 {
            let (frames, carry) = scaled_frames(128, 1_000, 0);
            assert_eq!(frames, 128);
            assert_eq!(carry, 0);
        }
    }
}
