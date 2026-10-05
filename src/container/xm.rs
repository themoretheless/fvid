//! FastTracker II module parsing for the player's own audio path.
//!
//! An `.xm` file carries no encoded audio either: it carries instruments -
//! linear PCM in the file itself - and a grid of rows saying which of them to
//! play when. So what this container hands a decoder is the row grid already
//! placed on a wall clock, with the sample data beside it. The placement belongs
//! here rather than in the mixer because only the file states the order the
//! patterns play in, the ticks a row lasts and the tempo both are measured
//! against, and a row that changes the tempo changes the length of every row
//! after it - the same argument that puts MIDI's tempo map in
//! [`crate::container::smf`].
//!
//! The slice form is what keeps the container inside the audio pipeline the
//! other readers use: a packet is one row of the module's own grid, timestamped
//! in milliseconds and lasting exactly the time its tempo gives it, followed by
//! one empty packet after the last row so a sample still sounding at the end is
//! heard letting go rather than cut off. One row per packet rather than the fixed
//! slices a performance allows: a row is the shortest thing a module states, and
//! its length is set by the tempo carried in that row itself, so splitting one
//! across packets would leave the mixer to guess which fraction of a tick it is
//! looking at.
//!
//! # What the file states, and in which unit
//!
//! Every number here is read as the format's own specs state it, cross-checked
//! against how libxm and OpenMPT read the same field. Where the two disagree the
//! comments name both readings rather than settling it silently:
//!
//! - Sample `length`, `loop start` and `loop length` are counted in **bytes**, so
//!   a 16-bit sample holds half as many frames as its header states, and so does
//!   its loop. The written specs imply frames; both implementations read bytes,
//!   and a file either of them wrote is only legible that way.
//! - 8-bit sample data is a byte-wise signed delta; 16-bit data is a word-wise
//!   signed delta over little-endian words. The nibble packing that turns up in
//!   the same field is ModPlug's 4-bit ADPCM, which is refused.
//! - A row lasts `speed` ticks at `2 500 000 / bpm` microseconds apiece, which is
//!   the same statement as `speed * 2.5 / bpm` seconds.
//! - Effects are stored as the letter the tracker shows, minus `A`, plus one, so
//!   `0` in the file states no effect and `16` is `P`, the tempo.

use crate::{Result, invalid};
use std::collections::HashSet;
use std::time::Duration;

/// Ticks per second the packet timestamps use, which is the millisecond.
pub const PACKET_TIMESCALE: u32 = 1_000;
/// Bytes starting one row of a packet: the row's length in microseconds, the
/// tempo it is played under, the ticks that divide it, and a byte held at zero so
/// the channel cells start on a round offset.
pub const ROW_HEADER_BYTES: usize = 8;
/// Bytes per channel cell of a row: note, instrument, volume column, and the two
/// bytes of the effect.
pub const CELL_BYTES: usize = 5;
/// The most channels the format states, and the most this reader takes.
pub const MAX_CHANNELS: usize = 32;
/// The note byte that tells a held sample to let go, one above the table's top.
pub const NOTE_KEY_OFF: u8 = 97;
/// The lowest and highest note a cell can name, in the file's own numbering. A
/// cell with nothing in it names no note and leaves the pitch to whatever is
/// already sounding.
pub const NOTE_MIN: u8 = 1;
pub const NOTE_MAX: u8 = 96;
/// What a module plays at when its header states nothing usable: FastTracker II's
/// own defaults, six ticks a row at 125 a minute.
pub const DEFAULT_SPEED: u8 = 6;
pub const DEFAULT_BPM: u16 = 125;
/// The tempo range a row may state, and the tick count a row may be stretched
/// over. Outside either, a module's timing stops being sound rather than silence.
pub const BPM_MIN: u16 = 32;
pub const BPM_MAX: u16 = 255;
pub const SPEED_MIN: u8 = 1;
pub const SPEED_MAX: u8 = 31;
/// The effect byte that states tempo: the letter `P`. Its parameter is a tick
/// count up to 31 and a tempo above that, which is the one column of a module
/// that changes the length of every row after it.
pub const EFFECT_TEMPO: u8 = 16;
/// The effect byte that ends the pattern and names the order to play next: `L`.
pub const EFFECT_JUMP: u8 = 12;
/// The effect byte that ends the pattern and names the row of the next order to
/// resume at: `N`.
pub const EFFECT_BREAK: u8 = 14;
/// A sample type byte that is not FastTracker II's own: the value ModPlug writes
/// where the reserved byte says `AD`, which is 4-bit ADPCM rather than
/// delta-coded PCM.
const SAMPLE_TYPE_ADPCM: u8 = 0xAD;
const SAMPLE_FLAG_16_BIT: u8 = 0x10;
const SAMPLE_FLAG_STEREO: u8 = 0x20;
const LOOP_MODE_NONE: u8 = 0;
const LOOP_MODE_PING_PONG: u8 = 2;
/// Which of an envelope's three flag bits say what: the shape is in use, held at
/// its sustain point, and cycling its loop.
const ENVELOPE_FLAG_ON: u8 = 1;
const ENVELOPE_FLAG_SUSTAIN: u8 = 2;
const ENVELOPE_FLAG_LOOP: u8 = 4;
/// Twelve points of a frame number and a value, which is all the shape of these
/// arrays is, and where they sit in the instrument header.
const ENVELOPE_POINTS: usize = 12;
const ENVELOPE_BYTES: usize = ENVELOPE_POINTS * 4;
/// Bytes in a sample record, and the shortest instrument header that states its
/// own name, its sample count and both envelopes.
const SAMPLE_RECORD_BYTES: usize = 40;
/// The instrument fields this reader names run through the fadeout, which ends
/// at 241; the tracker itself states two more bytes of reserved data after it.
const INSTRUMENT_HEADER_BYTES: usize = 241;
const MAGIC: &[u8] = b"Extended Module: ";
const OFFSET_NAME: usize = 17;
const OFFSET_TERMINATOR: usize = 37;
const OFFSET_TRACKER: usize = 38;
const OFFSET_VERSION: usize = 58;
const OFFSET_HEADER_SIZE: usize = 60;
const OFFSET_SONG_LENGTH: usize = 64;
const OFFSET_RESTART: usize = 66;
const OFFSET_CHANNELS: usize = 68;
const OFFSET_PATTERNS: usize = 70;
const OFFSET_INSTRUMENTS: usize = 72;
const OFFSET_FLAGS: usize = 74;
const OFFSET_SPEED: usize = 76;
const OFFSET_BPM: usize = 78;
const OFFSET_ORDERS: usize = 80;
/// The order table is 256 bytes wide whatever the song length says, so a header
/// stating less than this holds no table.
const ORDER_TABLE_BYTES: usize = 256;
const MIN_HEADER_SIZE: u32 = (OFFSET_ORDERS - OFFSET_HEADER_SIZE + ORDER_TABLE_BYTES) as u32;
/// Where the instrument header's own fields sit, from its start.
const INSTRUMENT_NAME: usize = 4;
const INSTRUMENT_SAMPLE_COUNT: usize = 27;
const INSTRUMENT_KEYMAP: usize = 33;
const INSTRUMENT_VOLUME_ENVELOPE: usize = 129;
const INSTRUMENT_FLAGS: usize = 233;
const INSTRUMENT_FADEOUT: usize = 239;
/// And the sample record's, from its start.
const SAMPLE_LENGTH: usize = 0;
const SAMPLE_LOOP_START: usize = 4;
const SAMPLE_LOOP_LENGTH: usize = 8;
const SAMPLE_VOLUME: usize = 12;
const SAMPLE_FINETUNE: usize = 13;
const SAMPLE_TYPE: usize = 14;
const SAMPLE_PANNING: usize = 15;
const SAMPLE_RELATIVE_NOTE: usize = 16;
const SAMPLE_RESERVED: usize = 17;
const SAMPLE_NAME: usize = 18;

/// Bounds a file has to stay inside to be read at all. A module is small data -
/// the sample bytes dominate it, and those are the ones a hostile header can ask
/// for without describing a file that holds them - so the caps sit above what
/// FastTracker II itself writes and turn a lying header into a refusal rather
/// than an allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest file the reader takes in.
    pub file_bytes: usize,
    /// Largest number of channels sounding together.
    pub channels: usize,
    /// Largest number of patterns, and of instruments, the header may name.
    pub patterns: usize,
    pub instruments: usize,
    /// Largest number of samples across every instrument.
    pub samples: usize,
    /// Largest number of decoded sample frames across every sample.
    pub sample_frames: usize,
    /// Longest timeline, in rows, the reader will lay out.
    pub rows: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 64 << 20,
            channels: MAX_CHANNELS,
            patterns: 256,
            instruments: 128,
            samples: 256,
            sample_frames: 16 << 20,
            rows: 20_000,
        }
    }
}

/// One cell of a pattern row: what a channel is told to do. The volume column is
/// kept in the byte the file states, since which of its values set a volume and
/// which start an effect is a mixing question rather than a reading one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub note: u8,
    pub instrument: u8,
    pub volume: u8,
    pub effect: u8,
    pub parameter: u8,
}

/// A pattern as the file holds it: rows of cells, one per channel, stored row by
/// row so a mixer reads a row without counting cells.
#[derive(Clone, Debug)]
pub struct Pattern {
    cells: Vec<Cell>,
    rows: usize,
    channels: usize,
}

