//! Rendering a FastTracker II module as PCM.
//!
//! A module states no encoded audio: it holds samples and a grid of rows saying
//! which of them to play when, so this decoder is an instrument rather than a
//! reverse-engineering. What it reads is what
//! [`crate::container::xm`] hands out - one row of the grid per packet, with the
//! tempo that row is played under beside its cells - and what it has to get right
//! is the same thing a player has to: which sample each channel is chewing
//! through, at what pitch, how loudly, and where between the speakers.
//!
//! # The profile, and what it leaves alone
//!
//! This plays the module's notes, its sample loops, its channel volumes and its
//! panning, including Pxy and column Dx/Ex slides plus C0..CF placement. Instrument
//! envelopes apply fixed-point interpolation, sustain and loop on tracker ticks. Volume-column slides 6x/7x apply on nonzero
//! ticks; fine slides 8x/9x apply once at the start of a row. The effect column supports 8xx panning, 9xx sample offsets, Cxx channel volume,
//! Gxx global volume, Hxy global volume slides, EAx/EBx fine volume slides, tick-scheduled Axy volume slides and Kxx key-off, ECx volume cuts, E9x/Rxy sample retrigger, Txy tremor, 1xx/2xx pitch slides, E1x/E2x fine and X1x/X2x extra-fine pitch slides, E5x note finetune, 3xx/5xx and column Fx tone portamento with E3x glissando, 0xy arpeggio, 4xy/6xy and column Ax/Bx vibrato, 7xy tremolo with E4x/E7x wave controls, EDx note delay, EEx held rows and E6x pattern loops plus the three effects the container consumed while laying the timeline out
//! ([`xm::EFFECT_TEMPO`], [`xm::EFFECT_JUMP`], [`xm::EFFECT_BREAK`]). Every one of
//! those omissions is counted in [`Ignored`], which the player can be asked for,
//! so a module that needs what is missing is heard as a thin rendering rather than
//! mistaken for a correct one.
//!
//! Without a volume envelope, key-off cuts the channel volume, while the sample
//! keeps advancing. A later explicit volume or instrument-default command can
//! restore that sample; a note without an instrument retains the zero volume.
//! Instrument auto-vibrato applies after pattern modulation on every tick.
//! Lxx positions volume and reference-gated panning envelopes without restarting samples.
//! Volume envelopes release on key-off and instrument fadeout multiplies the
//! result. Envelopes with unrenderable point metadata are still reported.
//!
//! # Pitch
//!
//! Both of the format's tables are computed rather than looked up, from the two
//! laws libxm states them by, which keeps the octaves exact in the linear table
//! and holds the Amiga table to within about a percentile of it, as the
//! hardware's own clock does. [`frames_per_second`] explains the unit they are in.

use crate::audio::{AudioDecode, AudioPacket};
use crate::container::xm::{self, Cell, Sample};
use crate::{Result, invalid};

/// The reference the format's own linear table is built on: the frames per second
/// a channel consumes at C-4, which is note 49 in the file's numbering and the
/// rate the tracker's sample recorder worked to. A sample holding one cycle in 32
/// frames sounds at 261 Hz there - middle C - which is what ties the table to the
/// notes the tracker shows.
pub const LINEAR_C4: f64 = 8363.0;
/// The Amiga's DMA clock, which is what the other table counts periods against.
/// libxm takes the PAL value and notes that no reason asks for it over NTSC; the
/// two differ by less than the percentile this whole table sits off the linear one
/// by anyway.
const AMIGA_CLOCK: f64 = 7_093_789.2;
/// The period of C-4 in the same scale the file's own periods are in, where a
/// period is 32 of the Amiga's clock ticks rather than one.
const AMIGA_C4_PERIOD: f64 = 856.0;
/// The loudest a channel's volume byte can ask for. The format's own top, and the
/// divisor every volume becomes a gain through.
const MAX_VOLUME: u8 = 64;
/// Standard XM effect Cxx overrides the volume column on the same row.
const EFFECT_SET_VOLUME: u8 = 0x0C;
const EFFECT_VOLUME_SLIDE: u8 = 0x0A;
const EFFECT_PITCH_UP: u8 = 1;
const EFFECT_PITCH_DOWN: u8 = 2;
const EFFECT_TONE_PORTAMENTO: u8 = 3;
const EFFECT_TONE_VOLUME_SLIDE: u8 = 5;
const EFFECT_VIBRATO: u8 = 4;
const EFFECT_VIBRATO_VOLUME_SLIDE: u8 = 6;
const EFFECT_TREMOLO: u8 = 7;
const EFFECT_EXTRA_FINE_PITCH: u8 = 0x21;
/// FT2 pattern slots are valid even when the file has no instrument in them.
const INSTRUMENT_SLOT_MAX: u8 = 128;
const EFFECT_SET_PANNING: u8 = 0x08;
const EFFECT_PANNING_SLIDE: u8 = 0x19;
const EFFECT_MULTI_RETRIGGER: u8 = 0x1B;
const EFFECT_TREMOR: u8 = 0x1D;
const EFFECT_SAMPLE_OFFSET: u8 = 0x09;
const EFFECT_KEY_OFF: u8 = 0x14;
const EFFECT_EXTENDED: u8 = 0x0E;
const EFFECT_GLOBAL_VOLUME: u8 = 0x10;
const EFFECT_GLOBAL_VOLUME_SLIDE: u8 = 0x11;
/// The width of the panning scale, which is also its centre: FastTracker II shows
/// 0xA4 as centre and the fixtures hold that default, while libxm's mixing puts the
/// acoustic centre at half the scale; the second is what this renderer uses, so a
/// module left at the tracker's own default plays a little right of centre, which
/// is what every libxm-based player does with the same file.
const PAN_SPAN: f64 = 256.0;
/// How far one frame moves a channel's level toward what its cell asks for, in
/// full-scale units. Without it a note starting or stopping mid-wave is heard as a
/// click, which is why libxm slides its channel volumes at this same rate.
const LEVEL_STEP: f32 = 1.0 / 256.0;
/// What one channel at full volume is worth in the mix, so a module with all 32
/// busy still fits inside full scale. libxm's own compromise between too quiet and
/// clipping.
const AMPLIFICATION: f32 = 0.25;
/// The volume column's direct-set range: 0x10 is the bottom of the scale and 0x50
/// its top. Column 6x/7x slides on nonzero ticks, 8x/9x fine slides on
/// tick zero, Cx sets panning, and Dx/Ex slides pan. Other effects remain counted.
const VOLUME_SET_MIN: u8 = 0x10;
const VOLUME_SET_MAX: u8 = 0x50;

/// What a renderer of this profile leaves in the file unread. Counted once per
/// instrument for the shapes an instrument states, and once per cell for the
/// columns a row fills in, so the numbers say both how much of the module is
/// unaffected and how often the player was asked for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ignored {
    /// Cells whose effect column names an unsupported command.
    pub effects: u32,
    /// Cells whose volume column asks for an unsupported command.
    pub volume_columns: u32,
    /// Instruments whose enabled envelopes have unrenderable point metadata.
    pub envelopes: u32,
    /// Retained for report compatibility; native instrument fadeout is supported.
    pub fades: u32,
    /// Unavailable instrument components or notes without a playable sample.
    /// A valid empty selection and the note it requests share one report.
    pub missing_samples: u32,
    /// Notes the pitch table has no answer for, whether because the note itself is
    /// outside it or because the sample's own relative note moves the total there.
    pub unplayable_notes: u32,
}

/// Sixteenths of a semitone above the table's lowest note, which is the unit both
/// pitch laws count in: `16 * (note - 1) + finetune`. Outside the table's own
/// range of notes there is no pitch to compute, whatever the finetune says.
pub fn note_steps(note: u8, finetune: i8) -> Option<i16> {
    if !(xm::NOTE_MIN..=xm::NOTE_MAX).contains(&note) {
        return None;
    }
    i16::try_from(i32::from(note - xm::NOTE_MIN) * 16 + i32::from(finetune)).ok()
}

/// How fast a channel consumes the sample it is reading, in frames per second,
/// which is the number the format's tables state and the pitch's whole story: the
/// sample's own length and content decide what frequency that comes out as.
///
/// The linear table doubles at every twelve notes and puts C-4 exactly on
/// [`LINEAR_C4`]; the Amiga one is the same law measured on the hardware's clock
/// and lands about a percentile below it, which is why a module written to sound
/// right on the hardware sounds a touch sharp on the table and vice versa.
pub fn frames_per_second(linear: bool, steps: i16) -> Option<f64> {
    frames_per_fractional_steps(linear, f64::from(steps))
}

fn frames_per_fractional_steps(linear: bool, steps: f64) -> Option<f64> {
    period_rate(linear, note_period(linear, steps))
}

fn note_period(linear: bool, steps: f64) -> f64 {
    if linear { 7680.0 - steps * 4.0 }
    else { (32.0 * AMIGA_C4_PERIOD * f64::exp2(steps / -192.0)).round() }
}

fn period_rate(linear: bool, period: f64) -> Option<f64> {
    // Both tables are held in a period's own field, and a period of none would be
    // a division by zero rather than a very low note.
    if period < 1.0 || period > f64::from(u16::MAX) {
        return None;
    }
    Some(if linear {
        LINEAR_C4 * f64::exp2((4608.0 - period) / 768.0)
    } else {
        4.0 * AMIGA_CLOCK / (period * 2.0)
    })
}

#[derive(Clone, Copy, Default)]
struct EnvelopeState {
    tick: u16,
    point: usize,
    value: i16,
    delta: i16,
}

impl EnvelopeState {
    fn reset(&mut self) { *self = Self { tick: u16::MAX, ..Self::default() }; }

    fn release(&mut self, envelope: &xm::Envelope) {
        if !envelope.playable() { return; }
        let point = self.point.min(usize::from(envelope.length) - 1);
        if self.tick >= envelope.points[point].0 { self.tick = envelope.points[point].0.wrapping_sub(1); }
    }

