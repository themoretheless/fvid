//! Rendering a MIDI performance as PCM.
//!
//! A Standard MIDI File states no sound at all, so this decoder is not a
//! reverse-engineering of a format but an instrument: it reads the packets the
//! SMF container hands out - events placed inside a slice of the timeline - and
//! plays them. What it has to get right is the same thing a player has to get
//! right: which keys are down when, how loudly, where in stereo, and for how long.
//!
//! The voice is a sine per key with a three-stage envelope, and every channel
//! sounds on it. The program number a file states is kept and reported, because
//! a listener switching tracks wants to see the file's own names, but no file
//! owes anything beyond the number and no instrument table this player carries
//! answers to it either. CC10 uses a centre-preserving linear stereo balance:
//! 0 is left, 64 keeps the original dual-mono level, and 127 is right. Mono
//! and legacy wider outputs keep the dry mix; pan LSB is not applied. RPN0
//! sets channel bend sensitivity in semitones/cents for sounding and future
//! notes. NRPN and unknown RPN data are ignored; CC121 nulls selection while
//! preserving sensitivity and channel tuning. RPN1 adds14-bit fractional tuning
//! over one semitone either side of center8192. RPN2 retunes active and future
//! notes in whole semitones; data LSB is ignored. Decoder reset restores
//! two-semitone bend sensitivity and centered fine/coarse tuning.

use crate::audio::{AudioDecode, AudioPacket};
use crate::container::smf::{self, Event};
use crate::{Result, invalid};

/// Seconds to reach full level from nothing.
const ATTACK_SECONDS: f32 = 0.005;
/// Seconds to fall from full level to the level a held key keeps.
const DECAY_SECONDS: f32 = 0.06;
/// The level a key held past its decay keeps.
const SUSTAIN_LEVEL: f32 = 0.7;
/// Seconds a note takes to die after its key is lifted.
const RELEASE_SECONDS: f32 = 0.04;
/// Keys a performance can sound at once. Beyond this the longest-held is let go,
/// which is what an instrument with too many fingers down does too.
const MAX_VOICES: usize = 32;
/// Default bend sensitivity until the channel selects RPN0 and supplies data.
const DEFAULT_BEND_SEMITONES: u8 = 2;
/// The level of one key struck at full force, so that a chord of a dozen still
/// has room before the scale is full.
const VOICE_LEVEL: f32 = 0.18;
/// Middle A, the key every tuning starts from.
const CONCERT_PITCH_KEY: f32 = 69.0;
const CONCERT_PITCH_HERTZ: f32 = 440.0;
/// The untuned centre of a bend.
const BEND_CENTRE: u16 = 8192;
/// A bend word's own span above and below the centre.
const BEND_SPAN: f32 = 8192.0;

/// What a key is doing: down and coming up to level, down and settling, held, or
/// let go.
#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
}

/// One sounding key.
struct Voice {
    channel: u8,
    key: u8,
    /// The key itself can be up while the sustain pedal keeps its envelope held.
    key_down: bool,
    /// Cycles advanced per frame, so a bend can retune a note already sounding.
    step: f32,
    phase: f32,
    /// Where the envelope is.
    level: f32,
    /// How hard the key was struck, which scales the envelope.
    force: f32,
    stage: Stage,
    /// What one frame adds to, or takes from, the level while a stage runs.
    slope: f32,
    /// The frame the key went down at, which is who gets let go first.
    started: u64,
}

/// What one channel is set to, held between packets because a file states a
/// setting once and means it until it says otherwise.
#[derive(Clone, Copy, Default)]
struct Channel {
    program: u8,
    /// MIDI's own defaults: the channel at full level and unshifted.
    volume: u8,
    expression: u8,
    pan: u8,
    gain: f32,
    left_balance: f32,
    right_balance: f32,
    bend_semitones: u8,
    bend_cents: u8,
    coarse_semitones: i8,
    fine_tuning: u16,
    rpn_msb: u8,
    rpn_lsb: u8,
    rpn_active: bool,
    bend: u16,
    sustain: bool,
}

impl Channel {
    fn update_mix(&mut self) {
        self.gain = f32::from(self.volume) / 127.0 * (f32::from(self.expression) / 127.0);
        (self.left_balance, self.right_balance) = if self.pan <= 64 {
            (1.0, f32::from(self.pan) / 64.0)
        } else { (f32::from(127 - self.pan) / 63.0, 1.0) };
    }

    fn bend_range(self) -> f32 {
        f32::from(self.bend_semitones) + f32::from(self.bend_cents) / 100.0
    }