impl Pattern {
    /// The cells of one row, in channel order.
    pub fn row(&self, index: usize) -> &[Cell] {
        &self.cells[index * self.channels..(index + 1) * self.channels]
    }

    pub fn row_count(&self) -> usize {
        self.rows
    }

    pub fn channels(&self) -> usize {
        self.channels
    }
}

/// One sample of an instrument, with its PCM already undone from the delta form
/// the file stores it in, so a mixer reads frames and nothing else.
#[derive(Clone, Debug)]
pub struct Sample {
    pub name: String,
    /// Signed 16-bit frames, whatever width the file stored them in.
    pub frames: Vec<i16>,
    /// First frame of the loop, and one past its last. A sample whose header
    /// puts the loop beyond its own end has no loop.
    pub loop_start: usize,
    pub loop_end: usize,
    /// Whether the loop runs backwards on every second pass.
    pub ping_pong: bool,
    /// 0..=64 in the file's own units.
    pub volume: u8,
    /// Sixteenths of a semitone, signed, added to the pitch the note names.
    pub finetune: i8,
    /// 0..=255, where the tracker's own centre is 0xA4 and 0xFF asks for the
    /// surround panning of the Amiga rather than a place between the speakers.
    pub panning: u8,
    /// The note this sample plays back at the rate it was recorded at, as an
    /// offset in notes from the one a cell names.
    pub relative_note: i8,
}

impl Sample {
    pub fn loops(&self) -> bool {
        self.loop_end > self.loop_start
    }
}

/// An envelope an instrument states: its points, and what the tracker does with
/// them. A shape of nothing is what a module written by FastTracker II itself
/// holds when its author never drew one, and `on` says exactly that.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Envelope {
    /// Frame number and value of each point, in the file's own order. The frame
    /// is counted in ticks, which is why the mixer rather than this reader turns
    /// it into time.
    pub points: Vec<(u16, u16)>,
    pub flags: u8,
}

impl Envelope {
    /// Whether the instrument asks to be shaped by this envelope at all.
    pub fn on(&self) -> bool {
        self.flags & ENVELOPE_FLAG_ON != 0
    }

    pub fn sustain(&self) -> bool {
        self.flags & ENVELOPE_FLAG_SUSTAIN != 0
    }

    pub fn looped(&self) -> bool {
        self.flags & ENVELOPE_FLAG_LOOP != 0
    }
}

/// An instrument: the samples it holds, the map from a note to one of them, and
/// the two envelopes that shape them.
#[derive(Clone, Debug)]
pub struct Instrument {
    pub name: String,
    pub samples: Vec<Sample>,
    /// 96 bytes, one per note the tracker shows, holding a one-based index into
    /// `samples` or 0 for none.
    pub keymap: Vec<u8>,
    pub volume_envelope: Envelope,
    pub panning_envelope: Envelope,
    /// The instrument's own fade, in the file's units, applied once a note is
    /// released.
    pub fadeout: u16,
}

/// A row of the timeline as played: where in the module it comes from, how long
/// it lasts, and under what tempo. The cells themselves stay in the pattern,
/// which a module that plays the same pattern twice reads twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimelineRow {
    pub order: usize,
    pub row: usize,
    /// Where the row starts on the timeline, and how long it lasts. A row that
    /// changes the tempo states the tempo its own length is measured by, and the
    /// rows after it inherit it, so both numbers belong to the row.
    pub start_microseconds: u64,
    pub duration_microseconds: u64,
    pub speed: u8,
    pub bpm: u16,
}

/// A parsed module: what it says about its own shape, its samples as frames, and
/// the row timeline the two together play as.
#[derive(Clone, Debug)]
pub struct Module {
    pub name: String,
    pub tracker: String,
    pub version: u16,
    /// Bit 0 of the header's flags: the module asks for the linear frequency
    /// table rather than the Amiga one.
    pub linear: bool,
    pub channels: usize,
    pub instruments: Vec<Instrument>,
    pub patterns: Vec<Pattern>,
    pub orders: Vec<u8>,
    pub restart: u16,
    /// The tempo the header starts at, which is what the first row is played
    /// under unless a row says otherwise.
    pub speed: u8,
    pub bpm: u16,
    timeline: Vec<TimelineRow>,
    last_microsecond: u64,
}

impl Module {
    /// Read a file, refusing anything the bounds above or the format itself rules
    /// out.
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Self> {
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "module is {} bytes, over the {} byte limit",
                bytes.len(),
                limits.file_bytes
            )));
        }
        if bytes.len() < OFFSET_ORDERS || &bytes[0..MAGIC.len()] != MAGIC {
            return Err(invalid(
                "not a FastTracker II module: no `Extended Module:` tag",
            ));
        }
        if bytes[OFFSET_TERMINATOR] != 0x1A {
            return Err(invalid("module name does not end where the format puts it"));
        }
        let version = read_le_u16(bytes, OFFSET_VERSION)?;
        // 1.02 is the earliest version whose header states a length of its own,
        // and 1.04 the last one FastTracker II wrote; anything else is a file
        // this reader has no layout for.
        if !(0x0102..=0x0104).contains(&version) {
            return Err(invalid(&format!(
                "module version {version:#06x} is not one of the three the format states"
            )));
        }
        let header_size = read_le_u32(bytes, OFFSET_HEADER_SIZE)?;
        if header_size < MIN_HEADER_SIZE {
            return Err(invalid(&format!(
                "module header states {header_size} bytes, less than the {MIN_HEADER_SIZE} its \
                 fields take"
            )));
        }
        // The size counts from its own field, so it states where the first record
        // starts as well as what it holds.
        let after_header = OFFSET_HEADER_SIZE
            .checked_add(usize::try_from(header_size).unwrap_or(usize::MAX))
            .ok_or_else(|| invalid("module header size overflows the file"))?;
        if after_header > bytes.len() {
            return Err(invalid("module header runs past the end of the file"));
        }
        let song_length = usize::from(read_le_u16(bytes, OFFSET_SONG_LENGTH)?);
        if song_length > ORDER_TABLE_BYTES {
            return Err(invalid("module states more orders than its table holds"));
        }
        let restart = read_le_u16(bytes, OFFSET_RESTART)?;
        let channels = usize::from(read_le_u16(bytes, OFFSET_CHANNELS)?);
        if channels == 0 || channels > limits.channels {
            return Err(invalid(&format!(
                "module names {channels} channels, outside the 1..={} this reader takes",
                limits.channels
            )));
        }
        let pattern_count = usize::from(read_le_u16(bytes, OFFSET_PATTERNS)?);
        let instrument_count = usize::from(read_le_u16(bytes, OFFSET_INSTRUMENTS)?);
        if pattern_count > limits.patterns || instrument_count > limits.instruments {
            return Err(invalid(&format!(
                "module names {pattern_count} patterns and {instrument_count} instruments, over \
                 the {} and {} this reader takes",
                limits.patterns, limits.instruments
            )));
        }
        let flags = read_le_u16(bytes, OFFSET_FLAGS)?;
        // A header that states no usable tempo plays at the tracker's own
        // defaults, which is what a module whose author never touched either box
        // holds and what every player of the format assumes.
        let speed = clamp_speed(read_le_u16(bytes, OFFSET_SPEED)?).unwrap_or(DEFAULT_SPEED);
        let bpm = clamp_bpm(read_le_u16(bytes, OFFSET_BPM)?).unwrap_or(DEFAULT_BPM);
        let orders = bytes[OFFSET_ORDERS..OFFSET_ORDERS + song_length].to_vec();
        // 1.04 stores the patterns before the instruments; the two versions
        // before it store them after. Nothing else about the two record families
        // differs, so only the walk changes.
        let patterns_first = version >= 0x0104;
        let mut at = after_header;
        let patterns;
        let instruments;
        if patterns_first {
            patterns = parse_patterns(bytes, &mut at, pattern_count, channels, version)?;
            instruments = parse_instruments(bytes, &mut at, instrument_count, limits)?;
        } else {
            instruments = parse_instruments(bytes, &mut at, instrument_count, limits)?;
            patterns = parse_patterns(bytes, &mut at, pattern_count, channels, version)?;
        }
        for order in &orders {
            if usize::from(*order) >= patterns.len() {
                return Err(invalid(&format!(
                    "module plays order {order}, which names pattern {} of {}",
                    usize::from(*order),
                    patterns.len()
                )));
            }
        }
        let (timeline, last_microsecond) =
            lay_out_timeline(&orders, &patterns, speed, bpm, limits)?;
        Ok(Self {
            name: read_name(bytes, OFFSET_NAME, 20),
            tracker: read_name(bytes, OFFSET_TRACKER, 20),
            version,
            linear: flags & 1 != 0,
            channels,
            instruments,
            patterns,
            orders,
            restart,
            speed,
            bpm,
            timeline,
            last_microsecond,
        })
    }

    /// The module's own length: the end of the last row it plays.
    pub fn duration(&self) -> Duration {
        Duration::from_micros(self.last_microsecond)
    }

    /// Packets to hand out: one per row of the timeline, plus the trailing empty
    /// one that lets a sample still sounding at the end be heard letting go.
    pub fn packets(&self) -> usize {
        self.timeline.len() + 1
    }

    /// Where a packet starts on the timeline and how long it lasts, in
    /// microseconds. The tail takes the length of the last row it follows, which
    /// is all the module states about how long a held note is allowed to ring.
    pub fn packet_time(&self, index: usize) -> (u64, u64) {
        let Some(row) = self.timeline.get(index) else {
            return (
                self.last_microsecond,
                self.timeline
                    .last()
                    .map_or(0, |row| row.duration_microseconds),
            );
        };
        (row.start_microseconds, row.duration_microseconds)
    }

    /// The packet a point on the timeline lands in: the last row that had begun
    /// by then, or the tail when the time is past the module's own end.
    pub fn packet_at_microsecond(&self, microseconds: u64) -> usize {
        if microseconds >= self.last_microsecond {
            return self.timeline.len();
        }
        self.timeline
            .partition_point(|row| row.start_microseconds <= microseconds)
            .saturating_sub(1)
    }

    /// The packet for one row: the tempo it is played under and the cells every
    /// channel is told to act on. The tail packet holds no row, so it states the
    /// length it was given with nothing for any channel to do.
    pub fn packet(&self, index: usize) -> Vec<u8> {
        let mut data = Vec::new();
        match self.timeline.get(index) {
            Some(row) => write_row(self, row, &mut data),
            None => {
                let tail = self.packet_time(index).1;
                let length = u32::try_from(tail).unwrap_or(u32::MAX);
                data.extend_from_slice(&length.to_le_bytes());
                data.extend_from_slice(&self.bpm.to_le_bytes());
                data.push(self.speed);
                data.push(0);
                data.resize(ROW_HEADER_BYTES + self.channels * CELL_BYTES, 0);
            }
        }
        data
    }

    /// Every row the module plays, in the order it plays them.
    pub fn timeline(&self) -> &[TimelineRow] {
        &self.timeline
    }

    /// The cells of one timeline row.
    pub fn row_cells(&self, row: &TimelineRow) -> &[Cell] {
        self.patterns[usize::from(self.orders[row.order])].row(row.row)
    }

    /// Which sample of an instrument a note selects, as an index into its
    /// `samples`, or `None` when the instrument names none for it. The keymap's
    /// byte is one-based in the file, and a mixer that wants the sample beside the
    /// slot it came from needs the zero-based index more than the reference.
    pub fn sample_slot(&self, instrument: usize, note: u8) -> Option<usize> {
        let instrument = self.instruments.get(instrument)?;
        if !(NOTE_MIN..=NOTE_MAX).contains(&note) {
            return None;
        }
        let slot = *instrument.keymap.get(usize::from(note - NOTE_MIN))?;
        let slot = usize::from(slot).checked_sub(1)?;
        instrument.samples.get(slot)?;
        Some(slot)
    }

    /// The sample an instrument's keymap selects for a note, or `None` when the
    /// instrument names none for it.
    pub fn sample_for(&self, instrument: usize, note: u8) -> Option<&Sample> {
        let slot = self.sample_slot(instrument, note)?;
        self.instruments.get(instrument)?.samples.get(slot)
    }

    /// The instrument a cell names, in the file's own one-based numbering, or
    /// `None` for the cell that names none.
    pub fn instrument(&self, number: u8) -> Option<&Instrument> {
        if number == 0 {
            return None;
        }
        self.instruments.get(usize::from(number) - 1)
    }
}