    /// Lxx primes the interpolation accumulator one tick before its destination.
    /// Exact-node handling is left to advance(), preserving loop/sustain rules.
    fn position(&mut self, envelope: &xm::Envelope, tick: u8) {
        if !envelope.playable() { return; }
        self.tick = u16::from(tick).wrapping_sub(1);
        let n = usize::from(envelope.length);
        let mut point = 0;
        let mut update = true;
        if n > 1 {
            point = 1;
            for _ in 0..n - 1 {
                if u16::from(tick) < envelope.points[point].0 {
                    point -= 1;
                    let elapsed = i32::from(tick) - i32::from(envelope.points[point].0);
                    if elapsed == 0 { update = false; break; }
                    let (x0, y0) = envelope.points[point];
                    let (x1, y1) = envelope.points[point + 1];
                    let span = (x1 as i16).wrapping_sub(x0 as i16);
                    if span <= 0 { break; }
                    self.delta = ((i32::from(y1.wrapping_sub(y0) as i8) << 8) / i32::from(span)) as i16;
                    let interpolated = (i64::from(self.delta) * (i64::from(elapsed) - 1)) as i16;
                    self.value = ((y0 as i8 as i16) << 8).wrapping_add(interpolated);
                    point += 1;
                    update = false;
                    break;
                }
                point += 1;
            }
            if update { point = point.saturating_sub(1); }
        }
        point = point.min(n - 1);
        if update {
            self.delta = 0;
            self.value = (envelope.points[point].1 as i8 as i16) << 8;
        }
        self.point = point;
    }

    /// Fixed-point interpolation, sustain and loop transitions happen on ticks.
    fn advance(&mut self, envelope: &xm::Envelope, released: bool) -> u16 {
        let n = usize::from(envelope.length);
        let mut point = self.point.min(n - 1);
        let mut interpolated = false;
        self.tick = self.tick.wrapping_add(1);
        if self.tick == envelope.points[point].0 {
            self.value = (envelope.points[point].1 as i8 as i16) << 8;
            point += 1;
            if envelope.looped() {
                point -= 1;
                if point == usize::from(envelope.loop_end)
                    && (!envelope.sustain() || point != usize::from(envelope.sustain_point) || !released)
                {
                    point = usize::from(envelope.loop_start);
                    self.tick = envelope.points[point].0;
                    self.value = (envelope.points[point].1 as i8 as i16) << 8;
                }
                point += 1;
            }
            if point < n {
                if envelope.sustain() && !released && point - 1 == usize::from(envelope.sustain_point) {
                    self.delta = 0;
                } else {
                    self.point = point;
                    let (x0, y0) = envelope.points[point - 1];
                    let (x1, y1) = envelope.points[point];
                    let span = (x1 as i16).wrapping_sub(x0 as i16);
                    if span > 0 {
                        self.delta = ((i32::from(y1.wrapping_sub(y0) as i8) << 8) / i32::from(span)) as i16;
                        interpolated = true;
                    } else { self.delta = 0; }
                }
            } else { self.delta = 0; }
        }
        if !interpolated {
            self.value = self.value.wrapping_add(self.delta);
            let high = (self.value >> 8) as u8;
            if high > 64 {
                self.delta = 0;
                return if high <= 160 { 64 * 256 } else { 0 };
            }
        }
        (self.value as u16).min(64 * 256)
    }
}

/// What one channel is doing. A module states its channels once, so there is one
/// of these per channel and a cell with nothing in it leaves it as it was.
#[derive(Clone, Copy, Default)]
struct Voice {
    /// One-based instrument selected for the next note; zero means unselected.
    instrument: u8,
    /// One-based owner of the active sample, retained by instrument-only cells.
    playing_instrument: u8,
    /// And which of that instrument's samples its current note is reading.
    sample: usize,
    /// Last unmapped note number, retained for E9x retrigger selection.
    last_note: u8,
    /// Rxy has independent nibble memories and a counter retained across rows.
    multi_period: u8,
    multi_volume: u8,
    multi_count: u8,
    tremor_parameter: u8,
    tremor_remaining: u8,
    tremor_on: bool,
    tremor_muted: bool,
    /// Current base period; pitch slides preserve sample phase.
    period: f64,
    output_period: f64,
    auto_phase: u8,
    auto_amplitude: u16,
    auto_sweep: u16,
    auto_key_off: bool,
    volume_shape: EnvelopeState,
    pan_shape: EnvelopeState,
    envelope_gain: Option<f32>,
    envelope_pan: Option<u8>,
    fade_remaining: Option<u16>,
    fade_speed: u16,
    /// Ordinary, fine and extra-fine up/down parameters are independent.
    pitch_slide_memory: [u8; 6],
    tone_speed: u16,
    tone_target: f64,
    tone_direction: u8,
    glissando: bool,
    arpeggio_active: bool,
    main_vibrato: bool,
    vibrato_phase: u8,
    vibrato_speed: u8,
    vibrato_depth: u8,
    vibrato_shape: u8,
    tremolo_phase: u8,
    tremolo_speed: u8,
    tremolo_depth: u8,
    tremolo_shape: u8,
    /// Temporary output modulation persists until an explicit volume write.
    tremolo_output: Option<u8>,
    /// Active sample tuning, preserved when only the next instrument changes.
    pitch_finetune: i8,
    sounding: bool,
    /// Retain sample phase while muted. No-envelope key-off cuts channel volume
    /// without clearing this; the unsupported-envelope release profile clears it.
    held: bool,
    /// Where in the sample the channel is, and how far it moves per frame, both in
    /// the sample's own frames.
    position: f64,
    /// Last nonzero9xx argument, retained per channel across plain triggers.
    sample_offset: u8,
    /// Last executed nonzero Axy parameter, retained independently per channel.
    volume_slide: u8,
    /// H00 uses this channel's last executed Hxy parameter, not Axy memory.
    global_volume_slide: u8,
    /// Independent last executed Pxy parameter, used by P00.
    panning_slide: u8,
    /// Independent tick-zero EAx/EBx memories, separate from Axy.
    fine_volume_up: u8,
    fine_volume_down: u8,
    reverse: bool,
    step: f64,
    volume: u8,
    panning: u8,
    /// Where the level is, and where the last cell asked it to be, both already in
    /// the mix's units, so a channel's worth of gain is one multiply.
    level: f32,
    target: f32,
}

/// A renderer for the row packets `crate::container::xm` writes.
pub struct XmDecoder {
    module: xm::Module,
    sample_rate: u32,
    output_channels: usize,
    voices: Vec<Voice>,
    ignored: Ignored,
    global_volume: u8,
}

impl XmDecoder {
    /// Take the module to play, in the same bytes the container read it from: a
    /// module states its sound in full inside itself, so the setup data is the
    /// file and there is nothing for a packet to carry besides the row.
    pub fn new(extra_data: &[u8], sample_rate: u32, channels: u16) -> Result<Self> {
        if sample_rate == 0 {
            return Err(invalid("module rendering needs a sample rate to render at"));
        }
        let module = xm::Module::parse(extra_data, &xm::Limits::default())?;
        // Both counted over instruments at once, since neither is a thing a row
        // asks for: the module either states them or does not, and every row it
        // plays is affected by the same answer.
        let shaped = module
            .instruments
            .iter()
            .filter(|instrument| {
                (instrument.volume_envelope.on() && !instrument.volume_envelope.playable())
                    || (instrument.panning_envelope.on() && !instrument.panning_envelope.playable())
            })
            .count();
        let ignored = Ignored {
            envelopes: u32::try_from(shaped).unwrap_or(u32::MAX),
            fades: 0,
            ..Ignored::default()
        };
        let voices = vec![Voice::default(); module.channels];
        Ok(Self {
            sample_rate,
            output_channels: usize::from(channels.clamp(1, 8)),
            voices,
            module,
            ignored,
            global_volume: MAX_VOLUME,
        })
    }

    /// The module being played, for a listing of what was opened.
    pub fn module(&self) -> &xm::Module {
        &self.module
    }

    /// How much of the module this rendering has left unread so far.
    pub fn ignored(&self) -> Ignored {
        self.ignored
    }

    /// The cells of one row, applied to the channels they name.
    fn apply_row(&mut self, row: &xm::Row) {
        let linear = self.module.linear;
        let Self {
            module,
            voices,
            ignored,
            sample_rate,
            global_volume,
            ..
        } = self;
        for (voice, cell) in voices.iter_mut().zip(&row.cells) {
            apply_cell(module, voice, cell, ignored, linear, *sample_rate);
            if cell.effect == EFFECT_GLOBAL_VOLUME {
                *global_volume = cell.parameter.min(MAX_VOLUME);
            }
        }
    }

    /// One frame of the mix: what every channel is worth right now, summed by
    /// speaker. Returns the dry sum too, which is what a mono output takes, since
    /// placement means nothing when there is nowhere to place it.
    fn mix_frame(&mut self) -> (f32, f32, f32) {
        let Self { module, voices, .. } = self;
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        let mut dry = 0.0f32;
        for voice in voices.iter_mut() {
            if !voice.sounding {
                continue;
            }
            voice.level = if voice.target > voice.level {
                (voice.level + LEVEL_STEP).min(voice.target)
            } else {
                (voice.level - LEVEL_STEP).max(voice.target)
            };
            if voice.level == 0.0 && !voice.held {
                // The unsupported-envelope release profile ends here. A retained
                // zero-volume sample advances, preserving phase and its natural end
                // if the channel is made audible again later.
                voice.sounding = false;
                continue;
            }
            let Some(sample) = usize::from(voice.playing_instrument)
                .checked_sub(1)
                .and_then(|index| module.instruments.get(index))
                .and_then(|instrument| instrument.samples.get(voice.sample))
            else {
                voice.sounding = false;
                continue;
            };
            let Some(value) = read_frame(sample, voice.position) else {
                // An unlooped sample that has run out simply stops, which is what
                // a sample longer than its row sounds like too.
                voice.sounding = false;
                voice.level = 0.0;
                continue;
            };
            advance(voice, sample);
            let sample = value * voice.level;
            dry += sample;
            let pan = f64::from(voice.envelope_pan.unwrap_or(voice.panning)) / PAN_SPAN;
            left += sample * (1.0 - pan).sqrt() as f32;
            right += sample * pan.sqrt() as f32;
        }
        (left, right, dry)
    }
}