    fn unset() -> Self {
        Self {
            program: 0,
            volume: 127,
            expression: 127,
            pan: 64,
            gain: 1.0,
            left_balance: 1.0,
            right_balance: 1.0,
            bend_semitones: DEFAULT_BEND_SEMITONES,
            bend_cents: 0,
            coarse_semitones: 0,
            fine_tuning: BEND_CENTRE,
            rpn_msb: 127,
            rpn_lsb: 127,
            rpn_active: false,
            bend: BEND_CENTRE,
            sustain: false,
        }
    }
}

/// Supported continuous channel controls.
const CONTROLLER_VOLUME: u8 = 7;
const CONTROLLER_EXPRESSION: u8 = 11;
const CONTROLLER_PAN: u8 = 10;
const CONTROLLER_SUSTAIN: u8 = 64;
/// The two ways a file says "stop": let the notes go, and cut the sound.
const CONTROLLER_ALL_NOTES_OFF: u8 = 123;
const CONTROLLER_ALL_SOUND_OFF: u8 = 120;
const CONTROLLER_RESET: u8 = 121;

/// How far one frame moves a level that has `seconds` of travel left.
fn slope_from(level: f32, seconds: f32, sample_rate: u32) -> f32 {
    let frames = (seconds * (sample_rate as f32)).max(1.0);
    level / frames
}

/// The cycle a key sounds at, bent the way its channel is bent.
fn step_of(key: u8, state: Channel, sample_rate: u32) -> f32 {
    let bend = (i32::from(state.bend) - i32::from(BEND_CENTRE)) as f32 / BEND_SPAN * state.bend_range();
    let tuning = f32::from(state.coarse_semitones)
        + (i32::from(state.fine_tuning) - i32::from(BEND_CENTRE)) as f32 / BEND_SPAN;
    let hertz = CONCERT_PITCH_HERTZ
        * 2.0f32.powf((f32::from(key) - CONCERT_PITCH_KEY + bend + tuning) / 12.0);
    // Fold whole cycles at retune, keeping every supported tuning bounded.
    (hertz / (sample_rate as f32)).fract()
}

/// A synthesiser for the SMF packets `crate::container::smf` writes.
pub struct MidiDecoder {
    sample_rate: u32,
    channels: u16,
    voices: Vec<Voice>,
    state: [Channel; 16],
    /// Frames rendered so far across packets, so a key held over one keeps its
    /// place in the cycle and the eviction order stays the playing order.
    rendered: u64,
}

impl MidiDecoder {
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self> {
        if sample_rate == 0 {
            return Err(invalid("MIDI rendering needs a sample rate to render at"));
        }
        Ok(Self {
            sample_rate,
            channels: channels.clamp(1, 8),
            voices: Vec::new(),
            state: [Channel::unset(); 16],
            rendered: 0,
        })
    }

    /// The program number a channel was last told, for a listing of what the
    /// file asked for.
    pub fn program(&self, channel: u8) -> Option<u8> {
        Some(self.state.get(usize::from(channel))?.program)
    }

    /// One event from a packet, applied to what is held.
    fn apply(&mut self, event: Event) {
        match event {
            Event::NoteOn {
                channel,
                key,
                velocity,
            } => self.press(channel, key, velocity),
            Event::NoteOff { channel, key } => self.lift(channel, key),
            Event::Program { channel, number } => {
                if let Some(state) = self.state.get_mut(usize::from(channel)) {
                    state.program = number;
                }
            }
            Event::Control {
                channel,
                number,
                value,
            } => self.control(channel, number, value),
            Event::PitchBend { channel, value } => {
                if let Some(state) = self.state.get_mut(usize::from(channel)) {
                    state.bend = value;
                }
                self.retune(channel);
            }
        }
    }

    fn retune(&mut self, channel: u8) {
        let Some(state) = self.state.get(usize::from(channel)).copied() else { return; };
        for voice in self.voices.iter_mut().filter(|voice| voice.channel == channel) {
            voice.step = step_of(voice.key, state, self.sample_rate);
        }
    }

    fn press(&mut self, channel: u8, key: u8, velocity: u8) {
        // A key struck twice is the same key struck again: the note already
        // sounding is let go so the new one starts its envelope from nothing.
        self.voices
            .retain(|voice| !(voice.channel == channel && voice.key == key));
        if self.voices.len() >= MAX_VOICES {
            let oldest = self
                .voices
                .iter()
                .enumerate()
                .min_by_key(|(_, voice)| voice.started)
                .map_or(0, |(index, _)| index);
            self.voices.remove(oldest);
        }
        let state = self.state[usize::from(channel & 15)];
        let step = step_of(key, state, self.sample_rate);
        let started = self.rendered;
        let force = f32::from(velocity) / 127.0;
        self.voices.push(Voice {
            channel,
            key,
            key_down: true,
            step,
            phase: 0.0,
            level: 0.0,
            // A key tapped lightly is quieter than the same tap struck hard, and
            // the ear reads that difference as steeper than a straight ratio.
            force: force * force,
            stage: Stage::Attack,
            slope: 1.0 / (ATTACK_SECONDS * (self.sample_rate as f32)),
            started,
        });
    }