/// One row of a packet: how long it lasts, the tempo that says so, and its
/// cells, in channel order.
fn write_row(module: &Module, row: &TimelineRow, data: &mut Vec<u8>) {
    data.extend_from_slice(
        &(u32::try_from(row.duration_microseconds).unwrap_or(u32::MAX)).to_le_bytes(),
    );
    data.extend_from_slice(&row.bpm.to_le_bytes());
    data.push(row.speed);
    data.push(0);
    for cell in module.row_cells(row) {
        data.extend_from_slice(&[
            cell.note,
            cell.instrument,
            cell.volume,
            cell.effect,
            cell.parameter,
        ]);
    }
}

/// A row as its packet states it, which is what a decoder reads back: the tempo
/// to mix under, the ticks that divide the row, and what each channel is told to
/// do. The tail packet holds the same shape with nothing in any channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub duration_microseconds: u32,
    pub bpm: u16,
    pub speed: u8,
    pub cells: Vec<Cell>,
}

/// The row a packet holds.
pub fn read_row(bytes: &[u8]) -> Result<Row> {
    if bytes.len() < ROW_HEADER_BYTES || !(bytes.len() - ROW_HEADER_BYTES).is_multiple_of(CELL_BYTES) {
        return Err(invalid(
            "module row packet holds neither a whole row nor a whole number of cells",
        ));
    }
    Ok(Row {
        duration_microseconds: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        bpm: u16::from_le_bytes([bytes[4], bytes[5]]),
        speed: bytes[6],
        cells: bytes[ROW_HEADER_BYTES..]
            .as_chunks::<CELL_BYTES>().0.iter()
            .map(|cell| Cell {
                note: cell[0],
                instrument: cell[1],
                volume: cell[2],
                effect: cell[3],
                parameter: cell[4],
            })
            .collect(),
    })
}

/// Microseconds one tick lasts at a tempo: the tracker divides a beat into 24
/// ticks, which is the same number as `2 500 000 / bpm`.
pub fn tick_microseconds(bpm: u16) -> u64 {
    2_500_000 / u64::from(bpm.max(1))
}

/// A tempo the format accepts, or `None` for one it does not. A tempo of 0 would
/// make every row last forever, and one above the range is a number the tracker
/// itself refuses to type.
fn clamp_bpm(bpm: u16) -> Option<u16> {
    (BPM_MIN..=BPM_MAX).contains(&bpm).then_some(bpm)
}

/// Ticks a row holds, which is the speed column's own range; above it a row is
/// longer than the tracker's display states, and zero is no length at all.
fn clamp_speed(speed: u16) -> Option<u8> {
    u8::try_from(speed)
        .ok()
        .filter(|speed| (SPEED_MIN..=SPEED_MAX).contains(speed))
}

/// Lay the rows out on a wall clock, following the order table and whatever the
/// rows themselves say about where to go next.
fn lay_out_timeline(
    orders: &[u8],
    patterns: &[Pattern],
    speed: u8,
    bpm: u16,
    limits: &Limits,
) -> Result<(Vec<TimelineRow>, u64)> {
    let mut timeline: Vec<TimelineRow> = Vec::new();
    let mut entered: HashSet<usize> = HashSet::new();
    let mut order = 0usize;
    let mut row = 0usize;
    let mut speed = speed;
    let mut bpm = bpm;
    let mut microseconds = 0u64;
    while order < orders.len() {
        if timeline.len() > limits.rows {
            return Err(invalid(&format!(
                "module does not end within the {} rows this reader takes",
                limits.rows
            )));
        }
        let pattern = &patterns[usize::from(orders[order])];
        if row >= pattern.row_count() {
            order += 1;
            row = 0;
            continue;
        }
        // An order entered from its first row twice is the module looping
        // itself - which trackers do, and a timeline cannot be measured against
        // - so the walk stops where the loop would begin, and the module is the
        // one pass of what it played.
        if row == 0 && !entered.insert(order) {
            break;
        }
        // The tempo a row states is the tempo that row is played under: the
        // tracker applies a tempo change from the tick it is read at, so the row
        // carrying it is the first thing the new number measures.
        let mut next: Option<(usize, usize)> = None;
        for cell in pattern.row(row) {
            match cell.effect {
                EFFECT_TEMPO if cell.parameter != 0 => {
                    if cell.parameter <= SPEED_MAX {
                        speed = clamp_speed(u16::from(cell.parameter)).unwrap_or(speed);
                    } else if let Some(tempo) = clamp_bpm(u16::from(cell.parameter)) {
                        bpm = tempo;
                    }
                }
                EFFECT_JUMP => next = Some((usize::from(cell.parameter), 0)),
                EFFECT_BREAK => {
                    // The parameter states the row the way the tracker shows it:
                    // in decimal, so `24` is the row the tracker calls 24.
                    let resume =
                        usize::from(cell.parameter >> 4) * 10 + usize::from(cell.parameter & 0xF);
                    next = Some((order + 1, resume));
                }
                _ => {}
            }
        }
        let duration = u64::from(speed) * tick_microseconds(bpm);
        timeline.push(TimelineRow {
            order,
            row,
            start_microseconds: microseconds,
            duration_microseconds: duration,
            speed,
            bpm,
        });
        microseconds += duration;
        row += 1;
        if let Some((next_order, resume)) = next {
            order = next_order;
            row = resume;
        }
    }
    Ok((timeline, microseconds))
}