/// E1x/E2x and X1x/X2x act once on a new row; held rows skip setup.
fn fine_pitch_command(cell: &Cell) -> Option<(usize, u8)> {
    if !matches!(cell.parameter >> 4, 1 | 2) { return None; }
    let offset = match cell.effect {
        EFFECT_EXTENDED => 2,
        EFFECT_EXTRA_FINE_PITCH => 4,
        _ => return None,
    };
    Some((offset + usize::from((cell.parameter >> 4) - 1), cell.parameter & 15))
}

/// FT2 period arithmetic is 16-bit, including its signed down-slide clamp.
/// The native pitch profile retains its pre-existing half-period finetunes.
fn pitch_slide(voice: &mut Voice, memory: usize, parameter: u8, linear: bool, rate: u32) {
    if parameter != 0 { voice.pitch_slide_memory[memory] = parameter; }
    let amount = f64::from(voice.pitch_slide_memory[memory])
        * if memory < 4 { 4.0 } else { 1.0 };
    let up = memory.is_multiple_of(2);
    let period = (voice.period + if up { -amount } else { amount }).rem_euclid(65536.0);
    voice.period = if up {
        if period < 1.0 || period >= 32768.0 { 1.0 } else { period }
    } else if (32000.0..32768.0).contains(&period) { 31999.0 } else { period };
    set_period_step(voice, voice.period, linear, rate);
}

/// Volume-column Fx takes preparation priority over main-column 3xx/5xx.
/// E90 is the exception: it triggers normally and only runs Fx on later ticks.
fn prepares_tone(cell: &Cell) -> bool {
    !(is_retrigger(cell) && cell.parameter == 0x90)
        && (cell.volume >= 0xF0 || matches!(cell.effect, EFFECT_TONE_PORTAMENTO | EFFECT_TONE_VOLUME_SLIDE))
}

fn prepare_tone(module: &xm::Module, voice: &mut Voice, note: u8, linear: bool) {
    let sample = current_sample(module, voice);
    let relative = sample.map_or(0, |sample| sample.relative_note);
    let fine = voice.pitch_finetune;
    let total = i16::from(note) + i16::from(relative);
    if !(1..=119).contains(&total) { return; }
    let target = note_period(linear, f64::from((total - 1) * 16) + f64::from(fine) / 8.0);
    if !(1.0..=65535.0).contains(&target) { return; }
    voice.tone_target = target;
    voice.tone_direction = if target == voice.period { 0 } else if target > voice.period { 1 } else { 2 };
}

/// Advance toward the remembered target without reselecting or restarting a sample.
fn tone_portamento(voice: &mut Voice, linear: bool, rate: u32) {
    if voice.tone_direction == 0 { return; }
    let amount = f64::from(voice.tone_speed);
    if voice.tone_direction == 2 {
        voice.period = (voice.period - amount).rem_euclid(65536.0);
        let signed = |p: f64| if p >= 32768.0 { p - 65536.0 } else { p };
        if signed(voice.period) <= signed(voice.tone_target) {
            voice.period = voice.tone_target;
            voice.tone_direction = 1;
        }
    } else {
        voice.period = (voice.period + amount).rem_euclid(65536.0);
        if voice.period >= voice.tone_target { voice.period = voice.tone_target; }
    }
    let output = if voice.glissando { semitone_period(voice, 0, linear) } else { voice.period };
    set_period_step(voice, output, linear, rate);
}

fn set_period_step(voice: &mut Voice, period: f64, linear: bool, rate: u32) {
    voice.output_period = period;
    set_final_period_step(voice, period, linear, rate);
}

fn set_final_period_step(voice: &mut Voice, period: f64, linear: bool, rate: u32) {
    if period == 0.0 { voice.step = 0.0; return; }
    if let Some(hz) = period_rate(linear, period) { voice.step = hz / f64::from(rate); }
}

/// FT2's bounded note search, including its B-7 ceiling and half-semitone
/// comparison points. The values follow the existing native pitch laws.
fn semitone_period(voice: &Voice, offset: u8, linear: bool) -> f64 {
    let fine = i32::from(voice.pitch_finetune >> 3) + 16;
    let table = |index: i32| note_period(linear, f64::from(index - 16));
    let (mut low, mut high) = (0, 8 * 12 * 16);
    for _ in 0..8 {
        let middle = (((low + high) >> 1) & !15) + fine;
        if voice.period >= table((middle - 8).max(0)) {
            high = (middle - fine) & !15;
        } else { low = (middle - fine) & !15; }
    }
    let mut index = low + fine + (i32::from(offset) << 4);
    if index >= 8 * 12 * 16 + 14 { index = 8 * 12 * 16 + 15; }
    table(index)
}

/// The remaining-tick clock runs backwards. FT2's 16-entry table overflow
/// selects the low nibble above index16, except index16 itself selects base.
fn arpeggio(voice: &mut Voice, parameter: u8, speed: u8, tick: u8, linear: bool, rate: u32) {
    let remaining = speed.wrapping_sub(tick) & 31;
    let phase = if remaining < 16 { remaining % 3 } else if remaining == 16 { 0 } else { 2 };
    let period = match phase {
        0 => voice.period,
        1 => semitone_period(voice, parameter >> 4, linear),
        _ => semitone_period(voice, parameter & 15, linear),
    };
    set_period_step(voice, period, linear, rate);
}

/// Mathematical quarter-wave samples floor(255*sin(i*pi/32)), the tracker scale.
const MODULATION_SINE: [u8; 32] = [
    0,24,49,74,97,120,141,161,180,197,212,224,235,244,250,253,
    255,253,250,244,235,224,212,197,180,161,141,120,97,74,49,24,
];

fn modulation_wave(phase: u8, shape: u8, ramp_negative: bool) -> u8 {
    let index = (phase >> 2) & 31;
    match shape & 3 {
        0 => MODULATION_SINE[usize::from(index)],
        1 => { let ramp = index << 3; if ramp_negative { !ramp } else { ramp } }
        _ => 255,
    }
}

fn vibrato(voice: &mut Voice, parameter: u8, linear: bool, rate: u32) {
    if parameter >> 4 != 0 { voice.vibrato_speed = (parameter >> 4) * 4; }
    if parameter & 15 != 0 { voice.vibrato_depth = parameter & 15; }
    let negative = voice.vibrato_phase >= 128;
    let wave = modulation_wave(voice.vibrato_phase, voice.vibrato_shape, negative);
    let amount = f64::from((u16::from(wave) * u16::from(voice.vibrato_depth)) >> 5);
    let output = (voice.period + if negative { -amount } else { amount }).rem_euclid(65536.0);
    set_period_step(voice, output, linear, rate);
    voice.vibrato_phase = voice.vibrato_phase.wrapping_add(voice.vibrato_speed);
}

fn tremolo(voice: &mut Voice, parameter: u8) {
    if parameter >> 4 != 0 { voice.tremolo_speed = (parameter >> 4) * 4; }
    if parameter & 15 != 0 { voice.tremolo_depth = parameter & 15; }
    // FT2's ramp inversion uses the vibrato sign, not the tremolo sign.
    let wave = modulation_wave(voice.tremolo_phase, voice.tremolo_shape, voice.vibrato_phase >= 128);
    let amount = ((u16::from(wave) * u16::from(voice.tremolo_depth)) >> 6) as i16;
    let volume = i16::from(voice.volume) + if voice.tremolo_phase >= 128 { -amount } else { amount };
    voice.tremor_muted = false;
    voice.tremolo_output = Some(volume.clamp(0, i16::from(MAX_VOLUME)) as u8);
    refresh_voice_target(voice);
    voice.tremolo_phase = voice.tremolo_phase.wrapping_add(voice.tremolo_speed);
}

fn is_glissando(cell: &Cell) -> bool {
    cell.effect == EFFECT_EXTENDED && cell.parameter >> 4 == 3
}

fn is_note_delay(cell: &Cell) -> bool {
    cell.effect == EFFECT_EXTENDED && cell.parameter >> 4 == 0xD
}

fn is_set_finetune(cell: &Cell) -> bool {
    cell.effect == EFFECT_EXTENDED && cell.parameter >> 4 == 5
}

fn is_retrigger(cell: &Cell) -> bool {
    cell.effect == EFFECT_EXTENDED && cell.parameter >> 4 == 9
}

fn is_fine_volume_slide(cell: &Cell) -> bool {
    cell.effect == EFFECT_EXTENDED && matches!(cell.parameter >> 4, 0xA | 0xB)
}

fn is_note_cut(cell: &Cell) -> bool {
    cell.effect == EFFECT_EXTENDED && cell.parameter >> 4 == 0x0C
}

/// Without a volume envelope, release cuts volume but retains sample phase.
/// A renderable volume envelope instead releases from its sustain position.
fn key_off(module: &xm::Module, voice: &mut Voice) {
    voice.auto_key_off = true;
    let enveloped = usize::from(voice.playing_instrument).checked_sub(1)
        .and_then(|index| module.instruments.get(index))
        .is_some_and(|instrument| instrument.volume_envelope.on());
    if enveloped {
        if let Some(instrument) = playing_instrument(module, voice) {
            if instrument.volume_envelope.playable() {
                voice.volume_shape.release(&instrument.volume_envelope);
                voice.held = true;
            } else { voice.held = false; }
        }
    } else {
        voice.volume = 0;
        voice.tremor_muted = false; voice.tremolo_output = None;
        voice.held = true;
    }
    refresh_voice_target(voice);
}

