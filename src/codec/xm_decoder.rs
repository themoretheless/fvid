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
//! panning. It does not play its *shapes*: an instrument's two envelopes are read
//! and reported but never applied, and no cell's effect column is acted on beyond
//! the three the container already consumed while laying the timeline out
//! ([`xm::EFFECT_TEMPO`], [`xm::EFFECT_JUMP`], [`xm::EFFECT_BREAK`]). Every one of
//! those omissions is counted in [`Ignored`], which the player can be asked for,
//! so a module that needs what is missing is heard as a thin rendering rather than
//! mistaken for a correct one.
//!
//! Leaving the envelopes out is not an arbitrary cut - it is what the format does
//! to itself without them. A module with no volume envelope has nothing that holds
//! a note after its key is lifted, so a key-off *cuts* it, which is what both
//! libxm and FastTracker II do with one (`xm_key_off` in libxm: "If no volume
//! envelope is used, also cut the note"). The consequence is that an instrument's
//! `fadeout` - the ramp a released note dies along - has nothing to act on either,
//! since it only runs while a note is sustained. Both are counted as ignored at
//! the count of instruments that state them, not per note.
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
/// its top. Every value above it is one of the column's own effects, which this
/// renderer does not act on.
const VOLUME_SET_MIN: u8 = 0x10;
const VOLUME_SET_MAX: u8 = 0x50;

/// What a renderer of this profile leaves in the file unread. Counted once per
/// instrument for the shapes an instrument states, and once per cell for the
/// columns a row fills in, so the numbers say both how much of the module is
/// unaffected and how often the player was asked for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ignored {
    /// Cells whose effect column names something other than the three the
    /// container already acted on.
    pub effects: u32,
    /// Cells whose volume column asks for a value outside the direct-set range.
    pub volume_columns: u32,
    /// Instruments drawing a volume or panning envelope.
    pub envelopes: u32,
    /// Instruments stating a fadeout, which nothing can sustain long enough to use
    /// while no envelope is applied.
    pub fades: u32,
    /// Notes on a channel whose instrument names no sample for them.
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
    let period = if linear {
        f64::from(7680 - i32::from(steps) * 4)
    } else {
        (32.0 * AMIGA_C4_PERIOD * f64::exp2(f64::from(steps) / -192.0)).round()
    };
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

/// What one channel is doing. A module states its channels once, so there is one
/// of these per channel and a cell with nothing in it leaves it as it was.
#[derive(Clone, Copy, Default)]
struct Voice {
    /// Instrument the channel is playing on, as an index, which sticks across the
    /// cells that name none.
    instrument: usize,
    /// And which of that instrument's samples its current note is reading.
    sample: usize,
    sounding: bool,
    /// Whether a key is down on this channel: a cell's note puts it down, a key-off
    /// lifts it, and what the volume byte asks for only holds while it is down.
    held: bool,
    /// Where in the sample the channel is, and how far it moves per frame, both in
    /// the sample's own frames.
    position: f64,
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
                instrument.volume_envelope.on() || instrument.panning_envelope.on()
            })
            .count();
        let faded = module
            .instruments
            .iter()
            .filter(|instrument| instrument.fadeout != 0)
            .count();
        let ignored = Ignored {
            envelopes: u32::try_from(shaped).unwrap_or(u32::MAX),
            fades: u32::try_from(faded).unwrap_or(u32::MAX),
            ..Ignored::default()
        };
        let voices = vec![Voice::default(); module.channels];
        Ok(Self {
            sample_rate,
            output_channels: usize::from(channels.clamp(1, 8)),
            voices,
            module,
            ignored,
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
            ..
        } = self;
        for (voice, cell) in voices.iter_mut().zip(&row.cells) {
            apply_cell(module, voice, cell, ignored, linear, *sample_rate);
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
            if voice.level == 0.0 {
                // A cut note has finished letting go, and a note that was never
                // raised has nothing to say. Either way the sample stops being
                // read, which is what makes room for the next one.
                voice.sounding = false;
                continue;
            }
            let Some(sample) = module
                .instruments
                .get(voice.instrument)
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
            let pan = f64::from(voice.panning) / PAN_SPAN;
            left += sample * (1.0 - pan).sqrt() as f32;
            right += sample * pan.sqrt() as f32;
        }
        (left, right, dry)
    }
}