fn parse_patterns(
    bytes: &[u8],
    at: &mut usize,
    count: usize,
    channels: usize,
    version: u16,
) -> Result<Vec<Pattern>> {
    let mut patterns = Vec::with_capacity(count);
    for index in 0..count {
        let header = *at;
        let header_size = read_le_u32(bytes, header)?;
        // Nine is the whole header the format states; less holds no row count,
        // and more is a header with fields this reader has no name for, which it
        // steps over rather than guesses at.
        let header_size = usize::try_from(header_size)
            .ok()
            .filter(|size| *size >= 9)
            .ok_or_else(|| {
                invalid(&format!(
                    "pattern {index} states a header of {header_size} bytes, which holds no row count"
                ))
            })?;
        if header + header_size > bytes.len() {
            return Err(invalid(&format!(
                "pattern {index}'s header runs past the end of the file"
            )));
        }
        if bytes[header + 4] != 0 {
            return Err(invalid(&format!(
                "pattern {index} is packed in a form the format does not define"
            )));
        }
        // 1.02 holds the row count in one byte counting from zero, which moves the
        // packed size up by one as well; every version after it holds the count
        // itself in two bytes.
        let early = version == 0x0102;
        let rows = if early {
            usize::from(bytes[header + 5]) + 1
        } else {
            usize::from(read_le_u16(bytes, header + 5)?)
        };
        if rows > 256 {
            return Err(invalid(&format!(
                "pattern {index} has {rows} rows, over the 256 the format holds"
            )));
        }
        let packed_size = usize::from(read_le_u16(bytes, header + if early { 6 } else { 7 })?);
        let data = header + header_size;
        let end = data
            .checked_add(packed_size)
            .ok_or_else(|| invalid("pattern data size overflows the file"))?;
        if end > bytes.len() {
            return Err(invalid(&format!(
                "pattern {index} runs past the end of the file"
            )));
        }
        let mut cells = vec![Cell::default(); rows * channels];
        // A pattern stating no packed data holds empty rows, which is how the
        // tracker writes one it never typed into.
        if packed_size != 0 {
            let mut cursor = data;
            for cell in &mut cells {
                let (read, next) = read_cell(bytes, cursor)?;
                *cell = read;
                cursor = next;
            }
            if cursor > end {
                return Err(invalid(&format!(
                    "pattern {index}'s rows run past the data its header states"
                )));
            }
        }
        *at = end;
        patterns.push(Pattern {
            cells,
            rows,
            channels,
        });
    }
    Ok(patterns)
}

/// One packed cell, and the offset after it. The flag byte says which fields
/// follow; without its top bit the five are all there, in order, and the byte
/// itself is the note.
fn read_cell(bytes: &[u8], at: usize) -> Result<(Cell, usize)> {
    let flag = *bytes
        .get(at)
        .ok_or_else(|| invalid("module's pattern data ends inside a cell"))?;
    if flag & 0x80 == 0 {
        let five = bytes
            .get(at..at + CELL_BYTES)
            .ok_or_else(|| invalid("module's pattern data ends inside a cell"))?;
        return Ok((
            Cell {
                note: five[0],
                instrument: five[1],
                volume: five[2],
                effect: five[3],
                parameter: five[4],
            },
            at + CELL_BYTES,
        ));
    }
    let mut cell = Cell::default();
    let mut at = at + 1;
    let mut take = |field: &mut u8| -> Result<()> {
        *field = *bytes
            .get(at)
            .ok_or_else(|| invalid("module's pattern data ends inside a cell"))?;
        at += 1;
        Ok(())
    };
    // Each of the five bits names one field on its own, and a field no bit names
    // holds nothing rather than repeating what came before it: what a tracker
    // shows as `===` is a mix of the two, and which of them it is belongs to the
    // effect, not to the file.
    if flag & 0x01 != 0 {
        take(&mut cell.note)?;
    }
    if flag & 0x02 != 0 {
        take(&mut cell.instrument)?;
    }
    if flag & 0x04 != 0 {
        take(&mut cell.volume)?;
    }
    if flag & 0x08 != 0 {
        take(&mut cell.effect)?;
    }
    if flag & 0x10 != 0 {
        take(&mut cell.parameter)?;
    }
    // The note column counts up from the table's bottom to the key-off above its
    // top, and a byte above that states no note the format knows.
    if cell.note > NOTE_KEY_OFF {
        cell.note = 0;
    }
    Ok((cell, at))
}

fn parse_instruments(
    bytes: &[u8],
    at: &mut usize,
    count: usize,
    limits: &Limits,
) -> Result<Vec<Instrument>> {
    let mut instruments = Vec::with_capacity(count);
    let mut sample_total = 0usize;
    let mut frame_total = 0usize;
    for index in 0..count {
        let start = *at;
        let header_size = read_le_u32(bytes, start)?;
        let header_size = usize::try_from(header_size)
            .ok()
            .filter(|size| *size >= INSTRUMENT_HEADER_BYTES)
            .ok_or_else(|| {
                invalid(&format!(
                    "instrument {index} states a header of {header_size} bytes, shorter than the \
                     {INSTRUMENT_HEADER_BYTES} up to its fadeout"
                ))
            })?;
        let header_end = start
            .checked_add(header_size)
            .ok_or_else(|| invalid("instrument header size overflows the file"))?;
        if header_end > bytes.len() {
            return Err(invalid(&format!(
                "instrument {index} runs past the end of the file"
            )));
        }
        let name = read_name(bytes, start + INSTRUMENT_NAME, 22);
        let sample_count = usize::from(read_le_u16(bytes, start + INSTRUMENT_SAMPLE_COUNT)?);
        sample_total += sample_count;
        if sample_total > limits.samples {
            return Err(invalid(&format!(
                "module has {sample_total} samples, over the {} limit",
                limits.samples
            )));
        }
        let keymap = bytes[start + INSTRUMENT_KEYMAP..start + INSTRUMENT_KEYMAP + 96].to_vec();
        // One word holds both envelopes' flags: the lower byte shapes the volume
        // and the upper the panning.
        let flags = read_le_u16(bytes, start + INSTRUMENT_FLAGS)?;
        let volume_envelope = Envelope {
            points: read_envelope(bytes, start + INSTRUMENT_VOLUME_ENVELOPE)?,
            flags: flags as u8,
        };
        let panning_envelope = Envelope {
            points: read_envelope(bytes, start + INSTRUMENT_VOLUME_ENVELOPE + ENVELOPE_BYTES)?,
            flags: (flags >> 8) as u8,
        };
        let fadeout = read_le_u16(bytes, start + INSTRUMENT_FADEOUT)?;
        // The sample records start where the instrument's own header ends, which
        // is no fixed offset: anything but the tracker itself puts more between
        // them, and the header's size is what says how much. All of an
        // instrument's records come first and its sample data follows them in the
        // same order, so this is two walks rather than one - which is also where a
        // reader that took the records and the data alternately would go wrong.
        let mut cursor = header_end;
        let mut stated = Vec::with_capacity(sample_count);
        for slot in 0..sample_count {
            if cursor + SAMPLE_RECORD_BYTES > bytes.len() {
                return Err(invalid("module's sample records run past its end"));
            }
            let data_bytes = usize::try_from(read_le_u32(bytes, cursor + SAMPLE_LENGTH)?)
                .map_err(|_| invalid("sample length is too large for this machine to hold"))?;
            let record = SampleHeader {
                name: read_name(bytes, cursor + SAMPLE_NAME, 22),
                data_bytes,
                loop_start: read_le_u32(bytes, cursor + SAMPLE_LOOP_START)? as usize,
                loop_length: read_le_u32(bytes, cursor + SAMPLE_LOOP_LENGTH)? as usize,
                volume: bytes[cursor + SAMPLE_VOLUME],
                finetune: bytes[cursor + SAMPLE_FINETUNE] as i8,
                kind: bytes[cursor + SAMPLE_TYPE],
                panning: bytes[cursor + SAMPLE_PANNING],
                relative_note: bytes[cursor + SAMPLE_RELATIVE_NOTE] as i8,
            };
            let reserved = bytes[cursor + SAMPLE_RESERVED];
            cursor += SAMPLE_RECORD_BYTES;
            if record.kind == SAMPLE_TYPE_ADPCM || reserved == SAMPLE_TYPE_ADPCM {
                return Err(invalid(&format!(
                    "instrument {index}'s sample {slot} is packed in ModPlug's 4-bit form, which \
                     this reader does not undo"
                )));
            }
            if record.kind & SAMPLE_FLAG_STEREO != 0 {
                return Err(invalid(&format!(
                    "instrument {index}'s sample {slot} is one of the pair a ModPlug extension \
                     interleaves, which is not the single stream the format itself states"
                )));
            }
            stated.push(record);
        }
        let mut records = Vec::with_capacity(stated.len());
        for record in stated {
            let width = if record.kind & SAMPLE_FLAG_16_BIT != 0 {
                2
            } else {
                1
            };
            let end = cursor
                .checked_add(record.data_bytes)
                .ok_or_else(|| invalid("sample data size overflows the file"))?;
            if end > bytes.len() {
                return Err(invalid(&format!(
                    "instrument {index}'s sample {} runs past the end of the file",
                    records.len()
                )));
            }
            frame_total += record.data_bytes / width;
            if frame_total > limits.sample_frames {
                return Err(invalid(&format!(
                    "module's samples hold {frame_total} frames, over the {} limit",
                    limits.sample_frames
                )));
            }
            let frames = if width == 2 {
                decode_delta_16(&bytes[cursor..end])
            } else {
                decode_delta_8(&bytes[cursor..end])
            };
            cursor = end;
            // Both loop fields count bytes, which is half as many frames in a
            // 16-bit sample as the header states.
            let loop_start = (record.loop_start / width).min(frames.len());
            // A loop the header puts beyond the sample it names is no loop, which
            // is what both implementations make of it rather than a licence to
            // read past the data.
            let loop_end = if record.kind & 0x03 == LOOP_MODE_NONE {
                loop_start
            } else {
                (loop_start + record.loop_length / width).min(frames.len())
            };
            records.push(Sample {
                name: record.name,
                frames,
                loop_start,
                loop_end,
                ping_pong: record.kind & 0x03 == LOOP_MODE_PING_PONG,
                volume: record.volume,
                finetune: record.finetune,
                panning: record.panning,
                relative_note: record.relative_note,
            });
        }
        *at = cursor;
        instruments.push(Instrument {
            name,
            samples: records,
            keymap,
            volume_envelope,
            panning_envelope,
            fadeout,
        });
    }
    Ok(instruments)
}