/// One cell, applied to the channel it belongs to.
///
/// The order is the tracker's own, and it is the order that gives the answer: a
/// note is triggered first, so it plays on the instrument the same cell names; the
/// instrument's sample then sets the channel's volume and placement; and the volume
/// column follows, then supported standard effects may override its settings.
fn apply_cell(
    module: &xm::Module,
    voice: &mut Voice,
    cell: &Cell,
    ignored: &mut Ignored,
    linear: bool,
    sample_rate: u32,
) {
    // A new row always restores an outgoing arpeggio before handling its note,
    // including an EDx row which returns early after instrument selection.
    if voice.arpeggio_active || (voice.main_vibrato && !matches!(cell.effect, EFFECT_VIBRATO | EFFECT_VIBRATO_VOLUME_SLIDE)) {
        set_period_step(voice, voice.period, linear, sample_rate);
    }
    voice.main_vibrato = matches!(cell.effect, EFFECT_VIBRATO | EFFECT_VIBRATO_VOLUME_SLIDE);
    voice.arpeggio_active = cell.effect == 0 && cell.parameter != 0;
    // K00 takes precedence over a new note on the same row. Other Kxx
    // values are scheduled on nonzero ticks by the row renderer.
    let immediate_key_off = cell.effect == EFFECT_KEY_OFF && cell.parameter == 0;
    let delayed = is_note_delay(cell) && cell.parameter & 15 != 0;
    let tone = prepares_tone(cell);
    let note = if immediate_key_off && !tone { xm::NOTE_KEY_OFF } else { cell.note };
    let populated = usize::from(cell.instrument) <= module.instruments.len();
    let accepted_instrument = cell.instrument != 0
        && (populated || cell.instrument <= INSTRUMENT_SLOT_MAX);
    if cell.instrument != 0 {
        if accepted_instrument {
            voice.instrument = cell.instrument;
            // A note reports its missing sample in trigger. An instrument-only
            // cell reports the absent selection here, once for that cell.
            if !delayed && !populated && !(xm::NOTE_MIN..=xm::NOTE_MAX).contains(&note) {
                ignored.missing_samples += 1;
            }
        } else {
            ignored.missing_samples += 1;
        }
    }
    if delayed {
        // Selection is immediate; note/defaults/direct column commands wait.
        // Still report unknown column behavior once, even if no trigger occurs.
        if !matches!(cell.volume, 0 | VOLUME_SET_MIN..=VOLUME_SET_MAX | 0x60..=0xBF | 0xC0..=0xFF) {
            ignored.volume_columns += 1;
        }
        return;
    }
    if tone {
        if cell.volume >= 0xF0 {
            if cell.volume & 15 != 0 { voice.tone_speed = u16::from(cell.volume & 15) * 64; }
        } else if cell.effect == EFFECT_TONE_PORTAMENTO && cell.parameter != 0 {
            voice.tone_speed = u16::from(cell.parameter) * 4;
        }
    }
    match note {
        0 => {
            if is_retrigger(cell) && cell.parameter & 15 == 0 {
                retrigger(module, voice, ignored, linear, sample_rate);
            }
        }
        xm::NOTE_KEY_OFF => key_off(module, voice),
        xm::NOTE_MIN..=xm::NOTE_MAX if tone => prepare_tone(module, voice, note, linear),
        xm::NOTE_MIN..=xm::NOTE_MAX => {
            let fine = is_set_finetune(cell).then(|| ((cell.parameter & 15) * 16).wrapping_sub(128) as i8);
            trigger(module, voice, cell.note, fine, ignored, linear, sample_rate);
            if cell.effect == EFFECT_SAMPLE_OFFSET {
                if cell.parameter != 0 {
                    voice.sample_offset = cell.parameter;
                }
                // FT2 counts decoded sample frames, for both8and16-bit samples.
                let offset = usize::from(voice.sample_offset) * 256;
                voice.position = offset as f64;
                if let Some(sample) = current_sample(module, voice) {
                    let end = if sample.loops() { sample.loop_end } else { sample.frames.len() };
                    if offset >= end {
                        voice.sounding = false;
                        // A stopped voice has no old gain to ramp from on a
                        // later note, especially after a zero-volume command.
                        voice.level = 0.0;
                    }
                }
            }
        }
        // Above the table's own top there is no pitch to ask for, and a note that
        // cannot be named is not the same failure as an instrument that holds
        // nothing for it.
        _ => ignored.unplayable_notes += 1,
    }
    if accepted_instrument && note != xm::NOTE_KEY_OFF {
        voice.multi_count = 0;
        reset_instrument_modulation(module, voice);
    }
    // An accepted instrument component resets the active sample defaults.
    // Without a note it changes only the next selection, preserving the active
    // sample, pitch and phase. An untriggered channel has no defaults to restore.
    if accepted_instrument
        && let Some(sample) = current_sample(module, voice)
    {
        voice.tremor_muted = false; voice.tremolo_output = None;
        voice.volume = sample.volume.min(MAX_VOLUME);
        voice.panning = sample.panning;
    }
    if matches!(cell.volume, VOLUME_SET_MIN..=VOLUME_SET_MAX | 0x80..=0x9F) {
        voice.tremor_muted = false; voice.tremolo_output = None;
    }
    match cell.volume {
        0 | 0x60..=0x7F | 0xB0..=0xBF | 0xD0..=0xFF => {} // Ordinary column slides run on nonzero ticks.
        VOLUME_SET_MIN..=VOLUME_SET_MAX => voice.volume = cell.volume - VOLUME_SET_MIN,
        0x80..=0x8F => voice.volume = voice.volume.saturating_sub(cell.volume & 0x0F),
        0x90..=0x9F => voice.volume = voice.volume.saturating_add(cell.volume & 0x0F).min(MAX_VOLUME),
        0xA0..=0xAF => {
            if cell.volume & 15 != 0 { voice.vibrato_speed = (cell.volume & 15) * 4; }
        }
        0xC0..=0xCF => voice.panning = (cell.volume & 0x0F) << 4,
        _ => ignored.volume_columns += 1,
    }
    if cell.effect == EFFECT_SET_PANNING {
        voice.panning = cell.parameter;
    }
    if cell.effect == EFFECT_SET_VOLUME {
        voice.tremor_muted = false; voice.tremolo_output = None;
        voice.volume = cell.parameter.min(MAX_VOLUME);
    }
    if cell.effect == EFFECT_EXTENDED {
        match cell.parameter >> 4 {
            4 => voice.vibrato_shape = cell.parameter & 15,
            7 => voice.tremolo_shape = cell.parameter & 15,
            _ => {}
        }
    }
    if cell.effect == 0x15 {
        if let Some(instrument) = playing_instrument(module, voice) {
            if instrument.volume_envelope.on() {
                voice.volume_shape.position(&instrument.volume_envelope, cell.parameter);
            }
            // FT2 gates pan positioning on the volume sustain flag, even when
            // volume shaping is disabled, rather than on the pan enabled flag.
            if instrument.volume_envelope.sustain() {
                voice.pan_shape.position(&instrument.panning_envelope, cell.parameter);
            }
        }
    }
    if is_glissando(cell) { voice.glissando = cell.parameter & 15 != 0; }
    if is_fine_volume_slide(cell) {
        fine_volume_slide(voice, cell.parameter);
    }
    if let Some((memory, amount)) = fine_pitch_command(cell) {
        pitch_slide(voice, memory, amount, linear, sample_rate);
    }
    if is_note_cut(cell) && cell.parameter & 15 == 0 {
        voice.tremor_muted = false; voice.tremolo_output = None;
        voice.volume = 0;
    }
    if cell.effect == EFFECT_MULTI_RETRIGGER {
        if cell.parameter & 15 != 0 { voice.multi_period = cell.parameter & 15; }
        if cell.parameter >> 4 != 0 { voice.multi_volume = cell.parameter >> 4; }
        // FT2 uses the volume-column handler's result, not just the input byte.
        let column_result = match cell.volume {
            VOLUME_SET_MIN..=VOLUME_SET_MAX | 0x80..=0x9F => voice.volume,
            0xA0..=0xAF => (cell.volume & 15) * 4,
            0xC0..=0xCF => voice.panning,
            _ => cell.volume,
        };
        if column_result == 0 {
            multi_retrigger(module, voice, cell, ignored, linear, sample_rate);
        }
    }
    if cell.effect != 0
        && !(cell.effect == EFFECT_EXTENDED && matches!(cell.parameter >> 4, 6 | 0xE))
        && !is_note_cut(cell)
        && !is_fine_volume_slide(cell)
        && !is_retrigger(cell)
        && !is_set_finetune(cell)
        && !is_note_delay(cell)
        && !is_glissando(cell)
        && !(cell.effect == EFFECT_EXTENDED && matches!(cell.parameter >> 4, 4 | 7))
        && fine_pitch_command(cell).is_none()
        && !immediate_key_off
        && !matches!(
            cell.effect,
            0x15 | EFFECT_VIBRATO | EFFECT_VIBRATO_VOLUME_SLIDE | EFFECT_TREMOLO | EFFECT_TONE_PORTAMENTO | EFFECT_TONE_VOLUME_SLIDE | EFFECT_PITCH_UP | EFFECT_PITCH_DOWN | EFFECT_TREMOR | EFFECT_MULTI_RETRIGGER | EFFECT_KEY_OFF | EFFECT_SET_PANNING | EFFECT_PANNING_SLIDE | EFFECT_SAMPLE_OFFSET | EFFECT_SET_VOLUME | EFFECT_VOLUME_SLIDE | EFFECT_GLOBAL_VOLUME | EFFECT_GLOBAL_VOLUME_SLIDE | xm::EFFECT_TEMPO | xm::EFFECT_JUMP | xm::EFFECT_BREAK
        )
    {
        ignored.effects += 1;
    }
    refresh_voice_target(voice);
}