    fn lift(&mut self, channel: u8, key: u8) {
        let sample_rate = self.sample_rate;
        let sustain = self.state[usize::from(channel)].sustain;
        for voice in self
            .voices
            .iter_mut()
            .filter(|voice| voice.channel == channel && voice.key == key)
        {
            voice.key_down = false;
            if !sustain && voice.stage != Stage::Release {
                voice.stage = Stage::Release;
                voice.slope = slope_from(voice.level, RELEASE_SECONDS, sample_rate);
            }
        }
        self.drop_finished();
    }

    /// Every key on one channel, or on all of them with `channel` past the last.
    fn let_go(&mut self, channel: u8, cut: bool) {
        let sample_rate = self.sample_rate;
        let state = self.state;
        for voice in self
            .voices
            .iter_mut()
            .filter(|voice| channel > 15 || voice.channel == channel)
        {
            voice.key_down = false;
            if cut {
                voice.level = 0.0;
                voice.stage = Stage::Release;
            } else if !state[usize::from(voice.channel)].sustain && voice.stage != Stage::Release {
                voice.stage = Stage::Release;
                voice.slope = slope_from(voice.level, RELEASE_SECONDS, sample_rate);
            }
        }
        self.drop_finished();
    }

    fn set_sustain(&mut self, channel: u8, enabled: bool) {
        self.state[usize::from(channel)].sustain = enabled;
        if enabled { return; }
        let sample_rate = self.sample_rate;
        for voice in self.voices.iter_mut().filter(|voice| voice.channel == channel && !voice.key_down) {
            if voice.stage != Stage::Release {
                voice.stage = Stage::Release;
                voice.slope = slope_from(voice.level, RELEASE_SECONDS, sample_rate);
            }
        }
        self.drop_finished();
    }

    fn control(&mut self, channel: u8, number: u8, value: u8) {
        match number {
            100 | 101 => {
                let state = &mut self.state[usize::from(channel)];
                if number == 100 { state.rpn_lsb = value.min(127); }
                else { state.rpn_msb = value.min(127); }
                state.rpn_active = true;
            }
            98 | 99 => self.state[usize::from(channel)].rpn_active = false,
            6 | 38 => {
                let state = &mut self.state[usize::from(channel)];
                if state.rpn_active && state.rpn_msb == 0 {
                    let changed = match state.rpn_lsb {
                        0 => {
                            if number == 6 { state.bend_semitones = value.min(127); }
                            else { state.bend_cents = value.min(127); }
                            true
                        }
                        1 => {
                            let value = u16::from(value.min(127));
                            state.fine_tuning = if number == 6 {
                                (state.fine_tuning & 127) | (value << 7)
                            } else { (state.fine_tuning & !127) | value };
                            true
                        }
                        // RPN2 is a signed semitone offset from MSB64; LSB is unused.
                        2 if number == 6 => {
                            state.coarse_semitones = (i16::from(value.min(127)) - 64) as i8;
                            true
                        }
                        _ => false,
                    };
                    if changed { self.retune(channel); }
                }
            }
            CONTROLLER_VOLUME | CONTROLLER_EXPRESSION | CONTROLLER_PAN => {
                let state = &mut self.state[usize::from(channel)];
                match number {
                    CONTROLLER_VOLUME => state.volume = value,
                    CONTROLLER_EXPRESSION => state.expression = value,
                    _ => state.pan = value.min(127),
                }
                state.update_mix();
            }
            CONTROLLER_SUSTAIN => self.set_sustain(channel, value >= 64),
            CONTROLLER_ALL_NOTES_OFF => self.let_go(channel, false),
            // All sound off is a panic: what was sounding stops in the frame it
            // is told to, not over the release time it would have taken.
            CONTROLLER_ALL_SOUND_OFF => self.let_go(channel, true),
            CONTROLLER_RESET => {
                // Reset expression and bend, preserving the selected program
                // and channel volume. Center the sounding voices as well as
                // the register used by notes triggered after this message.
                self.set_sustain(channel, false);
                self.state[channel as usize].expression = 127;
                self.state[channel as usize].update_mix();
                self.state[channel as usize].rpn_msb = 127;
                self.state[channel as usize].rpn_lsb = 127;
                self.state[channel as usize].rpn_active = false;
                self.apply(Event::PitchBend { channel, value: BEND_CENTRE });
            }
            // Other controllers, including modulation and pan LSB, are outside
            // this instrument's supported profile.
            _ => {}
        }
    }