/// A sample record as the file states it, before the data it names has been read:
/// the widths and loop points still in the file's own units.
struct SampleHeader {
    name: String,
    data_bytes: usize,
    loop_start: usize,
    loop_length: usize,
    volume: u8,
    finetune: i8,
    kind: u8,
    panning: u8,
    relative_note: i8,
}

/// One envelope's twelve points: a frame number and a value, both 16-bit.
fn read_envelope(bytes: &[u8], at: usize) -> Result<Vec<(u16, u16)>> {
    let slice = bytes
        .get(at..at + ENVELOPE_BYTES)
        .ok_or_else(|| invalid("module's envelopes run past its end"))?;
    Ok(slice
        .as_chunks::<4>().0.iter()
        .map(|pair| {
            (
                u16::from_le_bytes([pair[0], pair[1]]),
                u16::from_le_bytes([pair[2], pair[3]]),
            )
        })
        .collect())
}

/// Undo FastTracker II's byte-wise signed delta: every byte holds the change from
/// the frame before it, and the running total is the frame.
pub fn decode_delta_8(data: &[u8]) -> Vec<i16> {
    let mut value: i8 = 0;
    data.iter()
        .map(|byte| {
            value = value.wrapping_add(*byte as i8);
            widen_8(value)
        })
        .collect()
}

/// Widen the 8-bit form the tracker records samples in to the 16-bit one a mixer
/// reads, by repeating the byte into both halves so full scale stays full scale.
/// The same shape as a multiply by 257, which is what an i8's own bottom has no
/// answer for: -128 times 257 is below what an i16 holds.
fn widen_8(value: i8) -> i16 {
    let wide = i16::from(value);
    (wide << 8) | (wide & 0xFF)
}

/// Undo the same thing over little-endian words, which is the 16-bit form.
pub fn decode_delta_16(data: &[u8]) -> Vec<i16> {
    let mut value: i16 = 0;
    data.as_chunks::<2>().0.iter()
        .map(|pair| {
            value = value.wrapping_add(i16::from_le_bytes([pair[0], pair[1]]));
            value
        })
        .collect()
}

/// A name field, trimmed of the padding every writer fills it with.
fn read_name(bytes: &[u8], at: usize, width: usize) -> String {
    let field = bytes.get(at..at + width).unwrap_or(&[]);
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end])
        .trim_end()
        .to_string()
}

fn read_le_u16(bytes: &[u8], at: usize) -> Result<u16> {
    let pair = bytes
        .get(at..at + 2)
        .ok_or_else(|| invalid("module ends inside a 16-bit field"))?;
    Ok(u16::from_le_bytes([pair[0], pair[1]]))
}