/// Axy executes only on nonzero ticks. FT2 gives the upper nibble priority
/// when both are set, and A00 reuses the channel's last executed parameter.
fn volume_slide(voice: &mut Voice, parameter: u8) {
    if parameter != 0 { voice.volume_slide = parameter; }
    let parameter = voice.volume_slide;
    adjust_voice_volume(voice, parameter >> 4, parameter & 15);
}

/// EAx/EBx executes once on tick zero, after the volume column. A zero
/// parameter reuses only that channel's memory for the same direction.
fn fine_volume_slide(voice: &mut Voice, parameter: u8) {
    let up = parameter >> 4 == 0xA;
    let memory = if up { &mut voice.fine_volume_up } else { &mut voice.fine_volume_down };
    if parameter & 15 != 0 { *memory = parameter & 15; }
    let amount = *memory;
    if up { adjust_voice_volume(voice, amount, 0); }
    else { adjust_voice_volume(voice, 0, amount); }
}

/// Apply Hxy immediately in channel order, since clipping makes simultaneous
/// slides noncommutative. Only nonzero ticks execute/store these parameters.
fn global_volume_slide(global: &mut u8, voice: &mut Voice, parameter: u8) {
    if parameter != 0 { voice.global_volume_slide = parameter; }
    let parameter = voice.global_volume_slide;
    *global = if parameter >> 4 != 0 {
        global.saturating_add(parameter >> 4).min(MAX_VOLUME)
    } else {
        global.saturating_sub(parameter & 15)
    };
}

/// Main-column Pxy has effect memory; column Dx/Ex deliberately does not.
fn panning_slide(voice: &mut Voice, parameter: u8) {
    if parameter != 0 { voice.panning_slide = parameter; }
    let parameter = voice.panning_slide;
    voice.panning = if parameter >> 4 != 0 {
        voice.panning.saturating_add(parameter >> 4)
    } else {
        voice.panning.saturating_sub(parameter & 15)
    };
}

/// Rxy increments its own persistent counter; a firing applies the volume
/// table, then column overrides, then a phase restart without default reload.
fn multi_retrigger(module: &xm::Module, voice: &mut Voice, cell: &Cell, ignored: &mut Ignored, linear: bool, rate: u32) {
    voice.multi_count += 1;
    if voice.multi_count < voice.multi_period { return; }
    voice.multi_count = 0;
    let v = i16::from(voice.volume);
    voice.volume = match voice.multi_volume {
        1 => v-1, 2 => v-2, 3 => v-4, 4 => v-8, 5 => v-16,
        6 => (v>>1)+(v>>3)+(v>>4), 7 => v>>1,
        9 => v+1, 10 => v+2, 11 => v+4, 12 => v+8, 13 => v+16,
        14 => v+(v>>1), 15 => v*2, _ => v,
    }.clamp(0, i16::from(MAX_VOLUME)) as u8;
    match cell.volume {
        VOLUME_SET_MIN..=VOLUME_SET_MAX => voice.volume = cell.volume - VOLUME_SET_MIN,
        0xC0..=0xCF => voice.panning = (cell.volume & 15) << 4,
        _ => {}
    }
    voice.tremor_muted = false; voice.tremolo_output = None;
    retrigger(module, voice, ignored, linear, rate);
}

fn adjust_voice_volume(voice: &mut Voice, up: u8, down: u8) {
    voice.tremor_muted = false; voice.tremolo_output = None;
    voice.volume = if up != 0 {
        voice.volume.saturating_add(up).min(MAX_VOLUME)
    } else {
        voice.volume.saturating_sub(down)
    };
    refresh_voice_target(voice);
}

fn refresh_voice_target(voice: &mut Voice) {
    voice.target = if voice.held && !voice.tremor_muted {
        f32::from(voice.tremolo_output.unwrap_or(voice.volume)) / f32::from(MAX_VOLUME) * AMPLIFICATION
            * voice.envelope_gain.unwrap_or(1.0) * (f32::from(voice.fade_remaining.unwrap_or(32768)) / 32768.0)
    } else { 0.0 };
}

fn reset_instrument_modulation(module: &xm::Module, voice: &mut Voice) {
    voice.auto_key_off = false;
    if let Some(instrument) = playing_instrument(module, voice) {
        voice.fade_remaining = Some(32768);
        voice.fade_speed = instrument.fadeout;
        voice.envelope_gain = None;
        voice.envelope_pan = None;
        if instrument.volume_envelope.on() { voice.volume_shape.reset(); }
        if instrument.panning_envelope.on() { voice.pan_shape.reset(); }
        let auto = instrument.auto_vibrato;
        if auto.depth != 0 {
            voice.auto_phase = 0;
            if auto.sweep != 0 {
                voice.auto_amplitude = 0;
                voice.auto_sweep = (u16::from(auto.depth) << 8) / u16::from(auto.sweep);
            } else {
                voice.auto_amplitude = u16::from(auto.depth) << 8;
                voice.auto_sweep = 0;
            }
        }
    }
    if voice.vibrato_shape & 4 == 0 { voice.vibrato_phase = 0; }
    if voice.tremolo_shape & 4 == 0 { voice.tremolo_phase = 0; }
    voice.tremor_on = false;
    voice.tremor_remaining = 0;
}

/// Txy runs only in the ordinary tick dispatcher. Its phase pauses between
/// commands; output muting survives a blank row until a volume write restores it.
fn tremor(voice: &mut Voice, parameter: u8) {
    if parameter != 0 { voice.tremor_parameter = parameter; }
    if voice.tremor_remaining == 0 {
        voice.tremor_on = !voice.tremor_on;
        voice.tremor_remaining = if voice.tremor_on {
            voice.tremor_parameter >> 4
        } else { voice.tremor_parameter & 15 };
    } else { voice.tremor_remaining -= 1; }
    voice.tremor_muted = !voice.tremor_on;
    voice.tremolo_output = None;
    refresh_voice_target(voice);
}

fn playing_instrument<'a>(module: &'a xm::Module, voice: &Voice) -> Option<&'a xm::Instrument> {
    usize::from(voice.playing_instrument).checked_sub(1).and_then(|index| module.instruments.get(index))
}

fn needs_instrument_ticks(instrument: &xm::Instrument) -> bool {
    instrument.auto_vibrato.depth != 0 || instrument.fadeout != 0
        || (instrument.volume_envelope.on() && instrument.volume_envelope.playable())
        || (instrument.panning_envelope.on() && instrument.panning_envelope.playable())
}

fn instrument_tick(module: &xm::Module, voice: &mut Voice, linear: bool, rate: u32) {
    let Some(instrument) = playing_instrument(module, voice) else { return; };
    if voice.auto_key_off {
        let before = voice.fade_remaining.unwrap_or(32768);
        voice.fade_remaining = Some(before.saturating_sub(voice.fade_speed));
        if voice.fade_speed > before { voice.fade_speed = 0; }
    }
    let volume = &instrument.volume_envelope;
    voice.envelope_gain = if volume.on() && volume.playable() {
        Some(f32::from(voice.volume_shape.advance(volume, voice.auto_key_off)) / 16384.0)
    } else { None };
    let pan = &instrument.panning_envelope;
    voice.envelope_pan = if pan.on() && pan.playable() {
        let value = i32::from(voice.pan_shape.advance(pan, voice.auto_key_off)) - 8192;
        let width = (128 - (i32::from(voice.panning) - 128).abs()) << 3;
        let add = ((value * width) >> 16) as i8;
        Some(voice.panning.wrapping_add(add as u8))
    } else { None };
    refresh_voice_target(voice);
    auto_vibrato(module, voice, linear, rate);
}

/// Applied once per tracker tick, after pattern effects, including normal tick0.
/// Its output never feeds back into the pattern/base period.
fn auto_vibrato(module: &xm::Module, voice: &mut Voice, linear: bool, rate: u32) {
    let Some(instrument) = playing_instrument(module, voice) else { return; };
    let auto = instrument.auto_vibrato;
    if auto.depth == 0 { return; }
    let mut amplitude = voice.auto_amplitude;
    if voice.auto_sweep != 0 {
        amplitude = voice.auto_sweep;
        if !voice.auto_key_off {
            amplitude = amplitude.wrapping_add(voice.auto_amplitude);
            if amplitude >> 8 > u16::from(auto.depth) {
                amplitude = u16::from(auto.depth) << 8;
                voice.auto_sweep = 0;
            }
            voice.auto_amplitude = amplitude;
        }
    }
    voice.auto_phase = voice.auto_phase.wrapping_add(auto.rate);
    let phase = i32::from(voice.auto_phase);
    let wave = match auto.waveform {
        1 => if phase > 127 { 64 } else { -64 },
        2 => (((phase >> 1) + 64) & 127) - 64,
        3 => ((-(phase >> 1) + 64) & 127) - 64,
        _ => {
            static SINE: std::sync::OnceLock<[i8; 256]> = std::sync::OnceLock::new();
            let table = SINE.get_or_init(|| std::array::from_fn(|i| {
                (64.0 * (-(i as f64) * std::f64::consts::TAU / 256.0).sin()).round() as i8
            }));
            i32::from(table[usize::from(voice.auto_phase)])
        }
    };
    let offset = (wave * i32::from(amplitude as i16)) >> 14;
    let period = (voice.output_period + f64::from(offset)).rem_euclid(65536.0);
    set_final_period_step(voice, if period >= 32000.0 { 0.0 } else { period }, linear, rate);
}