    fn render_frames<const CHECK_GAIN: bool>(
        &mut self,
        events: &[smf::WindowEvent],
        output: &mut [f32],
    ) -> usize {
        let sample_rate = self.sample_rate;
        let channels = usize::from(self.channels);
        let frames = output.len() / channels;
        // The container's records are already in timeline order, so one cursor
        // walks them as the frames go by.
        let mut next = 0usize;
        for frame in 0..frames {
            while next < events.len()
                && u128::from(events[next].offset_microseconds) * u128::from(sample_rate)
                    <= frame as u128 * 1_000_000
            {
                self.apply(events[next].event);
                next += 1;
            }
            if self.voices.is_empty() {
                continue;
            }
            // Mixing coefficients change only with channel controls.
            let state = &self.state;
            let mut mix = 0.0f32;
            let mut left = 0.0f32;
            let mut right = 0.0f32;
            self.voices.retain_mut(|voice| {
                voice.level = match voice.stage {
                    Stage::Attack => {
                        let level = voice.level + voice.slope;
                        if level >= 1.0 {
                            voice.stage = Stage::Decay;
                            voice.slope =
                                slope_from(1.0 - SUSTAIN_LEVEL, DECAY_SECONDS, sample_rate);
                            1.0
                        } else {
                            level
                        }
                    }
                    Stage::Decay => {
                        let level = voice.level - voice.slope;
                        if level <= SUSTAIN_LEVEL {
                            voice.stage = Stage::Sustain;
                            voice.slope = 0.0;
                            SUSTAIN_LEVEL
                        } else {
                            level
                        }
                    }
                    Stage::Sustain => SUSTAIN_LEVEL,
                    Stage::Release => (voice.level - voice.slope).max(0.0),
                };
                if voice.level == 0.0 {
                    return false;
                }
                let channel = &state[usize::from(voice.channel & 15)];
                if !CHECK_GAIN || channel.gain != 0.0 {
                    let value = voice.level
                        * voice.force
                        * channel.gain
                        * VOICE_LEVEL
                        * (std::f32::consts::TAU * voice.phase).sin();
                    mix += value;
                    if channels == 2 {
                        left += value * channel.left_balance;
                        right += value * channel.right_balance;
                    }
                }
                // Even muted voices retain exact per-sample phase/envelope history.
                voice.phase += voice.step;
                if voice.phase >= 1.0 {
                    voice.phase -= 1.0;
                }
                true
            });
            if channels == 2 {
                output[frame * 2] = left.clamp(-1.0, 1.0);
                output[frame * 2 + 1] = right.clamp(-1.0, 1.0);
            } else {
                // Mono and legacy wider outputs keep the placement-free mix.
                let sample = mix.clamp(-1.0, 1.0);
                for channel in 0..channels {
                    output[frame * channels + channel] = sample;
                }
            }
        }
        next
    }

    fn drop_finished(&mut self) {
        self.voices.retain(|voice| voice.stage != Stage::Release || voice.level > 0.0);
    }
}