fn read_le_u32(bytes: &[u8], at: usize) -> Result<u32> {
    let four = bytes
        .get(at..at + 4)
        .ok_or_else(|| invalid("module ends inside a 32-bit field"))?;
    Ok(u32::from_le_bytes([four[0], four[1], four[2], four[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_NOTES: &[u8] = include_bytes!("../../tests/fixtures/xm/two-notes.xm");
    const EARLY_FORMAT: &[u8] = include_bytes!("../../tests/fixtures/xm/early-format.xm");

    /// A sample as the builder below writes it: the frames it holds, and the
    /// header bytes saying what the player is to do with them. Loop points are
    /// given in frames, which is where the mixer needs them, and the builder
    /// states them to the file in the bytes the format counts.
    struct SampleSpec {
        name: &'static str,
        frames: Vec<i16>,
        sixteen_bit: bool,
        loop_start: usize,
        loop_length: usize,
        loop_mode: u8,
        volume: u8,
        finetune: i8,
        panning: u8,
        relative_note: i8,
    }

    impl Default for SampleSpec {
        fn default() -> Self {
            Self {
                name: "sample",
                frames: Vec::new(),
                sixteen_bit: false,
                loop_start: 0,
                loop_length: 0,
                loop_mode: LOOP_MODE_NONE,
                volume: 64,
                finetune: 0,
                panning: 0xA4,
                relative_note: 0,
            }
        }
    }

    struct InstrumentSpec {
        name: &'static str,
        /// One slot per note, holding a one-based sample index or 0 for none.
        keymap: Vec<u8>,
        samples: Vec<SampleSpec>,
        envelope_flags: u16,
        fadeout: u16,
    }

    struct ModuleSpec {
        version: u16,
        linear: bool,
        speed: u16,
        bpm: u16,
        orders: Vec<u8>,
        /// One entry per pattern, holding its rows of cells; every pattern has
        /// the same number of channels as the first row of the first one.
        patterns: Vec<Vec<Vec<Cell>>>,
        instruments: Vec<InstrumentSpec>,
        /// Whether the rows are written packed the way the tracker writes them,
        /// or as bare five-byte cells.
        packed: bool,
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

    /// Frames a sample recorded at 8 bits holds, widened the way the reader
    /// widens them, so a round trip through the file is exact.
    fn eight_bit(values: &[i8]) -> Vec<i16> {
        values.iter().map(|value| widen_8(*value)).collect()
    }

    /// One period of a raised cosine over `frames` points, in the 16-bit field.
    fn sine(frames: usize) -> Vec<i16> {
        (0..frames)
            .map(|index| {
                let angle = index as f64 / frames as f64 * std::f64::consts::TAU;
                (angle.sin() * 20_000.0) as i16
            })
            .collect()
    }

    /// The fixture's 8-bit sample: a square wave 128 frames long, whose delta
    /// coding walks between plus and minus 32.
    fn square_frames() -> Vec<i16> {
        eight_bit(&(0..64).flat_map(|_| [32i8, -32]).collect::<Vec<_>>())
    }

    /// The fixture's 16-bit sample: one period of a sine in 64 frames.
    fn wave_frames() -> Vec<i16> {
        sine(64)
    }

    fn field(text: &str, width: usize) -> Vec<u8> {
        let mut out = vec![0u8; width];
        let bytes = text.as_bytes();
        out[..bytes.len().min(width)].copy_from_slice(&bytes[..bytes.len().min(width)]);
        out
    }

    /// Where a sample record starts, found through the name the file states for
    /// it, so a test that rewrites one field does not hard-code an offset that
    /// moves with the data around it.
    fn sample_record_at(bytes: &[u8], name: &str) -> usize {
        let name = name.as_bytes();
        bytes
            .windows(name.len())
            .position(|window| window == name)
            .expect("the sample's name is in the file")
            - SAMPLE_NAME
    }

    fn module_header(spec: &ModuleSpec, channels: usize) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&field("fvid two notes", OFFSET_TRACKER - OFFSET_NAME - 1));
        out.push(0x1A);
        out.extend_from_slice(&field("FastTracker II v2.00", 20));
        out.extend_from_slice(&spec.version.to_le_bytes());
        out.extend_from_slice(&MIN_HEADER_SIZE.to_le_bytes());
        out.extend_from_slice(&(spec.orders.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(channels as u16).to_le_bytes());
        out.extend_from_slice(&(spec.patterns.len() as u16).to_le_bytes());
        out.extend_from_slice(&(spec.instruments.len() as u16).to_le_bytes());
        out.extend_from_slice(&(u16::from(spec.linear)).to_le_bytes());
        out.extend_from_slice(&spec.speed.to_le_bytes());
        out.extend_from_slice(&spec.bpm.to_le_bytes());
        assert_eq!(out.len(), OFFSET_ORDERS);
        let mut table = vec![0u8; ORDER_TABLE_BYTES];
        table[..spec.orders.len()].copy_from_slice(&spec.orders);
        out.extend_from_slice(&table);
        out
    }

    fn pattern_bytes(rows: &[Vec<Cell>], version: u16, packed: bool) -> Vec<u8> {
        let mut data = Vec::new();
        for row in rows {
            for cell in row {
                if packed {
                    data.push(0x80 | 0x1F);
                }
                data.extend_from_slice(&[
                    cell.note,
                    cell.instrument,
                    cell.volume,
                    cell.effect,
                    cell.parameter,
                ]);
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(&9u32.to_le_bytes());
        out.push(0);
        if version == 0x0102 {
            out.push((rows.len() - 1) as u8);
            out.extend_from_slice(&(data.len() as u16).to_le_bytes());
            // The header states nine bytes whatever the version, and the fields
            // this one holds take eight of them, so one is left spare.
            out.push(0);
        } else {
            out.extend_from_slice(&(rows.len() as u16).to_le_bytes());
            out.extend_from_slice(&(data.len() as u16).to_le_bytes());
        }
        out.extend_from_slice(&data);
        out
    }

    fn instrument_bytes(instrument: &InstrumentSpec) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&243u32.to_le_bytes());
        out.extend_from_slice(&field(instrument.name, 22));
        out.push(0);
        out.extend_from_slice(&(instrument.samples.len() as u16).to_le_bytes());
        out.extend_from_slice(&(SAMPLE_RECORD_BYTES as u32).to_le_bytes());
        assert_eq!(out.len(), INSTRUMENT_KEYMAP);
        out.extend_from_slice(&instrument.keymap);
        // Both envelopes twelve points of nothing, which is the shape a module
        // holds when its author never drew one.
        out.extend_from_slice(&[0u8; 2 * ENVELOPE_BYTES]);
        out.extend_from_slice(&[0u8; INSTRUMENT_FLAGS - 225]);
        out.extend_from_slice(&instrument.envelope_flags.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&instrument.fadeout.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(out.len(), 243);
        // The records first, all of them, and then the sample data of this
        // instrument alone - which is where a reader that assumed one flat table
        // across the module would go wrong.
        for sample in &instrument.samples {
            out.extend_from_slice(&sample_record(sample));
        }
        for sample in &instrument.samples {
            out.extend_from_slice(&sample_data(sample));
        }
        out
    }

    fn sample_record(sample: &SampleSpec) -> Vec<u8> {
        let width = usize::from(sample.sixteen_bit) + 1;
        let mut out = Vec::new();
        out.extend_from_slice(&((sample.frames.len() * width) as u32).to_le_bytes());
        out.extend_from_slice(&((sample.loop_start * width) as u32).to_le_bytes());
        out.extend_from_slice(&((sample.loop_length * width) as u32).to_le_bytes());
        out.push(sample.volume);
        out.push(sample.finetune as u8);
        out.push(
            sample.loop_mode
                | if sample.sixteen_bit {
                    SAMPLE_FLAG_16_BIT
                } else {
                    0
                },
        );
        out.push(sample.panning);
        out.push(sample.relative_note as u8);
        out.push(0);
        out.extend_from_slice(&field(sample.name, 22));
        assert_eq!(out.len(), SAMPLE_RECORD_BYTES);
        out
    }

    fn sample_data(sample: &SampleSpec) -> Vec<u8> {
        let mut out = Vec::new();
        if sample.sixteen_bit {
            let mut previous: i16 = 0;
            for frame in &sample.frames {
                out.extend_from_slice(&frame.wrapping_sub(previous).to_le_bytes());
                previous = *frame;
            }
        } else {
            let mut previous: i8 = 0;
            for frame in &sample.frames {
                let value = (frame >> 8) as i8;
                out.push(value.wrapping_sub(previous) as u8);
                previous = value;
            }
        }
        out
    }

    fn write_module(spec: &ModuleSpec) -> Vec<u8> {
        let channels = spec
            .patterns
            .first()
            .and_then(|pattern| pattern.first())
            .map_or(1, Vec::len);
        let mut out = module_header(spec, channels);
        let patterns: Vec<Vec<u8>> = spec
            .patterns
            .iter()
            .map(|rows| pattern_bytes(rows, spec.version, spec.packed))
            .collect();
        let instruments: Vec<Vec<u8>> = spec.instruments.iter().map(instrument_bytes).collect();
        if spec.version >= 0x0104 {
            for bytes in &patterns {
                out.extend_from_slice(bytes);
            }
            for bytes in &instruments {
                out.extend_from_slice(bytes);
            }
        } else {
            for bytes in &instruments {
                out.extend_from_slice(bytes);
            }
            for bytes in &patterns {
                out.extend_from_slice(bytes);
            }
        }
        out
    }

    /// Every note takes the first sample, and the notes from `C-6` up the second
    /// when there is one, so a module of two samples can be heard to hold both.
    fn keymap(notes: usize) -> Vec<u8> {
        (0..96)
            .map(|index| if index < notes { 1 } else { 2 })
            .collect()
    }

    /// One instrument holding the two shapes, which is all of the fixtures but the
    /// one written to refuse.
    fn two_shapes() -> InstrumentSpec {
        InstrumentSpec {
            name: "two shapes",
            keymap: keymap(65),
            envelope_flags: 0,
            fadeout: 768,
            samples: vec![
                SampleSpec {
                    name: "square",
                    frames: square_frames(),
                    loop_start: 0,
                    loop_length: 128,
                    loop_mode: LOOP_MODE_NONE,
                    ..SampleSpec::default()
                },
                SampleSpec {
                    name: "sine",
                    frames: wave_frames(),
                    sixteen_bit: true,
                    loop_start: 0,
                    loop_length: 64,
                    loop_mode: 1,
                    volume: 48,
                    finetune: 3,
                    panning: 0x40,
                    relative_note: -12,
                },
            ],
        }
    }

    /// A module of one short sample, enough for the tests about rows rather than
    /// sound.
    fn one_shape() -> Vec<InstrumentSpec> {
        vec![InstrumentSpec {
            name: "one",
            keymap: vec![1; 96],
            envelope_flags: 0,
            fadeout: 0,
            samples: vec![SampleSpec {
                name: "a",
                frames: eight_bit(&[8, 8]),
                ..SampleSpec::default()
            }],
        }]
    }

    /// The committed fixture: two channels, one instrument holding an 8-bit
    /// square and a 16-bit sine, and three rows - the notes, a row holding them,
    /// and the key-off - at the tracker's own six ticks and 125 a minute, which
    /// is 120 ms a row.
    fn two_notes() -> Vec<u8> {
        write_module(&ModuleSpec {
            version: 0x0104,
            linear: true,
            speed: 6,
            bpm: 125,
            orders: vec![0],
            patterns: vec![vec![
                vec![cell(58, 1, 0x40, 0, 0), cell(73, 1, 0x40, 0, 0)],
                vec![cell(0, 0, 0, 0, 0), cell(0, 0, 0, 0, 0)],
                vec![
                    cell(NOTE_KEY_OFF, 0, 0, 0, 0),
                    cell(NOTE_KEY_OFF, 0, 0, 0, 0),
                ],
            ]],
            instruments: vec![two_shapes()],
            packed: true,
        })
    }

    /// The same shape as it looks to the reader before 1.04: the patterns after
    /// the instruments, the row count in one byte counting from zero, and the
    /// Amiga frequency table asked for rather than the linear one.
    fn early_format() -> Vec<u8> {
        write_module(&ModuleSpec {
            version: 0x0102,
            linear: false,
            speed: 4,
            bpm: 125,
            orders: vec![0, 0],
            patterns: vec![vec![
                vec![cell(1, 1, 0x40, 0, 0)],
                vec![cell(0, 0, 0, EFFECT_TEMPO, 40)],
            ]],
            instruments: vec![InstrumentSpec {
                name: "one shape",
                keymap: vec![1; 96],
                envelope_flags: 0,
                fadeout: 0,
                samples: vec![SampleSpec {
                    name: "ramp",
                    frames: eight_bit(&[0, 32, 64, 96, 96, 64, 32, 0]),
                    ..SampleSpec::default()
                }],
            }],
            packed: false,
        })
    }

    fn parse(bytes: &[u8]) -> Result<Module> {
        Module::parse(bytes, &Limits::default())
    }

    #[test]
    fn the_committed_fixtures_are_the_bytes_the_builder_writes() {
        let two = two_notes();
        let early = early_format();
        if std::env::var_os("FVID_WRITE_XM_FIXTURES").is_some() {
            std::fs::create_dir_all("tests/fixtures/xm").expect("directory");
            std::fs::write("tests/fixtures/xm/two-notes.xm", &two).expect("fixture");
            std::fs::write("tests/fixtures/xm/early-format.xm", &early).expect("fixture");
        }
        assert_eq!(
            TWO_NOTES,
            &two[..],
            "the committed module is not what the builder writes"
        );
        assert_eq!(
            EARLY_FORMAT,
            &early[..],
            "the committed early module is not what the builder writes"
        );
    }

    #[test]
    fn a_module_reads_back_as_the_rows_and_shapes_it_states() {
        let module = parse(TWO_NOTES).expect("the fixture opens");
        assert_eq!(module.version, 0x0104);
        assert!(module.linear, "the fixture states the linear table");
        assert_eq!(module.name, "fvid two notes");
        assert_eq!(module.tracker, "FastTracker II v2.00");
        assert_eq!(module.channels, 2);
        assert_eq!(module.orders, vec![0]);
        assert_eq!(module.instruments.len(), 1);
        assert_eq!(module.instruments[0].samples.len(), 2);
        assert_eq!(module.instruments[0].name, "two shapes");
        // Six ticks at 125 a minute is 120 ms, and three rows of it is the whole
        // timeline; the tail packet adds no length to the module's own end.
        assert_eq!(module.timeline().len(), 3);
        assert_eq!(
            module
                .timeline()
                .iter()
                .map(|row| row.duration_microseconds)
                .collect::<Vec<_>>(),
            vec![120_000; 3]
        );
        assert_eq!(
            module
                .timeline()
                .iter()
                .map(|row| row.start_microseconds)
                .collect::<Vec<_>>(),
            vec![0, 120_000, 240_000]
        );
        assert_eq!(module.duration(), Duration::from_millis(360));
        assert_eq!(module.packets(), 4);
        // A packet is one row of cells, so four bytes of length, two of tempo, a
        // tick count and a spare, then five bytes for each of two channels.
        assert_eq!(module.packet(0).len(), ROW_HEADER_BYTES + 2 * CELL_BYTES);
        assert_eq!(
            module.row_cells(&module.timeline()[0]),
            &[cell(58, 1, 0x40, 0, 0), cell(73, 1, 0x40, 0, 0)]
        );
        assert_eq!(module.packet(3).len(), ROW_HEADER_BYTES + 2 * CELL_BYTES);
        assert!(
            module.packet(3)[ROW_HEADER_BYTES..]
                .iter()
                .all(|byte| *byte == 0)
        );
        // The bytes a packet states are its row's own: read back, they name the
        // tempo to mix at and the cells every channel is told to act on.
        let packet = module.packet(0);
        assert_eq!(
            u32::from_le_bytes([packet[0], packet[1], packet[2], packet[3]]),
            120_000
        );
        assert_eq!(
            u16::from_le_bytes([packet[4], packet[5]]),
            module.timeline()[0].bpm
        );
        assert_eq!(packet[6], module.timeline()[0].speed);
        assert_eq!(
            &packet[ROW_HEADER_BYTES..ROW_HEADER_BYTES + CELL_BYTES],
            &[58, 1, 0x40, 0, 0]
        );
    }

    #[test]
    fn every_packet_reads_back_as_the_row_that_wrote_it() {
        let module = parse(TWO_NOTES).expect("the fixture opens");
        for index in 0..module.packets() {
            let row = read_row(&module.packet(index)).expect("a whole row");
            assert_eq!(row.cells.len(), module.channels);
            assert_eq!(
                u64::from(row.duration_microseconds),
                module.packet_time(index).1
            );
            if let Some(timeline) = module.timeline().get(index) {
                assert_eq!(row.speed, timeline.speed);
                assert_eq!(row.bpm, timeline.bpm);
                assert_eq!(row.cells, module.row_cells(timeline).to_vec());
            } else {
                assert!(
                    row.cells.iter().all(|cell| *cell == Cell::default()),
                    "the tail packet asks no channel to do anything"
                );
            }
        }
        // A packet that is not a whole number of cells is refused rather than
        // half read.
        assert!(read_row(&[]).is_err());
        assert!(read_row(&module.packet(0)[..ROW_HEADER_BYTES + CELL_BYTES - 1]).is_err());
    }

    #[test]
    fn the_samples_delta_coded_in_the_file_are_the_frames_in_memory() {
        let module = parse(TWO_NOTES).expect("the fixture opens");
        let samples = &module.instruments[0].samples;
        // The 8-bit square states its length in bytes, one frame apiece.
        assert_eq!(samples[0].frames, square_frames());
        assert_eq!(samples[0].frames.len(), 128);
        assert!(!samples[0].loops(), "the sample states no loop mode");
        // The 16-bit one states bytes, and holds half as many frames as that.
        assert_eq!(samples[1].frames.len(), 64);
        assert_eq!(samples[1].volume, 48);
        assert_eq!(samples[1].finetune, 3);
        assert_eq!(samples[1].panning, 0x40);
        assert_eq!(samples[1].relative_note, -12);
        assert_eq!(samples[1].loop_start, 0);
        assert_eq!(samples[1].loop_end, 64);
        assert!(samples[1].loops());
        assert!(!samples[1].ping_pong);
        // The first frame of each is what the file's own running total starts at.
        assert_eq!(samples[0].frames[0], eight_bit(&[32])[0]);
        assert_eq!(samples[1].frames, wave_frames());
    }

    #[test]
    fn a_loop_counted_in_bytes_is_read_as_frames_and_clamped_to_what_there_is() {
        let mut spec = two_notes();
        let square = sample_record_at(&spec, "square");
        // A forward loop starting two frames before the square's end and running
        // eight, which is six frames the data does not hold.
        spec[square + SAMPLE_TYPE] = 1;
        spec[square + SAMPLE_LOOP_START..square + SAMPLE_LOOP_START + 4]
            .copy_from_slice(&126u32.to_le_bytes());
        spec[square + SAMPLE_LOOP_LENGTH..square + SAMPLE_LOOP_LENGTH + 4]
            .copy_from_slice(&8u32.to_le_bytes());
        // The same field on the 16-bit sample counts bytes, so half of it is the
        // frame the mixer loops at.
        let sine = sample_record_at(&spec, "sine");
        spec[sine + SAMPLE_LOOP_START..sine + SAMPLE_LOOP_START + 4]
            .copy_from_slice(&40u32.to_le_bytes());
        let module = parse(&spec).expect("the module opens");
        let samples = &module.instruments[0].samples;
        assert_eq!(samples[0].frames, square_frames(), "only the loop moved");
        assert_eq!((samples[0].loop_start, samples[0].loop_end), (126, 128));
        assert!(samples[0].loops());
        assert!(!samples[0].ping_pong);
        assert_eq!((samples[1].loop_start, samples[1].loop_end), (20, 64));
    }

    #[test]
    fn both_packed_forms_of_a_row_hold_the_same_cells() {
        let mut spec = ModuleSpec {
            version: 0x0104,
            linear: true,
            speed: 6,
            bpm: 125,
            orders: vec![0],
            patterns: vec![vec![vec![cell(48, 1, 0x40, 1, 2), cell(0, 0, 0, 0, 0)]]],
            instruments: one_shape(),
            packed: true,
        };
        let packed = parse(&write_module(&spec)).expect("packed rows");
        spec.packed = false;
        let literal = parse(&write_module(&spec)).expect("literal rows");
        for module in [packed, literal] {
            assert_eq!(
                module.row_cells(&module.timeline()[0]),
                &[cell(48, 1, 0x40, 1, 2), cell(0, 0, 0, 0, 0)]
            );
        }
    }

    #[test]
    fn a_row_that_states_tempo_measures_the_rows_from_it_on() {
        // Six ticks a row at 125, then a row that slows the tempo to 50 and one
        // that doubles the ticks to 12: five rows, the last three twice as long
        // as the first and again as long as the second.
        let bytes = write_module(&ModuleSpec {
            version: 0x0104,
            linear: true,
            speed: 6,
            bpm: 125,
            orders: vec![0],
            patterns: vec![vec![
                vec![cell(60, 1, 0x40, 0, 0)],
                vec![cell(0, 0, 0, EFFECT_TEMPO, 50)],
                vec![cell(0, 0, 0, EFFECT_TEMPO, 12)],
                vec![cell(0, 0, 0, 0, 0)],
                vec![cell(NOTE_KEY_OFF, 0, 0, 0, 0)],
            ]],
            instruments: one_shape(),
            packed: true,
        });
        let module = parse(&bytes).expect("the module opens");
        assert_eq!(
            module
                .timeline()
                .iter()
                .map(|row| (row.speed, row.bpm, row.duration_microseconds))
                .collect::<Vec<_>>(),
            vec![
                (6, 125, 120_000),
                (6, 50, 300_000),
                (12, 50, 600_000),
                (12, 50, 600_000),
                (12, 50, 600_000),
            ]
        );
        // The tempo a row states measures that row, and so does the tick count.
        assert_eq!(tick_microseconds(50), 50_000);
        assert_eq!(tick_microseconds(125), 20_000);
    }

    #[test]
    fn a_module_that_jumps_back_is_one_pass_of_what_it_played() {
        // Two orders of one row each, the second jumping to the first: the walk
        // stops where the loop would begin rather than laying out a timeline with
        // no end to it.
        let bytes = write_module(&ModuleSpec {
            version: 0x0104,
            linear: true,
            speed: 6,
            bpm: 125,
            orders: vec![0, 1],
            patterns: vec![
                vec![vec![cell(60, 1, 0x40, 0, 0)]],
                vec![vec![cell(0, 0, 0, EFFECT_JUMP, 0)]],
            ],
            instruments: Vec::new(),
            packed: true,
        });
        let module = parse(&bytes).expect("the module opens");
        assert_eq!(
            module
                .timeline()
                .iter()
                .map(|row| row.order)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn a_row_plays_the_pattern_its_order_names() {
        // The order table names pattern 1 first and pattern 0 second, and song
        // length exceeds the pattern count: a row's cells come from the pattern
        // its order entry names, not from the pattern at the order's index.
        let bytes = write_module(&ModuleSpec {
            version: 0x0104,
            linear: true,
            speed: 6,
            bpm: 125,
            orders: vec![1, 0, 1],
            patterns: vec![
                vec![vec![cell(60, 1, 0x40, 0, 0)]],
                vec![vec![cell(72, 1, 0x40, 0, 0)]],
            ],
            instruments: Vec::new(),
            packed: true,
        });
        let module = parse(&bytes).expect("the module opens");
        let notes: Vec<u8> = module
            .timeline()
            .iter()
            .map(|row| module.row_cells(row)[0].note)
            .collect();
        assert_eq!(notes, vec![72, 60, 72]);
        for index in 0..module.packets() {
            module.packet(index);
        }
    }

    #[test]
    fn a_break_names_the_row_the_tracker_shows_it_in_decimal() {
        let bytes = write_module(&ModuleSpec {
            version: 0x0104,
            linear: true,
            speed: 6,
            bpm: 125,
            orders: vec![0, 1],
            patterns: vec![
                vec![vec![cell(60, 1, 0x40, EFFECT_BREAK, 0x18)]],
                (0..20).map(|_| vec![cell(0, 0, 0, 0, 0)]).collect(),
            ],
            instruments: Vec::new(),
            packed: true,
        });
        let module = parse(&bytes).expect("the module opens");
        // The break at order 0 hands over to the eighteenth row of the next order,
        // which is the row the tracker calls 18 rather than the 24th of a
        // parameter read as hex.
        assert_eq!(
            module
                .timeline()
                .iter()
                .map(|row| (row.order, row.row))
                .collect::<Vec<_>>(),
            vec![(0, 0), (1, 18), (1, 19)]
        );
    }

    #[test]
    fn the_layout_a_version_before_1_04_used_is_read_where_it_put_things() {
        let module = parse(EARLY_FORMAT).expect("the early fixture opens");
        assert_eq!(module.version, 0x0102);
        assert!(!module.linear, "the fixture asks for the Amiga table");
        assert_eq!(module.channels, 1);
        // The row count is one byte counting from zero, so the two rows the
        // builder wrote read back as two.
        assert_eq!(module.patterns[0].row_count(), 2);
        assert_eq!(
            module.instruments[0].samples[0].frames,
            eight_bit(&[0, 32, 64, 96, 96, 64, 32, 0])
        );
        // Four ticks at 125 is 80 ms, the row stating a tempo of 40 is played
        // under 40, and the order that starts again holds that tempo - which is
        // why the second pass is as slow as the end of the first.
        assert_eq!(
            module
                .timeline()
                .iter()
                .map(|row| (row.order, row.duration_microseconds))
                .collect::<Vec<_>>(),
            vec![(0, 80_000), (0, 250_000), (1, 250_000), (1, 250_000)]
        );
        assert_eq!(module.duration(), Duration::from_micros(830_000));
    }

    #[test]
    fn the_tempo_a_module_starts_at_falls_back_to_the_tracker_s_own_defaults() {
        let mut bytes = two_notes();
        // A header stating no tempo and a speed of 200: neither is a number the
        // tracker types, so the reader takes its defaults for both.
        bytes.splice(OFFSET_SPEED..OFFSET_SPEED + 2, 200u16.to_le_bytes());
        bytes.splice(OFFSET_BPM..OFFSET_BPM + 2, 0u16.to_le_bytes());
        let module = parse(&bytes).expect("the module opens");
        assert_eq!(
            (module.speed, module.bpm),
            (DEFAULT_SPEED, DEFAULT_BPM)
        );
        // Every row is measured by them, so the module is its three rows at 120
        // ms each.
        assert_eq!(module.duration(), Duration::from_millis(360));
    }

    #[test]
    fn a_file_that_is_not_a_module_is_refused_before_anything_is_read() {
        assert!(parse(b"RIFF....WAVEfmt ").is_err());
        let mut bytes = two_notes();
        bytes[37] = b' ';
        assert!(
            parse(&bytes).is_err(),
            "the name no longer ends where the format puts it"
        );
        let mut bytes = two_notes();
        bytes[OFFSET_VERSION..OFFSET_VERSION + 2].copy_from_slice(&0x0105u16.to_le_bytes());
        let error = parse(&bytes)
            .expect_err("a version after the last the format states");
        assert!(
            error.to_string().contains("not one of the three"),
            "{error}"
        );
        let mut bytes = two_notes();
        bytes[OFFSET_HEADER_SIZE..OFFSET_HEADER_SIZE + 4].copy_from_slice(&200u32.to_le_bytes());
        assert!(parse(&bytes).is_err(), "a header with no order table in it");
    }

    #[test]
    fn a_sample_in_a_form_this_reader_does_not_undo_is_named_as_one() {
        for (kind, reserved, message) in [
            (SAMPLE_TYPE_ADPCM, 0u8, "ModPlug's 4-bit form"),
            (0, SAMPLE_TYPE_ADPCM, "ModPlug's 4-bit form"),
            (SAMPLE_FLAG_STEREO, 0, "one of the pair a ModPlug extension"),
        ] {
            let mut bytes = two_notes();
            let record = sample_record_at(&bytes, "square");
            bytes[record + SAMPLE_TYPE] = kind;
            bytes[record + SAMPLE_RESERVED] = reserved;
            let error = parse(&bytes)
                .expect_err("a sample the reader cannot take")
                .to_string();
            assert!(error.contains(message), "{error} states no {message}");
        }
    }

    #[test]
    fn a_module_lying_about_its_own_sizes_is_refused_not_read_past_them() {
        // An order naming a pattern the header never wrote.
        let mut bytes = two_notes();
        bytes[OFFSET_ORDERS] = 7;
        assert!(parse(&bytes).is_err(), "no pattern seven to play");
        // Sample data running past the end of the file.
        let mut bytes = two_notes();
        let record = sample_record_at(&bytes, "square");
        bytes[record + SAMPLE_LENGTH..record + SAMPLE_LENGTH + 4]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&bytes).is_err(), "the file holds no such sample");
        // A pattern whose rows run past the packed data its header states.
        let mut bytes = two_notes();
        let pattern = OFFSET_ORDERS + ORDER_TABLE_BYTES;
        bytes[pattern + 7..pattern + 9].copy_from_slice(&8u16.to_le_bytes());
        assert!(
            parse(&bytes).is_err(),
            "the rows do not fit the eight bytes stated"
        );
        // A channel count above the most the format states.
        let mut bytes = two_notes();
        bytes[OFFSET_CHANNELS..OFFSET_CHANNELS + 2].copy_from_slice(&40u16.to_le_bytes());
        let error = parse(&bytes).expect_err("no 40 channels");
        assert!(error.to_string().contains("outside the 1..=32"), "{error}");
        // A sample count the instrument's own records cannot fill.
        let mut bytes = two_notes();
        let record = sample_record_at(&bytes, "square");
        // The builder states the tracker's own 243-byte instrument header, and the
        // first sample record starts right after it.
        let instrument = record - 243;
        bytes[instrument + INSTRUMENT_SAMPLE_COUNT..instrument + INSTRUMENT_SAMPLE_COUNT + 2]
            .copy_from_slice(&9u16.to_le_bytes());
        assert!(
            parse(&bytes).is_err(),
            "nine samples where the file holds two"
        );
    }

    #[test]
    fn the_note_column_keeps_what_the_format_names_and_drops_what_it_does_not() {
        // The key-off above the table's top stays; a byte above that states no
        // note the format knows, so the cell is read as naming none.
        let bytes = write_module(&ModuleSpec {
            version: 0x0104,
            linear: true,
            speed: 6,
            bpm: 125,
            orders: vec![0],
            patterns: vec![vec![vec![
                cell(NOTE_KEY_OFF, 1, 0, 0, 0),
                cell(200, 1, 0, 0, 0),
            ]]],
            instruments: Vec::new(),
            packed: true,
        });
        let module = parse(&bytes).expect("the module opens");
        assert_eq!(
            module.row_cells(&module.timeline()[0])[0].note,
            NOTE_KEY_OFF
        );
        assert_eq!(module.row_cells(&module.timeline()[0])[1].note, 0);
    }

    #[test]
    fn a_keymap_points_a_note_at_a_sample_and_no_note_at_nothing() {
        let module = parse(TWO_NOTES).expect("the fixture opens");
        // The fixture maps its first 65 notes to the square and the rest to the
        // sine, counting notes from the bottom of the table.
        assert_eq!(
            module.sample_for(0, 1).map(|sample| sample.name.as_str()),
            Some("square")
        );
        assert_eq!(
            module.sample_for(0, 65).map(|sample| sample.name.as_str()),
            Some("square")
        );
        assert_eq!(
            module.sample_for(0, 66).map(|sample| sample.name.as_str()),
            Some("sine")
        );
        assert!(
            module.sample_for(0, 0).is_none(),
            "a cell naming no note selects no sample"
        );
        assert!(
            module.sample_for(0, NOTE_KEY_OFF).is_none(),
            "the key-off is above the table, not its top note"
        );
        assert!(
            module.sample_for(1, 60).is_none(),
            "the module has one instrument"
        );
        assert!(
            module.instrument(0).is_none(),
            "instrument zero is the one no cell names"
        );
        assert_eq!(
            module.instrument(1).map(|i| i.name.as_str()),
            Some("two shapes")
        );
    }

    #[test]
    fn a_packet_index_lands_on_the_last_row_that_had_begun() {
        let module = parse(TWO_NOTES).expect("the fixture opens");
        assert_eq!(module.packet_at_microsecond(0), 0);
        assert_eq!(module.packet_at_microsecond(119_999), 0);
        assert_eq!(module.packet_at_microsecond(120_000), 1);
        assert_eq!(module.packet_at_microsecond(359_999), 2);
        // Past the module's own end the cursor is on the tail, which is the last
        // packet there is.
        assert_eq!(module.packet_at_microsecond(360_000), module.packets() - 1);
        assert_eq!(module.packet_time(3), (360_000, 120_000));
        assert_eq!(module.packet_time(0), (0, 120_000));
        // Every packet of the module's own timeline starts where its row does.
        for index in 0..module.timeline().len() {
            let row = &module.timeline()[index];
            assert_eq!(module.packet_time(index).0, row.start_microseconds);
            assert_eq!(module.packet_at_microsecond(row.start_microseconds), index);
        }
    }
}