fn current_sample<'a>(module: &'a xm::Module, voice: &Voice) -> Option<&'a Sample> {
    module
        .instruments
        .get(usize::from(voice.playing_instrument).checked_sub(1)?)?
        .samples
        .get(voice.sample)
}

/// Start the note a cell names on its channel's instrument.
fn trigger(
    module: &xm::Module,
    voice: &mut Voice,
    note: u8,
    finetune: Option<i8>,
    ignored: &mut Ignored,
    linear: bool,
    sample_rate: u32,
) {
    voice.last_note = note;
    let Some(instrument) = usize::from(voice.instrument).checked_sub(1) else {
        ignored.missing_samples += 1;
        silence(voice);
        return;
    };
    let Some(slot) = module.sample_slot(instrument, note) else {
        ignored.missing_samples += 1;
        silence(voice);
        return;
    };
    let Some(sample) = module.instruments[instrument].samples.get(slot) else {
        ignored.missing_samples += 1;
        return;
    };
    // A sample's relative note is the note it was recorded at, stated as an offset
    // from the one the cell names, so the pitch the table is asked for is the sum.
    let total = i16::from(note) + i16::from(sample.relative_note);
    // Pattern bytes stop at B-7, but sample transposition extends the
    // sounding pitch through A#9. A transposed 97 is a pitch, not key-off.
    let steps = (1..=119).contains(&total).then(|| (total - 1) * 16);
    let Some(steps) = steps else {
        ignored.unplayable_notes += 1;
        silence(voice);
        return;
    };
    // Raw XM finetune is 1/128 semitone; the public pitch helper counts
    // sixteenths. Keep the fractional conversion rather than dropping low bits.
    let finetune = finetune.unwrap_or(sample.finetune);
    let tuned_steps = f64::from(steps) + f64::from(finetune) / 8.0;
    let Some(rate) = frames_per_fractional_steps(linear, tuned_steps) else {
        ignored.unplayable_notes += 1;
        return;
    };
    voice.playing_instrument = voice.instrument;
    voice.sample = slot;
    voice.position = 0.0;
    voice.reverse = false;
    voice.pitch_finetune = finetune;
    voice.period = note_period(linear, tuned_steps);
    voice.output_period = voice.period;
    voice.step = rate / f64::from(sample_rate);
    voice.sounding = true;
    voice.held = true;
}

/// E9x reselects the remembered note through the current instrument mapping.
/// It restarts phase/direction/pitch without restoring channel volume or pan.
fn retrigger(module: &xm::Module, voice: &mut Voice, ignored: &mut Ignored, linear: bool, rate: u32) {
    if voice.last_note != 0 {
        trigger(module, voice, voice.last_note, None, ignored, linear, rate);
        refresh_voice_target(voice);
    }
}

/// Apply only the commands FT2 admits at an EDx deadline. Fine column slides
/// skipped on tick zero do not reappear here; ordinary column effects already
/// executed before the delayed trigger on this nonzero tick.
fn apply_delayed_cell(module: &xm::Module, voice: &mut Voice, cell: &Cell, ignored: &mut Ignored, linear: bool, rate: u32) {
    let mut due = *cell;
    if due.note == 0 { due.note = voice.last_note; }
    if usize::from(due.instrument) > module.instruments.len() && due.instrument > INSTRUMENT_SLOT_MAX {
        due.instrument = 0; // Invalid selection was reported on tick zero.
    }
    if !matches!(due.volume, VOLUME_SET_MIN..=VOLUME_SET_MAX | 0xC0..=0xCF) {
        due.volume = 0;
    }
    due.effect = 0;
    due.parameter = 0;
    apply_cell(module, voice, &due, ignored, linear, rate);
    voice.multi_count = 0;
    reset_instrument_modulation(module, voice);
}

/// Take the channel's note off in the frame it is asked for, which is what a note
/// that turns out to be unplayable is: no ramp, because there is no note to let go
/// of - the key was never down on anything.
fn silence(voice: &mut Voice) {
    voice.held = false;
    voice.sounding = false;
    voice.level = 0.0;
    voice.target = 0.0;
}

/// The frame under a channel's cursor, and where the cursor goes next.
fn read_frame(sample: &Sample, position: f64) -> Option<f32> {
    let frames = &sample.frames;
    let last = frames.len().checked_sub(1)?;
    let index = position.floor();
    if index < 0.0 || index > last as f64 {
        return None;
    }
    let a = index as usize;
    // Between two frames, the one above is the neighbour - across a loop's seam
    // that is the loop's own start rather than the frame the file holds past it,
    // which is what keeps a looped cycle from gaining a click at the join. A
    // ping-pong loop turns around inside its own range instead of joining, so its
    // neighbour stays the next frame.
    let b = if sample.loops() && !sample.ping_pong && a + 1 >= sample.loop_end {
        sample.loop_start
    } else {
        (a + 1).min(last)
    };
    let value = |frame: usize| f32::from(frames[frame]) / 32768.0;
    let weight = (position - index) as f32;
    Some(value(a) + (value(b) - value(a)) * weight)
}

/// Move the cursor, and follow the sample's own loop once it reaches the end of
/// what it plays.
fn advance(voice: &mut Voice, sample: &Sample) {
    if sample.loops() && sample.ping_pong && sample.loop_end - sample.loop_start > 1 {
        let start = sample.loop_start as f64;
        let span = (sample.loop_end - sample.loop_start - 1) as f64;
        let period = 2.0 * span;
        // Recover the unfolded phase from both position and direction. A folded
        // position alone loses which half of the loop is currently playing.
        let phase = if voice.reverse {
            period - (voice.position - start)
        } else {
            voice.position - start
        } + voice.step;
        if !voice.reverse && phase < span {
            // Includes the initial, non-looping prefix before loop_start.
            voice.position += voice.step;
            return;
        }
        let phase = phase.rem_euclid(period);
        voice.reverse = phase >= span;
        voice.position = start + phase.min(period - phase);
        return;
    }
    voice.position += voice.step;
    if sample.loops() && voice.position >= sample.loop_end as f64 {
        let start = sample.loop_start as f64;
        let length = (sample.loop_end - sample.loop_start) as f64;
        voice.position = start + (voice.position - start).rem_euclid(length);
    }
}