impl AudioDecode for MidiDecoder {
    /// Render one window of the timeline. The window is the packet's own length
    /// in milliseconds, which the container fixes at 100.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        duration: u64,
    ) -> Result<Option<AudioPacket>> {
        let events = smf::read_window(data)?;
        if events.windows(2).any(|pair| pair[0].offset_microseconds > pair[1].offset_microseconds) {
            return Err(invalid("MIDI event offsets are not ordered"));
        }
        if events.iter().any(|event| u128::from(event.offset_microseconds) >= u128::from(duration) * 1_000) {
            return Err(invalid("MIDI event offset exceeds its window"));
        }
        let sample_rate = self.sample_rate;
        let channels = usize::from(self.channels);
        let geometry_error = || invalid("MIDI PCM window geometry exceeds addressable storage");
        let frames = usize::try_from(u128::from(duration) * u128::from(sample_rate) / 1_000)
            .map_err(|_| geometry_error())?;
        let samples = frames.checked_mul(channels)
            .filter(|&samples| samples <= isize::MAX as usize / std::mem::size_of::<f32>())
            .ok_or_else(geometry_error)?;
        let byte_count = samples * std::mem::size_of::<f32>();
        let mut output = Vec::new();
        output.try_reserve_exact(samples)
            .map_err(|_| invalid("MIDI PCM window sample allocation failed"))?;
        // Reserve both buffers before any event or oscillator state is changed.
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(byte_count)
            .map_err(|_| invalid("MIDI PCM window byte allocation failed"))?;
        output.resize(samples, 0.0f32);
        // Choose once per packet. A zero-gain control later in the window
        // requires the checked path even when all channels currently have gain.
        let check_gain = self.state.iter().any(|channel| channel.gain == 0.0)
            || events.iter().any(|event| matches!(event.event,
                Event::Control { number: CONTROLLER_VOLUME | CONTROLLER_EXPRESSION, value: 0, .. }));
        let next = if check_gain {
            self.render_frames::<true>(&events, &mut output)
        } else { self.render_frames::<false>(&events, &mut output) };
        // Events after the final sample still precede the next window. Apply their
        // state changes without advancing an oscillator or envelope a sample early.
        for event in &events[next..] {
            self.apply(event.event);
        }
        self.rendered += frames as u64;
        for sample in output {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(Some(AudioPacket {
            data: bytes,
            pts,
            timebase_num: 1,
            // Source PTS and durations are milliseconds, independent of PCM rate.
            timebase_den: 1_000,
        }))
    }

    fn reset(&mut self) {
        self.voices.clear();
        self.state = [Channel::unset(); 16];
        self.rendered = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{BEND_CENTRE, MAX_VOICES, MidiDecoder, RELEASE_SECONDS, SUSTAIN_LEVEL, VOICE_LEVEL};
    use crate::audio::AudioDecode;
    use crate::container::smf::{self, Event, WINDOW_MILLISECONDS};

    /// A packet holding `events`, each at its own offset into the window.
    fn packet(events: &[(u32, Event)]) -> Vec<u8> {
        let mut data = Vec::new();
        for (offset, event) in events {
            smf::write_record(*event, *offset, &mut data);
        }
        data
    }

    fn note(channel: u8, key: u8, velocity: u8) -> (u32, Event) {
        (
            0,
            Event::NoteOn {
                channel,
                key,
                velocity,
            },
        )
    }

    fn render(decoder: &mut MidiDecoder, data: &[u8], window: u64) -> Vec<f32> {
        let samples = decoder
            .decode_encoded(data, window, WINDOW_MILLISECONDS)
            .expect("renders")
            .expect("MIDI always yields a packet");
        // A packet is 100 ms of the timeline, timed against the sample rate the
        // track was opened at.
        assert_eq!((samples.timebase_num, samples.timebase_den), (1, 1_000));
        assert_eq!(samples.pts, window);
        samples
            .data
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
            .collect()
    }

    fn synth() -> MidiDecoder {
        MidiDecoder::new(44_100, 2).expect("decoder")
    }

    /// Frames of one window at this rate.
    const WINDOW_FRAMES: usize = 4410;

    fn peak(samples: &[f32]) -> f32 {
        samples
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
    }

    /// How many times the wave crossed zero upwards, which counts its periods.
    fn rising_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
            .count()
    }

    /// One channel's samples out of an interleaved block.
    fn left(samples: &[f32]) -> Vec<f32> {
        samples.chunks_exact(2).map(|pair| pair[0]).collect()
    }

    #[test]
    fn a_packet_becomes_the_frames_its_window_spans() {
        let mut decoder = synth();
        let samples = render(&mut decoder, &packet(&[]), 0);
        // 100 ms at 44.1 kHz, two channels, four bytes a sample.
        assert_eq!(samples.len(), WINDOW_FRAMES * 2);
    }

    #[test]
    fn maximum_bend_sensitivity_keeps_discrete_phase_bounded_and_resettable() {
        let bytes = include_bytes!("../../tests/fixtures/playback-errors/midi-bend-range/range-resource.mid");
        let file = smf::SmfFile::parse(bytes, &Default::default()).unwrap();
        for rate in [8_000, 44_100] {
            let mut decoder = MidiDecoder::new(rate, 2).unwrap();
            let mut observed = 0;
            for index in 0..file.windows() {
                let output = decoder.decode_packet(&file.window_packet(index), index as i64 * 100, 100).unwrap().unwrap();
                assert!(output.packet.data.chunks_exact(4).all(|sample| {
                    let value = f32::from_le_bytes(sample.try_into().unwrap());
                    value.is_finite() && value.abs() <= 1.0
                }));
                for voice in &decoder.voices {
                    assert!((0.0..1.0).contains(&voice.phase));
                    assert!((0.0..1.0).contains(&voice.step));
                    observed += 1;
                }
                assert!(decoder.voices.len() <= MAX_VOICES);
            }
            assert!(observed >= 3);
            decoder.reset();
            assert_eq!(decoder.state[0].bend_range(), 2.0);
            assert!(!decoder.state[0].rpn_active);
            assert_eq!((decoder.state[0].rpn_msb, decoder.state[0].rpn_lsb), (127, 127));
        }
    }

    #[test]
    fn channel_tuning_extremes_preserve_note_identity_and_bounded_phase() {
        for bytes in [
            include_bytes!("../../tests/fixtures/playback-errors/midi-coarse-tuning/value-0.mid").as_slice(),
            include_bytes!("../../tests/fixtures/playback-errors/midi-coarse-tuning/value-127.mid").as_slice(),
            include_bytes!("../../tests/fixtures/playback-errors/midi-fine-tuning/value-0.mid").as_slice(),
            include_bytes!("../../tests/fixtures/playback-errors/midi-fine-tuning/value-16383.mid").as_slice(),
        ] {
            let file = smf::SmfFile::parse(bytes, &Default::default()).unwrap();
            for rate in [8_000, 44_100] {
                let mut decoder = MidiDecoder::new(rate, 2).unwrap();
                let mut observed = 0;
                for index in 0..file.windows() {
                    let output = decoder.decode_packet(&file.window_packet(index), index as i64 * 100, 100).unwrap().unwrap();
                    assert!(output.packet.data.chunks_exact(4).all(|s| {
                        let value = f32::from_le_bytes(s.try_into().unwrap());
                        value.is_finite() && value.abs() <= 1.0
                    }));
                    for voice in &decoder.voices {
                        assert_eq!(voice.key, 64, "NoteOff must retain the original key");
                        assert!((0.0..1.0).contains(&voice.phase));
                        assert!((0.0..1.0).contains(&voice.step));
                        observed += 1;
                    }
                    assert!(decoder.voices.len() <= MAX_VOICES);
                }
                assert!(observed > 0);
                decoder.reset();
                assert_eq!(decoder.state[0].coarse_semitones, 0);
                assert_eq!(decoder.state[0].fine_tuning, BEND_CENTRE);
                assert_eq!(decoder.state[0].bend_range(), 2.0);
            }
        }
    }

    #[test]
    fn both_channels_carry_the_same_performance() {
        let mut decoder = synth();
        let samples = render(&mut decoder, &packet(&[note(0, 60, 127)]), 0);
        let left = left(&samples);
        let right: Vec<f32> = samples.chunks_exact(2).map(|pair| pair[1]).collect();
        assert!(peak(&left) > 0.0);
        assert_eq!(left, right);
    }

    #[test]
    fn a_held_key_sounds_at_its_own_pitch() {
        let mut decoder = synth();
        let samples = left(&render(&mut decoder, &packet(&[note(0, 69, 127)]), 0));
        // 440 Hz across a tenth of a second is 44 periods.
        let measured = rising_crossings(&samples) as f32 * 44_100.0 / WINDOW_FRAMES as f32;
        assert!((measured - 440.0).abs() < 20.0, "A4 back as {measured} Hz");
        // An octave up is double the periods in the same window.
        let upper = left(&render(&mut synth(), &packet(&[note(0, 81, 127)]), 0));
        assert!(rising_crossings(&upper) > rising_crossings(&samples) * 2 - 2);
    }

    #[test]
    fn the_envelope_rises_holds_and_a_released_note_falls_away() {
        let mut decoder = synth();
        let attack = left(&render(&mut decoder, &packet(&[note(0, 60, 127)]), 0));
        // The 5 ms of attack is 220 frames, so the window's own loudest moment is
        // the top of the envelope.
        assert!(peak(&attack[200..400]) > VOICE_LEVEL * 0.97);
        // Two thirds of the window on, the 60 ms of decay has run and what is left
        // is the level a held key keeps.
        let plateau = peak(&attack[3200..3400]);
        assert!(
            (plateau - VOICE_LEVEL * SUSTAIN_LEVEL).abs() < VOICE_LEVEL * 0.05,
            "held at {plateau}"
        );
        // The key lifted at the start of the next packet: 40 ms of release, which
        // is over well before the window's last tenth.
        let lifted = left(&render(
            &mut decoder,
            &packet(&[(
                0,
                Event::NoteOff {
                    channel: 0,
                    key: 60,
                },
            )]),
            100,
        ));
        let released_for = (RELEASE_SECONDS * 44_100.0) as usize;
        assert!(
            peak(&lifted[..released_for / 4]) > 0.0,
            "release never sounded"
        );
        assert!(
            peak(&lifted[released_for + 1000..]) == 0.0,
            "still at {:?}",
            peak(&lifted[released_for + 1000..])
        );
    }

    #[test]
    fn a_note_started_in_one_packet_is_held_across_the_next() {
        let mut decoder = synth();
        render(&mut decoder, &packet(&[note(0, 60, 127)]), 0);
        let held = left(&render(&mut decoder, &packet(&[]), 100));
        // Nothing new happens in the second packet and still it sounds, at the
        // level a held key keeps.
        assert!(peak(&held) > VOICE_LEVEL * SUSTAIN_LEVEL * 0.99);
        // And it is still the same note: middle C is 26 periods in a tenth of a
        // second, and the cycle did not restart at the packet boundary.
        assert!((rising_crossings(&held) as f32 - 26.2).abs() < 1.5);
    }

    #[test]
    fn events_further_into_a_packet_sound_later_in_its_samples() {
        let mut decoder = synth();
        // Struck halfway through the window, so half of it is silence before the
        // key goes down.
        let samples = left(&render(
            &mut decoder,
            &packet(&[(
                50_000,
                Event::NoteOn {
                    channel: 0,
                    key: 60,
                    velocity: 127,
                },
            )]),
            0,
        ));
        assert!(
            samples[..WINDOW_FRAMES / 2]
                .iter()
                .all(|sample| *sample == 0.0)
        );
        assert!(peak(&samples[WINDOW_FRAMES / 2..]) > 0.0);
        // A window nothing happens in is a window of silence.
        let silence = render(&mut synth(), &packet(&[]), 0);
        assert!(silence.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn how_hard_a_key_is_struck_changes_how_loud_it_is() {
        let mut decoder = synth();
        let soft = left(&render(&mut decoder, &packet(&[note(0, 60, 40)]), 0));
        let mut decoder = synth();
        let hard = left(&render(&mut decoder, &packet(&[note(0, 60, 127)]), 0));
        assert!(peak(&soft) < peak(&hard) / 4.0);
        assert!(peak(&soft) > 0.0);
    }

    #[test]
    fn a_channel_set_to_nothing_is_silent_and_a_full_one_is_not() {
        let mut decoder = synth();
        let quiet = left(&render(
            &mut decoder,
            &packet(&[
                (
                    0,
                    Event::Control {
                        channel: 3,
                        number: 7,
                        value: 0,
                    },
                ),
                note(3, 64, 127),
            ]),
            0,
        ));
        assert_eq!(peak(&quiet), 0.0);
        let mut decoder = synth();
        let loud = left(&render(
            &mut decoder,
            &packet(&[
                (
                    0,
                    Event::Control {
                        channel: 3,
                        number: 7,
                        value: 127,
                    },
                ),
                note(3, 64, 127),
            ]),
            0,
        ));
        assert!(peak(&loud) > VOICE_LEVEL * 0.99);
        // Expression is a second, independent handle on the same channel.
        let mut decoder = synth();
        render(
            &mut decoder,
            &packet(&[(
                0,
                Event::Control {
                    channel: 3,
                    number: 11,
                    value: 64,
                },
            )]),
            0,
        );
        let half = left(&render(&mut decoder, &packet(&[note(3, 64, 127)]), 100));
        assert!(peak(&half) < peak(&loud) * 0.6);
    }

    #[test]
    fn one_channels_notes_off_leaves_the_other_sounding_and_a_panic_stops_all() {
        let mut decoder = synth();
        render(
            &mut decoder,
            &packet(&[note(0, 60, 127), note(1, 64, 127)]),
            0,
        );
        let after = left(&render(
            &mut decoder,
            &packet(&[(
                0,
                Event::Control {
                    channel: 0,
                    number: 123,
                    value: 0,
                },
            )]),
            100,
        ));
        // Two keys down is twice the level of one, so the released channel shows
        // up as the window's own peak falling to the other key's alone.
        let mut both = synth();
        let alone = left(&render(&mut both, &packet(&[note(1, 64, 127)]), 0));
        let tail_start = WINDOW_FRAMES * 2 / 3;
        assert!(peak(&after[tail_start..]) <= peak(&alone[tail_start..]) * 1.05);
        let panicked = left(&render(
            &mut decoder,
            &packet(&[(
                0,
                Event::Control {
                    channel: 1,
                    number: 120,
                    value: 0,
                },
            )]),
            200,
        ));
        assert!(peak(&panicked[panicked.len() / 2..]) == 0.0);
    }

    #[test]
    fn a_bend_moves_the_keys_already_down() {
        let mut decoder = synth();
        render(&mut decoder, &packet(&[note(0, 69, 127)]), 0);
        let bent = left(&render(
            &mut decoder,
            &packet(&[(
                0,
                Event::PitchBend {
                    channel: 0,
                    value: 16_383,
                },
            )]),
            100,
        ));
        let mut straight = synth();
        render(&mut straight, &packet(&[note(0, 69, 127)]), 0);
        let level = left(&render(&mut straight, &packet(&[]), 100));
        // A full bend is the two semitones the format defaults to, so a twelfth
        // more periods in the same window.
        assert!(rising_crossings(&bent) > rising_crossings(&level) * 11 / 10);
    }

    #[test]
    fn a_program_number_is_kept_for_a_listing() {
        let mut decoder = synth();
        render(
            &mut decoder,
            &packet(&[(
                0,
                Event::Program {
                    channel: 5,
                    number: 42,
                },
            )]),
            0,
        );
        assert_eq!(decoder.program(5), Some(42));
        assert_eq!(decoder.program(6), Some(0));
        assert_eq!(decoder.program(16), None);
    }

    #[test]
    fn too_many_keys_at_once_let_the_oldest_go() {
        let mut decoder = synth();
        let events = (0..40u8)
            .map(|index| {
                (
                    u32::from(index) * 2_000,
                    Event::NoteOn {
                        channel: 0,
                        key: 40 + index,
                        velocity: 100,
                    },
                )
            })
            .collect::<Vec<_>>();
        let samples = left(&render(&mut decoder, &packet(&events), 0));
        // Still sounding, and never past the scale.
        assert!(peak(&samples) > 0.0);
        assert!(peak(&samples) <= 1.0);
        assert_eq!(decoder.voices.len(), MAX_VOICES);
        // A key struck twice is the same key: it restarts rather than doubling up.
        let mut decoder = synth();
        render(&mut decoder, &packet(&[note(2, 60, 127)]), 0);
        render(
            &mut decoder,
            &packet(&[(
                10_000,
                Event::NoteOn {
                    channel: 2,
                    key: 60,
                    velocity: 127,
                },
            )]),
            100,
        );
        assert_eq!(decoder.voices.len(), 1);
    }

    #[test]
    fn a_released_note_is_silent_and_stays_silent() {
        // With every voice dropped the instrument stops generating, rather than
        // leaving a tail of ever-smaller numbers to hum.
        let mut decoder = synth();
        render(
            &mut decoder,
            &packet(&[(
                90_000,
                Event::NoteOn {
                    channel: 0,
                    key: 60,
                    velocity: 127,
                },
            )]),
            0,
        );
        render(
            &mut decoder,
            &packet(&[(
                0,
                Event::NoteOff {
                    channel: 0,
                    key: 60,
                },
            )]),
            100,
        );
        for window in 2..5 {
            let samples = render(&mut decoder, &packet(&[]), window * 100);
            assert!(
                samples.iter().all(|sample| *sample == 0.0),
                "window {window} still sounding"
            );
        }
        assert!(decoder.voices.is_empty());
    }

    #[test]
    fn sustain_keeps_the_existing_voice_budget_and_releases_unrendered_keys() {
        let mut decoder = synth();
        decoder.apply(Event::Control { channel: 0, number: 64, value: 127 });
        for key in 0..=127 {
            decoder.apply(Event::NoteOn { channel: 0, key, velocity: 127 });
            decoder.apply(Event::NoteOff { channel: 0, key });
            assert!(decoder.voices.len() <= MAX_VOICES);
        }
        assert_eq!(decoder.voices.len(), MAX_VOICES);
        assert!(decoder.voices.iter().all(|voice| !voice.key_down));
        decoder.apply(Event::Control { channel: 0, number: 64, value: 0 });
        assert!(decoder.voices.is_empty());
    }

    #[test]
    fn a_reset_forgets_what_was_held() {
        let mut decoder = synth();
        render(&mut decoder, &packet(&[note(0, 60, 127)]), 0);
        render(
            &mut decoder,
            &packet(&[(
                0,
                Event::Control {
                    channel: 0,
                    number: 7,
                    value: 30,
                },
            )]),
            100,
        );
        decoder.apply(Event::Control { channel: 0, number: 64, value: 127 });
        assert!(decoder.rendered > 0);
        decoder.reset();
        assert_eq!(decoder.rendered, 0);
        assert!(decoder.voices.is_empty());
        assert!(!decoder.state[0].sustain);
        assert_eq!(decoder.state[0].volume, 127);
        let after = left(&render(&mut decoder, &packet(&[]), 200));
        assert!(after.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn refuses_a_packet_it_cannot_read() {
        let mut decoder = synth();
        assert!(decoder.decode_encoded(&[1, 2, 3], 0, 100).is_err());
        assert!(MidiDecoder::new(0, 2).is_err());
    }

    /// The other side of the packet format, from the container's own writer.
    #[test]
    fn records_written_by_the_container_are_read_back() {
        let mut data = Vec::new();
        smf::write_record(
            Event::NoteOn {
                channel: 2,
                key: 61,
                velocity: 90,
            },
            12_345,
            &mut data,
        );
        assert_eq!(
            smf::read_window(&data).expect("reads"),
            vec![smf::WindowEvent {
                offset_microseconds: 12_345,
                event: Event::NoteOn {
                    channel: 2,
                    key: 61,
                    velocity: 90
                },
            }]
        );
    }
}