/// One cell, applied to the channel it belongs to.
///
/// The order is the tracker's own, and it is the order that gives the answer: a
/// note is triggered first, so it plays on the instrument the same cell names; the
/// instrument's sample then sets the channel's volume and placement; and the volume
/// column, if it holds a level rather than an effect, has the last word over both.
fn apply_cell(
    module: &xm::Module,
    voice: &mut Voice,
    cell: &Cell,
    ignored: &mut Ignored,
    linear: bool,
    sample_rate: u32,
) {
    if cell.instrument != 0 {
        if usize::from(cell.instrument) <= module.instruments.len() {
            voice.instrument = usize::from(cell.instrument) - 1;
        } else {
            ignored.missing_samples += 1;
        }
    }
    match cell.note {
        0 => {}
        xm::NOTE_KEY_OFF => voice.held = false,
        xm::NOTE_MIN..=xm::NOTE_MAX => {
            trigger(module, voice, cell.note, ignored, linear, sample_rate)
        }
        // Above the table's own top there is no pitch to ask for, and a note that
        // cannot be named is not the same failure as an instrument that holds
        // nothing for it.
        _ => ignored.unplayable_notes += 1,
    }
    // The instrument's own sample says how loud and where, and does so on a change
    // of instrument whether or not a note came with it - which is why a cell naming
    // an instrument alone is the way a module retunes a held note.
    if cell.instrument != 0
        && let Some(sample) = current_sample(module, voice)
    {
        voice.volume = sample.volume.min(MAX_VOLUME);
        voice.panning = sample.panning;
    }
    match cell.volume {
        0 => {}
        VOLUME_SET_MIN..=VOLUME_SET_MAX => voice.volume = cell.volume - VOLUME_SET_MIN,
        _ => ignored.volume_columns += 1,
    }
    if cell.effect != 0
        && !matches!(
            cell.effect,
            xm::EFFECT_TEMPO | xm::EFFECT_JUMP | xm::EFFECT_BREAK
        )
    {
        ignored.effects += 1;
    }
    voice.target = if voice.held {
        f32::from(voice.volume) / f32::from(MAX_VOLUME) * AMPLIFICATION
    } else {
        0.0
    };
}

fn current_sample<'a>(module: &'a xm::Module, voice: &Voice) -> Option<&'a Sample> {
    module
        .instruments
        .get(voice.instrument)?
        .samples
        .get(voice.sample)
}

/// Start the note a cell names on its channel's instrument.
fn trigger(
    module: &xm::Module,
    voice: &mut Voice,
    note: u8,
    ignored: &mut Ignored,
    linear: bool,
    sample_rate: u32,
) {
    let Some(slot) = module.sample_slot(voice.instrument, note) else {
        ignored.missing_samples += 1;
        silence(voice);
        return;
    };
    let Some(sample) = module.instruments[voice.instrument].samples.get(slot) else {
        ignored.missing_samples += 1;
        return;
    };
    // A sample's relative note is the note it was recorded at, stated as an offset
    // from the one the cell names, so the pitch the table is asked for is the sum.
    let total = i16::from(note) + i16::from(sample.relative_note);
    let steps = u8::try_from(total)
        .ok()
        .and_then(|total| note_steps(total, sample.finetune));
    let Some(steps) = steps else {
        ignored.unplayable_notes += 1;
        silence(voice);
        return;
    };
    let Some(rate) = frames_per_second(linear, steps) else {
        ignored.unplayable_notes += 1;
        return;
    };
    voice.sample = slot;
    voice.position = 0.0;
    voice.step = rate / f64::from(sample_rate);
    voice.sounding = true;
    voice.held = true;
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
    voice.position += voice.step;
    if !sample.loops() || voice.position < sample.loop_end as f64 {
        return;
    }
    let start = sample.loop_start as f64;
    let length = (sample.loop_end - sample.loop_start) as f64;
    if sample.ping_pong && length > 1.0 {
        // Forward over the loop's frames, then back over them again, with neither
        // end played twice: the cycle is two passes minus the two turning frames.
        let period = 2.0 * length - 2.0;
        let phase = (voice.position - start).rem_euclid(period);
        voice.position = start + phase.min(period - phase);
    } else {
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
        self.apply_row(&row);
        let sample_rate = self.sample_rate;
        let channels = self.output_channels;
        let frames = usize::try_from(duration * u64::from(sample_rate) / 1_000)
            .map_err(|_| invalid("module row is longer than any block this renderer writes"))?;
        let mut output = vec![0.0f32; frames * channels];
        for frame in 0..frames {
            let (left, right, dry) = self.mix_frame();
            let frame = frame * channels;
            if channels == 1 {
                output[frame] = dry.clamp(-1.0, 1.0);
                continue;
            }
            output[frame] = left.clamp(-1.0, 1.0);
            output[frame + 1] = right.clamp(-1.0, 1.0);
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
            pts,
            timebase_num: 1,
            timebase_den: sample_rate,
        }))
    }

    fn reset(&mut self) {
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
        assert_eq!((audio.timebase_num, audio.timebase_den), (1, RATE));
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
        let cells = vec![cell(200, 0, 0xF0, 1, 0), cell(0, 9, 0, 0, 0)];
        let silent = render(&mut decoder, &row_packet(125, 6, &cells), 120);
        assert_eq!(peak(&silent), 0.0, "nothing here can sound");
        assert_eq!(
            decoder.ignored(),
            Ignored {
                effects: 1,
                volume_columns: 1,
                envelopes: 0,
                fades: 1,
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
                fades: 1,
                ..Ignored::default()
            }
        );
        assert!(peak(&render(&mut played, &row_packet(125, 6, &cells), 120)) > 0.0);
        // A cell naming an instrument the module does not have still plays the note
        // it carries, on the instrument the channel already held. That is what the
        // trackers do, so the count above is a count of what was missing rather
        // than of what went silent.
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
        assert!(
            peak(&retried) > 0.0,
            "the note played on the instrument the channel held"
        );
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