impl AudioDecode for XmDecoder {
    /// Render one row. The row is as long as its own tempo makes it, which the
    /// packet states in the milliseconds the pipeline counts in.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        duration: u64,
    ) -> Result<Option<AudioPacket>> {
        let row = xm::read_row(data)?;
        let continuation = data[7] & xm::ROW_CONTINUATION_FLAG != 0;
        let sample_rate = self.sample_rate;
        let channels = self.output_channels;
        // Round absolute endpoints once, rather than losing a fractional PCM
        // sample independently in every row. Adjacent rows then share a grid.
        let start = u128::from(pts) * u128::from(sample_rate) / 1_000;
        let end = (u128::from(pts) + u128::from(duration)) * u128::from(sample_rate) / 1_000;
        let output_pts = u64::try_from(start)
            .map_err(|_| invalid("module presentation exceeds the sample clock"))?;
        let frames = usize::try_from(end - start)
            .map_err(|_| invalid("module row is longer than any block this renderer writes"))?;
        let samples = frames.checked_mul(channels)
            .ok_or_else(|| invalid("module row sample count overflows"))?;
        let mut output = Vec::new();
        output.try_reserve_exact(samples)
            .map_err(|_| invalid("module row allocation failed"))?;
        output.resize(samples, 0.0f32);
        if !continuation {
            self.apply_row(&row);
            for voice in &mut self.voices { instrument_tick(&self.module, voice, self.module.linear, sample_rate); }
        }
        let mut global_gain = f32::from(self.global_volume) / f32::from(MAX_VOLUME);
        // Delayed Kxx uses five argument bits; ECx uses its low nibble.
        // Traverse deadlines without allocating or scanning cells per sample.
        let effect_tick = |cell: &Cell| {
            let tick = if cell.effect == EFFECT_KEY_OFF {
                cell.parameter & 31
            } else if is_note_cut(cell) || is_note_delay(cell) {
                cell.parameter & 15
            } else { return None; };
            ((tick != 0 || continuation) && tick < row.speed).then_some(tick)
        };
        let automatic_ticks = self.voices.iter().any(|voice| {
            playing_instrument(&self.module, voice).is_some_and(needs_instrument_ticks)
        });
        let mut regular_ticks = automatic_ticks || row.cells.iter().any(|cell| {
            (cell.effect == 0 && cell.parameter != 0)
                ||
            matches!(cell.effect, EFFECT_VOLUME_SLIDE | EFFECT_GLOBAL_VOLUME_SLIDE | EFFECT_PANNING_SLIDE | EFFECT_MULTI_RETRIGGER | EFFECT_TREMOR | EFFECT_PITCH_UP | EFFECT_PITCH_DOWN | EFFECT_TONE_PORTAMENTO | EFFECT_TONE_VOLUME_SLIDE | EFFECT_VIBRATO | EFFECT_VIBRATO_VOLUME_SLIDE | EFFECT_TREMOLO)
                || matches!(cell.volume, 0x60..=0x7F | 0xB0..=0xBF | 0xD0..=0xFF)
                || (is_retrigger(cell) && cell.parameter & 15 != 0)
        });
        let mut next_tick = if regular_ticks && (row.speed > 1 || continuation) {
            Some(if continuation { 0 } else { 1 })
        } else { row.cells.iter().filter_map(effect_tick).min() };
        let tick_frame = |tick: u8| {
            let at = u128::from(pts) * 1_000
                + u128::from(xm::tick_span_microseconds(row.bpm, tick));
            (at * u128::from(sample_rate) / 1_000_000).saturating_sub(start)
        };
        // The endpoint iteration applies deadlines sharing a rounded row end,
        // including rows too short to contain one output sample.
        for frame in 0..=frames {
            while let Some(tick) = next_tick.filter(|&tick| tick_frame(tick) <= frame as u128) {
                for (voice, cell) in self.voices.iter_mut().zip(&row.cells) {
                    // FT2 applies the volume column before the ordinary effect.
                    // Column zero parameters have no A00-style effect memory.
                    match cell.volume {
                        0x60..=0x6F => adjust_voice_volume(voice, 0, cell.volume & 15),
                        0x70..=0x7F => adjust_voice_volume(voice, cell.volume & 15, 0),
                        0xB0..=0xBF => vibrato(voice, cell.volume & 15, self.module.linear, sample_rate),
                        0xD0..=0xDF => {
                            // FT2's byte-wrap pan-left quirk: D0 jumps to zero.
                            voice.panning = if cell.volume & 15 == 0 { 0 }
                                else { voice.panning.saturating_sub(cell.volume & 15) };
                        }
                        0xE0..=0xEF => voice.panning = voice.panning.saturating_add(cell.volume & 15),
                        0xF0..=0xFF => tone_portamento(voice, self.module.linear, sample_rate),
                        _ => {}
                    }
                    if matches!(cell.effect, EFFECT_VIBRATO | EFFECT_VIBRATO_VOLUME_SLIDE) {
                        vibrato(voice, if cell.effect == EFFECT_VIBRATO { cell.parameter } else { 0 }, self.module.linear, sample_rate);
                    }
                    if cell.effect == EFFECT_TREMOLO { tremolo(voice, cell.parameter); }
                    if cell.effect == 0 && cell.parameter != 0 {
                        arpeggio(voice, cell.parameter, row.speed, tick, self.module.linear, sample_rate);
                    }
                    if matches!(cell.effect, EFFECT_TONE_PORTAMENTO | EFFECT_TONE_VOLUME_SLIDE) {
                        tone_portamento(voice, self.module.linear, sample_rate);
                    }
                    if matches!(cell.effect, EFFECT_PITCH_UP | EFFECT_PITCH_DOWN) {
                        pitch_slide(voice, usize::from(cell.effect - 1), cell.parameter, self.module.linear, sample_rate);
                    }
                    if matches!(cell.effect, EFFECT_VOLUME_SLIDE | EFFECT_TONE_VOLUME_SLIDE | EFFECT_VIBRATO_VOLUME_SLIDE) {
                        volume_slide(voice, cell.parameter);
                    }
                    if cell.effect == EFFECT_GLOBAL_VOLUME_SLIDE {
                        global_volume_slide(&mut self.global_volume, voice, cell.parameter);
                    }
                    if cell.effect == EFFECT_PANNING_SLIDE {
                        panning_slide(voice, cell.parameter);
                    }
                    if cell.effect == EFFECT_TREMOR {
                        tremor(voice, cell.parameter);
                    }
                    if cell.effect == EFFECT_MULTI_RETRIGGER {
                        multi_retrigger(&self.module, voice, cell, &mut self.ignored, self.module.linear, sample_rate);
                    }
                    if is_retrigger(cell) && cell.parameter & 15 != 0
                        && tick.is_multiple_of(cell.parameter & 15)
                    {
                        retrigger(&self.module, voice, &mut self.ignored, self.module.linear, sample_rate);
                        voice.multi_count = 0;
                        reset_instrument_modulation(&self.module, voice);
                    }
                    if effect_tick(cell) == Some(tick) {
                        if is_note_delay(cell) {
                            apply_delayed_cell(&self.module, voice, cell, &mut self.ignored, self.module.linear, sample_rate);
                        } else {
                            if is_note_cut(cell) {
                                voice.volume = 0; voice.tremor_muted = false; voice.tremolo_output = None;
                                refresh_voice_target(voice);
                            } else { key_off(&self.module, voice); }
                        }
                    }
                }
                for voice in &mut self.voices { instrument_tick(&self.module, voice, self.module.linear, sample_rate); }
                // EDx can start the first automatic instrument mid-row. Keep
                // dispatching following ticks even when row setup had no voice.
                if !regular_ticks {
                    regular_ticks = self.voices.iter().any(|voice| {
                        playing_instrument(&self.module, voice).is_some_and(needs_instrument_ticks)
                    });
                }
                global_gain = f32::from(self.global_volume) / f32::from(MAX_VOLUME);
                next_tick = if regular_ticks && tick + 1 < row.speed {
                    Some(tick + 1)
                } else {
                    row.cells.iter().filter_map(effect_tick).filter(|&other| other > tick).min()
                };
            }
            if frame == frames { break; }
            let (left, right, dry) = self.mix_frame();
            let frame = frame * channels;
            if channels == 1 {
                output[frame] = (dry * global_gain).clamp(-1.0, 1.0);
                continue;
            }
            output[frame] = (left * global_gain).clamp(-1.0, 1.0);
            output[frame + 1] = (right * global_gain).clamp(-1.0, 1.0);
            // What a wide layout gets besides the two speakers the module states
            // is nothing: a tracker places sound between left and right and has no
            // word for the rest.
        }
        let bytes = output
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect::<Vec<_>>();
        Ok(Some(AudioPacket {
            data: bytes,
            pts: output_pts,
            timebase_num: 1,
            timebase_den: sample_rate,
        }))
    }

    fn reset(&mut self) {
        self.global_volume = MAX_VOLUME;
        for voice in &mut self.voices {
            *voice = Voice::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AudioDecode, Cell, Ignored, Sample, XmDecoder, frames_per_second, note_steps, read_frame,
    };
    use crate::container::xm;

    /// The committed fixture: two channels, one instrument holding an 8-bit
    /// square that does not loop and a 16-bit sine that does, and three rows - the
    /// notes, a row holding them, and the key-off.
    const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/xm/two-notes.xm");
    const RATE: u32 = 44_100;

    fn opened(channels: u16) -> XmDecoder {
        XmDecoder::new(FIXTURE, RATE, channels).expect("the fixture is a module")
    }

    /// One row as its packet states it, so a test can ask a channel to do
    /// something the committed fixture's own rows do not.
    fn row_packet(bpm: u16, speed: u8, cells: &[Cell]) -> Vec<u8> {
        let duration = u32::try_from(u64::from(speed) * xm::tick_microseconds(bpm))
            .expect("a row this test writes is minutes at most");
        let mut out = Vec::new();
        out.extend_from_slice(&duration.to_le_bytes());
        out.extend_from_slice(&bpm.to_le_bytes());
        out.push(speed);
        out.push(0);
        for cell in cells {
            out.extend_from_slice(&[
                cell.note,
                cell.instrument,
                cell.volume,
                cell.effect,
                cell.parameter,
            ]);
        }
        out
    }

    fn cell(note: u8, instrument: u8, volume: u8, effect: u8, parameter: u8) -> Cell {
        Cell {
            note,
            instrument,
            volume,
            effect,
            parameter,
        }
    }

    /// Render one packet and read its PCM back as interleaved samples.
    fn render(decoder: &mut XmDecoder, data: &[u8], milliseconds: u64) -> Vec<f32> {
        let audio = decoder
            .decode_encoded(data, 0, milliseconds)
            .expect("renders")
            .expect("a packet of PCM");
        assert_eq!((audio.timebase_num, audio.timebase_den), (1, decoder.sample_rate));
        audio
            .data
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four bytes")))
            .collect()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples
            .iter()
            .fold(0.0f32, |loudest, sample| loudest.max(sample.abs()))
    }

    fn sample(frames: Vec<i16>, loop_start: usize, loop_end: usize, ping_pong: bool) -> Sample {
        Sample {
            name: String::new(),
            frames,
            loop_start,
            loop_end,
            ping_pong,
            volume: 64,
            finetune: 0,
            panning: 0,
            relative_note: 0,
        }
    }

    #[test]
    fn the_linear_table_is_the_formats_own_reference() {
        // C-4 is note 49, and the table's reference rate is exactly its own.
        let c4 = note_steps(49, 0).expect("a note in the table");
        assert_eq!(c4, 768);
        assert_eq!(frames_per_second(true, c4), Some(super::LINEAR_C4));
        // A-4 is the semitone scale's anchor: 32 frames of sample make one cycle,
        // so the note sounds a percentile-and-a-half below 440 Hz on a linear
        // module and that is what the tracker's own tuning is.
        let a4 = note_steps(58, 0).expect("a note in the table");
        let audible = frames_per_second(true, a4).expect("a period the table holds") / 32.0;
        assert!((audible - 439.5).abs() < 0.1, "A-4 sounds at {audible}");
        // An octave is exactly double, and a semitone exactly the twelfth root -
        // which is what a table computed from a law holds and a hand-typed one
        // rounds away.
        let octave_up = frames_per_second(true, c4 + 192).expect("in range");
        let semitone = frames_per_second(true, c4 + 16).expect("in range");
        assert!((octave_up / c4_rate() - 2.0).abs() < 1e-9);
        assert!((semitone / c4_rate() - 2.0f64.powf(1.0 / 12.0)).abs() < 1e-9);
        // A finetune is a sixteenth of the semitone, counted the same way.
        assert_eq!(note_steps(58, 15), Some(927));
        assert_eq!(note_steps(58, -16), Some(896));
        assert_eq!(note_steps(0, 0), None);
        assert_eq!(note_steps(xm::NOTE_KEY_OFF, 0), None);
        // Past the top of the table the period field would hold nothing, and a
        // period of none is not a very high note.
        assert_eq!(frames_per_second(true, 1920), None);
    }

    fn c4_rate() -> f64 {
        frames_per_second(true, 768).expect("the reference note")
    }

    #[test]
    fn the_amiga_table_is_the_same_law_on_the_hardware_clock() {
        // Both tables state the same pitch for the same note, to within the
        // percentile the hardware's own clock and the format's reference differ by.
        for steps in [0, 768, 912, 1535] {
            let linear = frames_per_second(true, steps).expect("in range");
            let amiga = frames_per_second(false, steps).expect("in range");
            let ratio = amiga / linear;
            assert!(
                (0.98..=1.0).contains(&ratio),
                "at {steps} sixteenths the tables are {ratio} apart"
            );
        }
        assert!(frames_per_second(false, 1920).is_some());
    }

    #[test]
    fn a_loop_of_its_own_frames_never_leaves_them() {
        let looping = sample((0..100).collect(), 40, 60, false);
        // The cursor lands past the loop's end and comes back inside it, and stays
        // there however far it was asked to go.
        for (position, step) in [(59.5, 1.0), (72.3, 4.0), (1_000.0, 0.5)] {
            let mut voice = super::Voice {
                sounding: true,
                position,
                step,
                ..Default::default()
            };
            super::advance(&mut voice, &looping);
            assert!(
                (voice.position - 40.0).abs() < 20.0,
                "at {position} plus {step} the cursor went to {}",
                voice.position
            );
        }
        // Across the join the neighbour is the loop's own start, which is the frame
        // the file holds there rather than the one it holds past it.
        let joined = sample(vec![0, 16_384, -16_384], 0, 2, false);
        let value = read_frame(&joined, 1.5).expect("inside the sample");
        assert!(
            (value - 0.25).abs() < 1e-6,
            "the seam interpolates to {value}"
        );
    }

    #[test]
    fn a_ping_pong_loop_turns_around_inside_its_own_range() {
        let mirrored = sample((0..100).collect(), 40, 60, true);
        let mut voice = super::Voice {
            sounding: true,
            position: 58.0,
            step: 2.5,
            ..Default::default()
        };
        super::advance(&mut voice, &mirrored);
        // Past the end by half a frame, so the pass comes back half a frame short
        // of the end rather than jumping to the start.
        assert!(
            (voice.position - 57.5).abs() < 1e-9,
            "turned to {}",
            voice.position
        );
        super::advance(&mut voice, &mirrored);
        assert!((voice.position - 55.0).abs() < 1e-9, "reverse direction must persist");
        assert!(voice.reverse);
        // And a whole cycle of it walks the loop up and back down without ever
        // leaving it, which is what the same cursor does over a long note.
        for _ in 0..1000 {
            super::advance(&mut voice, &mirrored);
            assert!(
                voice.position >= 40.0 && voice.position < 60.0,
                "left the loop at {}",
                voice.position
            );
        }
    }

    #[test]
    fn a_sample_without_a_loop_stops_when_it_runs_out() {
        let ends = sample(vec![0, 8_192, -8_192], 0, 0, false);
        // The file's last frame still plays, on its own, and the next one does not
        // exist at all.
        assert_eq!(read_frame(&ends, 2.0), Some(-0.25));
        assert_eq!(read_frame(&ends, 3.0), None);
        // Nothing to wrap, so nothing holds the cursor back.
        let mut voice = super::Voice {
            sounding: true,
            position: 2.5,
            step: 1.0,
            ..Default::default()
        };
        super::advance(&mut voice, &ends);
        assert_eq!(voice.position, 3.5);
        // Between two frames the one above is the neighbour, which is what a
        // quarter of the way through a rise is worth.
        assert_eq!(read_frame(&ends, 0.25), Some(0.0625));
    }

    #[test]
    fn the_fixture_plays_its_notes_holds_them_and_lets_go() {
        let module = xm::Module::parse(FIXTURE, &xm::Limits::default()).expect("parses");
        let mut decoder = opened(2);
        let mut sounding = Vec::new();
        for index in 0..module.packets() {
            let duration = module.packet_time(index).1;
            let frames = render(&mut decoder, &module.packet(index), duration / 1_000);
            // 120 ms rows at the tracker's own six ticks and 125 a minute.
            assert_eq!(
                frames.len(),
                2 * usize::try_from(120 * u64::from(RATE) / 1_000).expect("fits")
            );
            assert!(peak(&frames) <= 1.0, "packet {index} clips");
            sounding.push(peak(&frames) > 0.0);
        }
        // The notes, the row that holds them, the key-off - which cuts, because no
        // envelope is applied that could hold the note past it - and the silent
        // tail after the last row.
        assert_eq!(sounding, vec![true, true, true, false]);
    }

    #[test]
    fn a_held_note_is_the_sample_volume_at_its_pan() {
        // The second row names no note and changes nothing, so the only thing
        // sounding on it is the sine the second channel took: a 64-frame cycle
        // whose loudest frame is 20 000 of the 16-bit field, at the volume its own
        // cell sets and placed by the sample's own 0x40.
        let module = xm::Module::parse(FIXTURE, &xm::Limits::default()).expect("parses");
        let mut decoder = opened(2);
        render(&mut decoder, &module.packet(0), 120);
        let held = render(&mut decoder, &module.packet(1), 120);
        let left = peak(&stride(&held, 2, 0));
        let right = peak(&stride(&held, 2, 1));
        let level = 20_000.0f32 / 32_768.0 * (48.0 / 64.0) * super::AMPLIFICATION;
        assert!(
            (right - level * (64.0f32 / 256.0).sqrt()).abs() < 2e-3,
            "right is {right}, asked for {}",
            level * (64.0f32 / 256.0).sqrt()
        );
        // Equal-power placement, so the two speakers' powers add to the one level.
        assert!(
            (left / right - (192.0f32 / 64.0).sqrt()).abs() < 0.01,
            "the pair is {left} and {right}"
        );
        // The square on the other channel has run out long before this row ends:
        // 128 frames at A-4's rate is nine milliseconds of a 120 ms row.
        let mut one = opened(1);
        render(&mut one, &module.packet(0), 120);
        let dry = render(&mut one, &module.packet(1), 120);
        assert!(
            (peak(&dry) - level).abs() < 2e-3,
            "mono hears {dry_peak} where the sample alone is {level}",
            dry_peak = peak(&dry)
        );
    }

    #[test]
    fn a_key_off_cuts_because_nothing_holds_the_note() {
        let module = xm::Module::parse(FIXTURE, &xm::Limits::default()).expect("parses");
        let mut decoder = opened(2);
        for index in 0..2 {
            render(&mut decoder, &module.packet(index), 120);
        }
        let cut = render(&mut decoder, &module.packet(2), 120);
        let frames = cut.len() / 2;
        let head = &cut[..1];
        let tail_start = (frames - 100) * 2;
        // The row starts with the note still sounding and ends in silence, and the
        // let-go takes the ramp's 48 frames rather than the row's 5 292.
        assert!(peak(head) > 0.0);
        assert_eq!(peak(&cut[tail_start..]), 0.0);
        let quiet = cut
            .chunks_exact(2)
            .position(|frame| peak(frame) == 0.0)
            .expect("the note stops somewhere in the row");
        assert!(
            (30..200).contains(&quiet),
            "the note stopped {quiet} frames in"
        );
    }

    #[test]
    fn every_thing_a_row_asks_for_that_is_not_played_is_counted() {
        let mut decoder = opened(2);
        // These reserved commands remain unsupported; pitch modulation now plays.
        let cells = vec![cell(200, 0, 0x51, 0x22, 0), cell(0, 9, 0, 0, 0)];
        let silent = render(&mut decoder, &row_packet(125, 6, &cells), 120);
        assert_eq!(peak(&silent), 0.0, "nothing here can sound");
        assert_eq!(
            decoder.ignored(),
            Ignored {
                effects: 1,
                volume_columns: 1,
                envelopes: 0,
                fades: 0,
                missing_samples: 1,
                unplayable_notes: 1,
            }
        );
        // The three effects the container already acted on are not counted again,
        // and a cell that sets a level within the column's range is played.
        let mut played = opened(2);
        let cells = vec![
            cell(58, 1, 0x50, xm::EFFECT_TEMPO, 8),
            cell(0, 0, 0x10, xm::EFFECT_JUMP, 0),
        ];
        render(&mut played, &row_packet(125, 6, &cells), 120);
        assert_eq!(
            played.ignored(),
            Ignored {
                fades: 0,
                ..Ignored::default()
            }
        );
        assert!(peak(&render(&mut played, &row_packet(125, 6, &cells), 120)) > 0.0);
        // A valid but unpopulated instrument slot replaces the next selection.
        // Triggering it must not replay the previous instrument, and the cell
        // reports the missing sample once.
        let mut held = opened(2);
        render(
            &mut held,
            &row_packet(125, 6, &[cell(58, 1, 0x40, 0, 0), cell(0, 0, 0, 0, 0)]),
            120,
        );
        let retried = render(
            &mut held,
            &row_packet(125, 6, &[cell(73, 9, 0, 0, 0), cell(0, 0, 0, 0, 0)]),
            120,
        );
        assert_eq!(held.ignored().missing_samples, 1);
        assert_eq!(peak(&retried), 0.0, "an empty slot cannot replay the prior sample");
    }

    #[test]
    fn refuses_what_it_cannot_play() {
        assert!(XmDecoder::new(b"RIFF....", RATE, 2).is_err());
        assert!(XmDecoder::new(FIXTURE, 0, 2).is_err());
        let mut decoder = opened(2);
        // A packet that is not a whole number of channel cells is not a row.
        assert!(decoder.decode_encoded(&[0; 9], 0, 120).is_err());
    }

    #[test]
    fn a_reset_silences_what_was_sounding() {
        let module = xm::Module::parse(FIXTURE, &xm::Limits::default()).expect("parses");
        let mut decoder = opened(2);
        render(&mut decoder, &module.packet(0), 120);
        decoder.reset();
        let after = render(&mut decoder, &module.packet(1), 120);
        assert_eq!(peak(&after), 0.0, "a reset channel holds nothing");
    }

    /// One speaker's worth of an interleaved block.
    fn stride(samples: &[f32], channels: usize, channel: usize) -> Vec<f32> {
        samples
            .chunks_exact(channels)
            .map(|frame| frame[channel])
            .collect()
    }
}
