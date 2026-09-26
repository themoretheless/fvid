//! Standard MIDI File parsing for the player's own audio path.
//!
//! A Standard MIDI File carries no encoded audio at all: it carries a score, so
//! what this container hands a decoder is the performance events that fall
//! inside a slice of the timeline, already placed on a wall clock. That mapping
//! belongs here rather than in the synthesiser because only the file states the
//! unit a tick is measured in and only its track data states the tempo changes
//! that unit is scaled by.
//!
//! The slice form is what keeps the container inside the audio pipeline the
//! other readers use: a packet is 100 ms of the timeline, its timestamps are in
//! milliseconds, and the events inside it are offset from that packet's start.

use crate::{Result, invalid};
use std::time::Duration;

/// What a file plays at until a track says otherwise: 120 quarters a minute.
pub const DEFAULT_TEMPO_MICROSECONDS: u32 = 500_000;
/// The span of the timeline one packet carries, in microseconds.
pub const WINDOW_MICROSECONDS: u64 = 100_000;
/// Ticks per second the packet timestamps use, which is the millisecond, so a
/// packet is a round number of them long.
pub const WINDOW_TIMESCALE: u32 = 1_000;
/// Milliseconds one packet spans.
pub const WINDOW_MILLISECONDS: u64 = WINDOW_MICROSECONDS / 1_000;
/// Bytes per event record inside a packet: the tag, the offset from the packet
/// start, the channel, the two one-byte operands and a wide operand for the one
/// message that needs more than a byte.
pub const RECORD_BYTES: usize = 10;

const TAG_NOTE_ON: u8 = 1;
const TAG_NOTE_OFF: u8 = 2;
const TAG_PROGRAM: u8 = 3;
const TAG_CONTROL: u8 = 4;
const TAG_PITCH_BEND: u8 = 5;

/// Bounds a file has to stay inside to be read at all. A performance is small
/// data, so these sit far above what a real one weighs and are there to turn a
/// hostile header into a refusal rather than an allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest file the reader takes in.
    pub file_bytes: usize,
    /// Largest number of tracks sounding together.
    pub tracks: usize,
    /// Largest number of performance events across them.
    pub events: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 32 << 20,
            tracks: 64,
            events: 1_000_000,
        }
    }
}

/// One thing that happens on the timeline, in the operands a MIDI channel
/// message carries. What a file can state that has no bearing on the sound -
/// system exclusive dumps, aftertouch, lyrics, cues - is dropped at parse time
/// rather than given a shape here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    NoteOn {
        channel: u8,
        key: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        key: u8,
    },
    Program {
        channel: u8,
        number: u8,
    },
    Control {
        channel: u8,
        number: u8,
        value: u8,
    },
    /// The 14-bit bending word, 8192 being the untuned centre.
    PitchBend {
        channel: u8,
        value: u16,
    },
}

/// An event at a point of the wall clock the file's tempo map produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimedEvent {
    pub microseconds: u64,
    pub event: Event,
}

/// An event inside a packet, measured from that packet's own start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowEvent {
    pub offset_microseconds: u32,
    pub event: Event,
}

/// A parsed file: what it says about its own shape, plus every event of the
/// performance in the order a synthesiser has to see them.
#[derive(Clone, Debug)]
pub struct SmfFile {
    /// 0 plays one track, 1 plays them together with track 0 as the conductor,
    /// 2 plays independent tracks a listener selects between.
    pub format: u8,
    pub ticks_per_quarter: u16,
    pub track_count: usize,
    /// What the performance called itself, which is the first name stated for the
    /// whole file or, failing that, the first stated for a track. Empty when the
    /// file says nothing, as most do.
    pub name: String,
    events: Vec<TimedEvent>,
    last_microsecond: u64,
}

impl SmfFile {
    /// Read a file, refusing anything the bounds above or the format itself
    /// rules out.
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Self> {
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "SMF is {} bytes, over the {} byte limit",
                bytes.len(),
                limits.file_bytes
            )));
        }
        if bytes.len() < 14 || &bytes[0..4] != b"MThd" {
            return Err(invalid("not a Standard MIDI File: no MThd header"));
        }
        // The header states its own length, and less than six bytes states less
        // than the format number, so such a file has no header to read.
        let header_length = read_be_u32(bytes, 4)?;
        let header_length = usize::try_from(header_length)
            .ok()
            .filter(|length| (6..=bytes.len() - 8).contains(length))
            .ok_or_else(|| invalid("SMF header length is not a header this file has"))?;
        let format = read_be_u16(bytes, 8)?;
        if format > 2 {
            return Err(invalid("SMF format is not one of the three the spec lists"));
        }
        let track_count = usize::from(read_be_u16(bytes, 10)?);
        if track_count == 0 {
            return Err(invalid("SMF states no track to read"));
        }
        if track_count > limits.tracks {
            return Err(invalid(&format!(
                "SMF has {track_count} tracks, over the {} limit",
                limits.tracks
            )));
        }
        // The top bit of the division picks the clock: clear means ticks per
        // quarter note, set means SMPTE frames and subframes per second, which
        // would need the tempo map replaced by a frame rate to place at all.
        let division = read_be_u16(bytes, 12)?;
        if division & 0x8000 != 0 {
            return Err(invalid(
                "SMF timed in SMPTE frames is not supported; only quarter-note divisions",
            ));
        }
        let ppq = u64::from(division);
        if ppq == 0 {
            return Err(invalid("SMF says a quarter note takes no ticks"));
        }
        let mut raw: Vec<RawEvent> = Vec::new();
        let mut tempos: Vec<(u64, u32)> = Vec::new();
        let mut ends: Vec<u64> = Vec::with_capacity(track_count);
        let mut names: Vec<(u8, String)> = Vec::new();
        let mut at = 8 + header_length;
        for _ in 0..track_count {
            let header = read_track_header(bytes, at)?;
            let payload = at + 8;
            let end = payload
                .checked_add(header.length)
                .filter(|end| *end <= bytes.len())
                .ok_or_else(|| invalid("SMF track runs past the end of the file"))?;
            ends.push(parse_track(
                &bytes[payload..end],
                header.kind,
                limits,
                &mut raw,
                &mut tempos,
                &mut names,
            )?);
            at = end;
        }
        // A file that never states a tempo plays at the default throughout.
        if tempos.is_empty() {
            tempos.push((0, DEFAULT_TEMPO_MICROSECONDS));
        }
        let segments = tempo_segments(&tempos, ppq);
        let place = |tick: u64| tick_to_microsecond(&segments, ppq, tick);
        let mut events = raw
            .iter()
            .map(|event| TimedEvent {
                microseconds: place(event.tick),
                event: event.event,
            })
            .collect::<Vec<_>>();
        events.sort_by_key(|event| event.microseconds);
        // The timeline runs to where the longest track stopped, which is not the
        // same as where its last note was: a track that ends in a bar of rest
        // still asks the listener to wait out that bar.
        let last_microsecond = ends
            .iter()
            .map(|tick| place(*tick))
            .chain(events.last().map(|event| event.microseconds))
            .max()
            .unwrap_or(0);
        Ok(Self {
            format: format as u8,
            ticks_per_quarter: division,
            track_count,
            name: names
                .iter()
                .find(|(kind, _)| *kind == 0x00)
                .or_else(|| names.iter().find(|(kind, _)| *kind == 0x03))
                .map(|(_, name)| name.clone())
                .unwrap_or_default(),
            events,
            last_microsecond,
        })
    }

    /// The file's own length: the point its last track stopped.
    pub fn duration(&self) -> Duration {
        Duration::from_micros(self.last_microsecond)
    }

    /// Packets to hand out: enough to cover the timeline, plus one more so a
    /// note held to the end is heard releasing rather than cut off.
    pub fn windows(&self) -> usize {
        (self.last_microsecond / WINDOW_MICROSECONDS) as usize + 2
    }

    /// The packet for one window: the events inside it, each measured from this
    /// window's own start.
    pub fn window_packet(&self, index: usize) -> Vec<u8> {
        let from = index as u64 * WINDOW_MICROSECONDS;
        let to = from + WINDOW_MICROSECONDS;
        let start = self
            .events
            .partition_point(|event| event.microseconds < from);
        let mut data = Vec::new();
        for event in &self.events[start..] {
            if event.microseconds >= to {
                break;
            }
            let offset = u32::try_from(event.microseconds - from).unwrap_or(u32::MAX);
            write_record(event.event, offset, &mut data);
        }
        data
    }
}

/// What one track holds before the tempo map turns its ticks into time.
#[derive(Clone, Copy)]
struct RawEvent {
    tick: u64,
    event: Event,
}

/// A chunk's own four bytes and length.
struct ChunkHeader {
    kind: [u8; 4],
    length: usize,
}

fn read_track_header(bytes: &[u8], at: usize) -> Result<ChunkHeader> {
    let kind = bytes
        .get(
            at..at
                .checked_add(4)
                .ok_or_else(|| invalid("SMF chunk kind overflows"))?,
        )
        .ok_or_else(|| invalid("SMF ends where a track header should start"))?;
    let kind = [kind[0], kind[1], kind[2], kind[3]];
    let length = usize::try_from(read_be_u32(bytes, at + 4)?)
        .map_err(|_| invalid("SMF track is longer than any file this reader can hold"))?;
    Ok(ChunkHeader { kind, length })
}

/// Read one track's messages, and report the tick it ended at.
///
/// The file's own length bounds a track's data, so a writer that padded the last
/// track with a truncated event is read as far as the bytes reach rather than
/// refused; anything that cannot be placed at all is.
fn parse_track(
    data: &[u8],
    kind: [u8; 4],
    limits: &Limits,
    raw: &mut Vec<RawEvent>,
    tempos: &mut Vec<(u64, u32)>,
    names: &mut Vec<(u8, String)>,
) -> Result<u64> {
    if kind != *b"MTrk" {
        return Err(invalid(&format!(
            "SMF chunk is headed {:?}, not a track",
            String::from_utf8_lossy(&kind)
        )));
    }
    let mut at = 0usize;
    let mut tick: u64 = 0;
    let mut running: Option<u8> = None;
    while at < data.len() {
        let delta = read_vlq(data, &mut at)?;
        tick = tick.saturating_add(u64::from(delta));
        let mut status = *data
            .get(at)
            .ok_or_else(|| invalid("SMF event ends at its delta time"))?;
        if status < 0x80 {
            // Running status: the writer left the previous message's status in
            // force, which is how a run of notes costs one byte each.
            status = running.ok_or_else(|| {
                invalid("SMF starts a message with a data byte and no status to run")
            })?;
        } else {
            at += 1;
            if status < 0xF0 {
                running = Some(status);
            }
        }
        let channel = status & 0x0f;
        match status {
            0xFF => match parse_meta(data, &mut at, tick, tempos, names)? {
                // An end of track is the end of the track's own data, however
                // much padding a writer left after it.
                Meta::End => break,
                Meta::None => {}
            },
            // System common. A dump bounds itself by a length, and nothing in
            // one changes a note, so it is stepped over.
            0xF0 | 0xF7 => {
                let length = read_vlq(data, &mut at)? as usize;
                at = at
                    .checked_add(length)
                    .filter(|end| *end <= data.len())
                    .ok_or_else(|| {
                        invalid("SMF system exclusive runs past the end of its track")
                    })?;
            }
            0xF2 => {
                read_byte(data, &mut at)?;
                read_byte(data, &mut at)?;
            }
            0xF1 | 0xF3 | 0xF4 | 0xF5 | 0xF6 | 0xF9 | 0xFB | 0xFC | 0xFD => {
                read_byte(data, &mut at)?;
            }
            0xF8 | 0xFA | 0xFE => {}
            _ => {
                let operands = if matches!(status & 0xf0, 0xC0 | 0xD0) {
                    1
                } else {
                    2
                };
                let mut operands_bytes = [0u8; 2];
                for slot in operands_bytes.iter_mut().take(operands) {
                    *slot = read_byte(data, &mut at)?;
                }
                let Some(event) = channel_event(status, channel, &operands_bytes) else {
                    continue;
                };
                push_event(raw, tick, event, limits)?;
            }
        }
    }
    Ok(tick)
}

/// What a meta event turned out to be: the two that matter to the timeline are
/// answered apart from the rest, which a synthesiser has no use for.
enum Meta {
    None,
    End,
}

fn parse_meta(
    data: &[u8],
    at: &mut usize,
    tick: u64,
    tempos: &mut Vec<(u64, u32)>,
    names: &mut Vec<(u8, String)>,
) -> Result<Meta> {
    let kind = read_byte(data, at)?;
    let length = read_vlq(data, at)? as usize;
    let end = at
        .checked_add(length)
        .filter(|end| *end <= data.len())
        .ok_or_else(|| invalid("SMF meta event runs past the end of its track"))?;
    let payload = &data[*at..end];
    *at = end;
    match kind {
        0x51 => {
            let bytes: [u8; 3] = payload
                .try_into()
                .map_err(|_| invalid("SMF tempo change is not three bytes long"))?;
            let microsecond =
                (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
            if microsecond == 0 {
                return Err(invalid(
                    "SMF tempo change says a quarter note takes no time",
                ));
            }
            tempos.push((tick, microsecond));
        }
        0x2F => return Ok(Meta::End),
        // The two names a file can give: the whole performance's, and one track's.
        0x00 | 0x03 => {
            let text: String = payload.iter().map(|byte| char::from(*byte)).collect();
            if !text.is_empty() {
                names.push((kind, text));
            }
        }
        _ => {}
    }
    Ok(Meta::None)
}

/// The channel message a status byte and its operands state, or `None` for the
/// ones that carry no sound - the two aftertouch messages.
fn channel_event(status: u8, channel: u8, operands: &[u8; 2]) -> Option<Event> {
    let (first, second) = (operands[0], operands[1]);
    Some(match status & 0xf0 {
        0x80 => Event::NoteOff {
            channel,
            key: first,
        },
        // A note on with no velocity is how a writer with a single port says
        // note off, and the spec says to read it that way.
        0x90 if second == 0 => Event::NoteOff {
            channel,
            key: first,
        },
        0x90 => Event::NoteOn {
            channel,
            key: first,
            velocity: second,
        },
        0xA0 | 0xD0 => return None,
        0xB0 => Event::Control {
            channel,
            number: first,
            value: second,
        },
        0xC0 => Event::Program {
            channel,
            number: first,
        },
        0xE0 => Event::PitchBend {
            channel,
            value: (u16::from(second) << 7) | u16::from(first),
        },
        _ => return None,
    })
}

fn push_event(raw: &mut Vec<RawEvent>, tick: u64, event: Event, limits: &Limits) -> Result<()> {
    if raw.len() >= limits.events {
        return Err(invalid(&format!(
            "SMF has more than {} events",
            limits.events
        )));
    }
    raw.push(RawEvent { tick, event });
    Ok(())
}

/// The timeline as a piecewise-linear map from ticks, one piece per tempo the
/// file stated, each carrying the tick it starts at, the time that tick is
/// already at, and the tempo held from there.
fn tempo_segments(tempos: &[(u64, u32)], ppq: u64) -> Vec<(u64, u64, u32)> {
    let mut sorted = tempos.to_vec();
    sorted.sort_by_key(|change| change.0);
    let mut segments = Vec::with_capacity(sorted.len() + 1);
    let mut tick = 0u64;
    let mut microsecond = 0u64;
    let mut tempo = DEFAULT_TEMPO_MICROSECONDS;
    for (at, next) in sorted {
        if at < tick {
            continue;
        }
        microsecond += (at - tick) * u64::from(tempo) / ppq;
        tick = at;
        tempo = next;
        segments.push((at, microsecond, next));
    }
    if segments.first().map_or(true, |segment| segment.0 != 0) {
        segments.insert(0, (0, 0, DEFAULT_TEMPO_MICROSECONDS));
    }
    segments
}

fn tick_to_microsecond(segments: &[(u64, u64, u32)], ppq: u64, tick: u64) -> u64 {
    let index = segments
        .partition_point(|segment| segment.0 <= tick)
        .saturating_sub(1);
    let (start_tick, start_us, tempo) = segments[index];
    // A segment's start is already in microseconds; what lies inside it is still
    // counted in ticks, and a tick is `tempo / ppq` of a microsecond.
    start_us + tick.saturating_sub(start_tick) * u64::from(tempo) / ppq
}

fn read_be_u16(bytes: &[u8], at: usize) -> Result<u16> {
    let pair = bytes
        .get(at..at + 2)
        .ok_or_else(|| invalid("SMF ends inside a header field"))?;
    Ok(u16::from_be_bytes([pair[0], pair[1]]))
}

fn read_be_u32(bytes: &[u8], at: usize) -> Result<u32> {
    let four = bytes
        .get(at..at + 4)
        .ok_or_else(|| invalid("SMF ends inside a header field"))?;
    Ok(u32::from_be_bytes([four[0], four[1], four[2], four[3]]))
}

fn read_byte(data: &[u8], at: &mut usize) -> Result<u8> {
    let byte = *data
        .get(*at)
        .ok_or_else(|| invalid("SMF event runs past the end of its track"))?;
    *at += 1;
    Ok(byte)
}

/// A variable-length quantity: seven bits per byte, the top bit saying another
/// follows. Four bytes is the whole 28-bit span the spec allows a delta.
fn read_vlq(data: &[u8], at: &mut usize) -> Result<u32> {
    let mut value: u32 = 0;
    for _ in 0..4 {
        let byte = read_byte(data, at)?;
        value = value
            .checked_shl(7)
            .and_then(|shifted| shifted.checked_add(u32::from(byte & 0x7f)))
            .ok_or_else(|| invalid("SMF variable-length number is too long to count"))?;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(invalid("SMF variable-length number runs past four bytes"))
}

/// Append one event record to a packet. The synthesiser's side of this format is
/// [`read_window`], and the two are pinned to each other by a test.
pub fn write_record(event: Event, offset_microseconds: u32, data: &mut Vec<u8>) {
    let (tag, channel, first, second, wide) = match event {
        Event::NoteOn {
            channel,
            key,
            velocity,
        } => (TAG_NOTE_ON, channel, key, velocity, 0),
        Event::NoteOff { channel, key } => (TAG_NOTE_OFF, channel, key, 0, 0),
        Event::Program { channel, number } => (TAG_PROGRAM, channel, number, 0, 0),
        Event::Control {
            channel,
            number,
            value,
        } => (TAG_CONTROL, channel, number, value, 0),
        Event::PitchBend { channel, value } => (TAG_PITCH_BEND, channel, 0, 0, value),
    };
    data.push(tag);
    data.extend_from_slice(&offset_microseconds.to_le_bytes());
    data.push(channel);
    data.extend_from_slice(&[first, second]);
    data.extend_from_slice(&wide.to_le_bytes());
}

/// Read a packet's records back: the decoder's side of the format above, which
/// refuses a packet that is not a whole number of records rather than reading
/// past its end.
pub fn read_window(data: &[u8]) -> Result<Vec<WindowEvent>> {
    if !data.len().is_multiple_of(RECORD_BYTES) {
        return Err(invalid(&format!(
            "MIDI packet is {} bytes, which is not a whole number of {RECORD_BYTES}-byte events",
            data.len()
        )));
    }
    let mut events = Vec::with_capacity(data.len() / RECORD_BYTES);
    for record in data.chunks_exact(RECORD_BYTES) {
        let offset = u32::from_le_bytes(record[1..5].try_into().unwrap());
        let channel = record[5];
        if channel > 15 {
            return Err(invalid(
                "MIDI packet names a channel the format does not have",
            ));
        }
        let wide = u16::from_le_bytes(record[8..10].try_into().unwrap());
        let event = match record[0] {
            TAG_NOTE_ON => Event::NoteOn {
                channel,
                key: record[6],
                velocity: record[7],
            },
            TAG_NOTE_OFF => Event::NoteOff {
                channel,
                key: record[6],
            },
            TAG_PROGRAM => Event::Program {
                channel,
                number: record[6],
            },
            TAG_CONTROL => Event::Control {
                channel,
                number: record[6],
                value: record[7],
            },
            TAG_PITCH_BEND => Event::PitchBend {
                channel,
                value: wide,
            },
            other => {
                return Err(invalid(&format!(
                    "MIDI packet carries event tag {other}, which this format does not define"
                )));
            }
        };
        events.push(WindowEvent {
            offset_microseconds: offset,
            event,
        });
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A header for `tracks` tracks at `ppq` ticks a quarter.
    fn header(format: u16, tracks: u16, ppq: u16) -> Vec<u8> {
        let mut bytes = b"MThd".to_vec();
        bytes.extend_from_slice(&6u32.to_be_bytes());
        bytes.extend_from_slice(&format.to_be_bytes());
        bytes.extend_from_slice(&tracks.to_be_bytes());
        bytes.extend_from_slice(&ppq.to_be_bytes());
        bytes
    }

    /// One `MTrk` chunk holding `events` as written.
    fn track(events: &[u8]) -> Vec<u8> {
        let mut bytes = b"MTrk".to_vec();
        bytes.extend_from_slice(&(events.len() as u32).to_be_bytes());
        bytes.extend_from_slice(events);
        bytes
    }

    /// An end-of-track `tick` after the previous event.
    fn end_of_track(tick: u32) -> Vec<u8> {
        let mut bytes = write_vlq(tick);
        bytes.extend_from_slice(&[0xFF, 0x2F, 0x00]);
        bytes
    }

    fn write_vlq(mut value: u32) -> Vec<u8> {
        let mut parts = vec![(value & 0x7f) as u8];
        value >>= 7;
        while value > 0 {
            parts.push(((value & 0x7f) as u8) | 0x80);
            value >>= 7;
        }
        parts.reverse();
        parts
    }

    fn tempo(tick: u32, microsecond: u32) -> Vec<u8> {
        let mut bytes = write_vlq(tick);
        bytes.extend_from_slice(&[0xFF, 0x51, 0x03]);
        // Three bytes big-endian, which is as wide as a tempo is stated.
        bytes.extend_from_slice(&microsecond.to_be_bytes()[1..]);
        bytes
    }

    fn note_on(tick: u32, key: u8, velocity: u8) -> Vec<u8> {
        let mut bytes = write_vlq(tick);
        bytes.extend_from_slice(&[0x90, key, velocity]);
        bytes
    }

    /// A one-note, one-bar file at 120 bpm: the note at the top of the bar and
    /// the track ending a quarter later.
    fn sample_file() -> Vec<u8> {
        let mut events = tempo(0, 500_000);
        events.extend_from_slice(&note_on(0, 60, 100));
        events.extend_from_slice(&end_of_track(480));
        [header(0, 1, 480), track(&events)].concat()
    }

    #[test]
    fn reads_a_single_track_file_and_its_events() {
        let file = SmfFile::parse(&sample_file(), &Limits::default()).expect("parses");
        assert_eq!(
            (file.format, file.ticks_per_quarter, file.track_count),
            (0, 480, 1)
        );
        assert_eq!(
            file.events,
            vec![TimedEvent {
                microseconds: 0,
                event: Event::NoteOn {
                    channel: 0,
                    key: 60,
                    velocity: 100
                },
            }]
        );
        // A quarter at 120 bpm is half a second, and that is where the track
        // stopped rather than where its note started.
        assert_eq!(file.duration(), Duration::from_millis(500));
    }

    #[test]
    fn delta_times_span_the_variable_length_range() {
        for (value, bytes) in [
            (0u32, vec![0x00]),
            (0x7f, vec![0x7f]),
            (0x80, vec![0x81, 0x00]),
            (0x3fff, vec![0xFF, 0x7F]),
            (0x1FFFFF, vec![0xFF, 0xFF, 0x7F]),
        ] {
            let mut at = 0usize;
            assert_eq!(read_vlq(&bytes, &mut at).expect("reads"), value);
            assert_eq!(at, bytes.len());
            assert_eq!(write_vlq(value), bytes);
        }
        // Five bytes would run past the 28 bits the spec gives a delta.
        let long = [0xFF, 0xFF, 0xFF, 0xFF, 0x7F];
        let mut at = 0usize;
        assert!(read_vlq(&long, &mut at).is_err());
    }

    #[test]
    fn running_status_carries_a_note_run_without_repeating_it() {
        let mut events = note_on(0, 60, 100);
        // No status byte on this one: the 0x90 above is still in force. Its two
        // delta bytes say 192 ticks, which is two quarters at 96 a quarter, and
        // the key and velocity follow as though the status had been repeated.
        events.extend_from_slice(&[0x81, 0x40, 0x40, 0x60]);
        events.extend_from_slice(&end_of_track(0));
        let bytes = [header(1, 1, 96), track(&events)].concat();
        let file = SmfFile::parse(&bytes, &Limits::default()).expect("parses");
        assert_eq!(
            file.events,
            vec![
                TimedEvent {
                    microseconds: 0,
                    event: Event::NoteOn {
                        channel: 0,
                        key: 60,
                        velocity: 100
                    },
                },
                TimedEvent {
                    microseconds: 1_000_000,
                    event: Event::NoteOn {
                        channel: 0,
                        key: 64,
                        velocity: 96
                    },
                },
            ]
        );
    }

    #[test]
    fn a_note_on_with_no_velocity_is_a_note_off() {
        let mut events = note_on(0, 60, 100);
        events.extend_from_slice(&note_on(10, 60, 0));
        events.extend_from_slice(&end_of_track(0));
        let bytes = [header(0, 1, 480), track(&events)].concat();
        let file = SmfFile::parse(&bytes, &Limits::default()).expect("parses");
        assert!(matches!(
            file.events[1].event,
            Event::NoteOff { key: 60, .. }
        ));
    }

    #[test]
    fn every_channel_and_meta_message_the_format_has_is_read_or_dropped() {
        let mut events = note_on(0, 60, 100);
        events.extend_from_slice(&[0, 0x80, 60, 0]); // note off
        events.extend_from_slice(&[0, 0xC2, 45]); // program on channel 2
        events.extend_from_slice(&[0, 0xB3, 7, 100]); // control on channel 3
        events.extend_from_slice(&[0, 0xE4, 0x00, 0x40]); // no bend on channel 4
        events.extend_from_slice(&[0, 0xD5, 64]); // channel aftertouch, dropped
        events.extend_from_slice(&[0, 0xA6, 64, 70]); // poly aftertouch, dropped
        events.extend_from_slice(&[0, 0xF2, 1, 2, 3]); // song position, stepped over
        events.extend_from_slice(&[0, 0xF3, 9]); // song select
        events.extend_from_slice(&[0, 0xF6]); // tune request, no operands
        events.extend_from_slice(&[0, 0xF0, 3, 1, 2, 3]); // a dump
        events.extend_from_slice(&[0, 0xFF, 0x01, 1, 65]); // text, dropped
        events.extend_from_slice(&end_of_track(0));
        let bytes = [header(0, 1, 480), track(&events)].concat();
        let file = SmfFile::parse(&bytes, &Limits::default()).expect("parses");
        assert_eq!(
            file.events
                .iter()
                .map(|event| event.event)
                .collect::<Vec<_>>(),
            vec![
                Event::NoteOn {
                    channel: 0,
                    key: 60,
                    velocity: 100
                },
                Event::NoteOff {
                    channel: 0,
                    key: 60
                },
                Event::Program {
                    channel: 2,
                    number: 45
                },
                Event::Control {
                    channel: 3,
                    number: 7,
                    value: 100
                },
                Event::PitchBend {
                    channel: 4,
                    value: 8192
                },
            ]
        );
    }

    #[test]
    fn tempo_changes_move_later_ticks_without_moving_earlier_ones() {
        let mut events = tempo(0, 500_000);
        events.extend_from_slice(&note_on(0, 60, 100));
        // One quarter at 120 bpm...
        events.extend_from_slice(&note_on(480, 61, 100));
        // ... then the file halves the rate...
        events.extend_from_slice(&tempo(0, 1_000_000));
        // ... so this quarter takes twice as long.
        events.extend_from_slice(&note_on(480, 62, 100));
        events.extend_from_slice(&end_of_track(0));
        let bytes = [header(0, 1, 480), track(&events)].concat();
        let file = SmfFile::parse(&bytes, &Limits::default()).expect("parses");
        assert_eq!(
            file.events
                .iter()
                .map(|event| event.microseconds)
                .collect::<Vec<_>>(),
            vec![0, 500_000, 1_500_000]
        );
    }

    #[test]
    fn a_file_without_a_tempo_states_the_default_one() {
        let mut events = note_on(0, 60, 100);
        events.extend_from_slice(&note_on(480, 62, 100));
        events.extend_from_slice(&end_of_track(0));
        let bytes = [header(0, 1, 480), track(&events)].concat();
        let file = SmfFile::parse(&bytes, &Limits::default()).expect("parses");
        assert_eq!(file.events[1].microseconds, 500_000);
    }

    #[test]
    fn tracks_of_a_format_one_file_share_the_timeline() {
        let mut conductor = tempo(0, 500_000);
        conductor.extend_from_slice(&end_of_track(960));
        let mut melody = note_on(0, 64, 100);
        melody.extend_from_slice(&end_of_track(480));
        let bytes = [header(1, 2, 480), track(&conductor), track(&melody)].concat();
        let file = SmfFile::parse(&bytes, &Limits::default()).expect("parses");
        assert_eq!(file.track_count, 2);
        assert_eq!(
            file.events,
            vec![TimedEvent {
                microseconds: 0,
                event: Event::NoteOn {
                    channel: 0,
                    key: 64,
                    velocity: 100
                },
            }]
        );
        // The conductor's bar, not the melody's note, is how long the listener
        // waits: both tracks start together and the longer one sets the length.
        assert_eq!(file.duration(), Duration::from_secs(1));
    }

    #[test]
    fn packet_records_survive_the_round_trip() {
        let mut events = tempo(0, 500_000);
        events.extend_from_slice(&note_on(0, 60, 100));
        events.extend_from_slice(&[0, 0xB0, 0x07, 0x64]);
        events.extend_from_slice(&[0, 0xE0, 0x00, 0x40]);
        events.extend_from_slice(&note_on(48, 60, 0));
        events.extend_from_slice(&[0, 0xC0, 0x05]);
        events.extend_from_slice(&end_of_track(0));
        let bytes = [header(0, 1, 480), track(&events)].concat();
        let file = SmfFile::parse(&bytes, &Limits::default()).expect("parses");
        let packet = file.window_packet(0);
        assert_eq!(packet.len(), 5 * RECORD_BYTES);
        assert_eq!(read_window(&packet).expect("reads back").len(), 5);
        assert_eq!(
            read_window(&packet).expect("reads back"),
            vec![
                WindowEvent {
                    offset_microseconds: 0,
                    event: Event::NoteOn {
                        channel: 0,
                        key: 60,
                        velocity: 100
                    },
                },
                WindowEvent {
                    offset_microseconds: 0,
                    event: Event::Control {
                        channel: 0,
                        number: 7,
                        value: 100
                    },
                },
                WindowEvent {
                    offset_microseconds: 0,
                    event: Event::PitchBend {
                        channel: 0,
                        value: 8192
                    },
                },
                WindowEvent {
                    offset_microseconds: 50_000,
                    event: Event::NoteOff {
                        channel: 0,
                        key: 60
                    },
                },
                WindowEvent {
                    offset_microseconds: 50_000,
                    event: Event::Program {
                        channel: 0,
                        number: 5
                    },
                },
            ]
        );
    }

    #[test]
    fn a_packet_ends_at_its_own_window() {
        let mut events = note_on(0, 60, 100);
        events.extend_from_slice(&note_on(960, 61, 100));
        events.extend_from_slice(&note_on(960, 62, 100));
        events.extend_from_slice(&end_of_track(0));
        let bytes = [header(0, 1, 480), track(&events)].concat();
        let file = SmfFile::parse(&bytes, &Limits::default()).expect("parses");
        // Each note waits two quarters of the default tempo, so a second between
        // them: each is alone in its packet and the nine packets between are
        // silent.
        assert_eq!(file.windows(), 22);
        let filled = (0..file.windows())
            .filter(|index| !file.window_packet(*index).is_empty())
            .collect::<Vec<_>>();
        assert_eq!(filled, vec![0, 10, 20]);
        for index in filled {
            assert_eq!(
                read_window(&file.window_packet(index)).expect("one").len(),
                1
            );
        }
        // The trailing packet is empty but exists, which is where a held note is
        // given back its release.
        assert!(file.window_packet(21).is_empty());
    }

    #[test]
    fn refuses_a_file_that_is_not_one() {
        assert!(SmfFile::parse(b"RIFF\x00\x00\x00\x00", &Limits::default()).is_err());
        assert!(SmfFile::parse(b"MThd", &Limits::default()).is_err());
        // A header that promises more than the file holds.
        assert!(
            SmfFile::parse(
                &b"MThd\x00\x00\x00\x40\x00\x00\x00\x01\x01\xe0".to_vec(),
                &Limits::default()
            )
            .is_err()
        );
        // A header short enough to state nothing.
        assert!(
            SmfFile::parse(
                &[b"MThd", 4u32.to_be_bytes().as_slice(), &[0, 0, 0, 1]].concat(),
                &Limits::default()
            )
            .is_err()
        );
    }

    #[test]
    fn refuses_a_timeline_it_cannot_place() {
        // The top bit of the division states SMPTE frames rather than quarters.
        let bytes = [header(0, 1, 0xE02D), track(&end_of_track(0))].concat();
        let error = SmfFile::parse(&bytes, &Limits::default()).unwrap_err();
        assert!(error.to_string().contains("SMPTE"), "{error}");
        // A division of no ticks at all places nothing anywhere.
        let bytes = [header(0, 1, 0), track(&end_of_track(0))].concat();
        assert!(SmfFile::parse(&bytes, &Limits::default()).is_err());
        // And a format number the spec never gave out.
        let mut bytes = header(9, 1, 480);
        bytes.extend_from_slice(&track(&end_of_track(0)));
        assert!(SmfFile::parse(&bytes, &Limits::default()).is_err());
    }

    #[test]
    fn refuses_a_track_that_is_not_one() {
        let mut bytes = header(0, 1, 480);
        bytes.extend_from_slice(b"XXXX");
        bytes.extend_from_slice(&0u32.to_be_bytes());
        let error = SmfFile::parse(&bytes, &Limits::default()).unwrap_err();
        assert!(error.to_string().contains("not a track"), "{error}");
        // The header the spec gives every file, but no track at all.
        assert!(SmfFile::parse(&header(0, 0, 480), &Limits::default()).is_err());
    }

    #[test]
    fn refuses_data_that_runs_past_the_file() {
        // A track header that promises more than the bytes left.
        let bytes = [
            header(0, 1, 480),
            b"MTrk".to_vec(),
            64u32.to_be_bytes().to_vec(),
            note_on(0, 60, 100),
        ]
        .concat();
        assert!(SmfFile::parse(&bytes, &Limits::default()).is_err());
        // An event cut off in the middle of its operands.
        let mut events = note_on(0, 60, 100);
        events.truncate(events.len() - 1);
        let bytes = [header(0, 1, 480), track(&events)].concat();
        assert!(SmfFile::parse(&bytes, &Limits::default()).is_err());
        // A meta event whose payload is longer than the track, and a tempo change
        // that is not a tempo.
        let bytes = [header(0, 1, 480), track(&[0x00, 0xFF, 0x01, 0x09])].concat();
        assert!(SmfFile::parse(&bytes, &Limits::default()).is_err());
        let bytes = [header(0, 1, 480), track(&[0x00, 0xFF, 0x51, 0x02, 1, 2])].concat();
        assert!(SmfFile::parse(&bytes, &Limits::default()).is_err());
        // A dump whose own length outruns its track.
        let bytes = [header(0, 1, 480), track(&[0x00, 0xF0, 0x40, 1, 2])].concat();
        assert!(SmfFile::parse(&bytes, &Limits::default()).is_err());
    }

    #[test]
    fn refuses_a_data_byte_with_no_status_to_run() {
        let bytes = [header(0, 1, 480), track(&[0x00, 0x40, 0x60])].concat();
        let error = SmfFile::parse(&bytes, &Limits::default()).unwrap_err();
        assert!(error.to_string().contains("no status"), "{error}");
    }

    #[test]
    fn holds_to_its_own_bounds() {
        let bytes = sample_file();
        let tight_file = Limits {
            file_bytes: bytes.len() - 1,
            ..Limits::default()
        };
        assert!(SmfFile::parse(&bytes, &tight_file).is_err());
        let pair = [
            header(1, 2, 480),
            track(&end_of_track(0)),
            track(&end_of_track(0)),
        ]
        .concat();
        let one_track = Limits {
            tracks: 1,
            ..Limits::default()
        };
        assert!(SmfFile::parse(&pair, &one_track).is_err());
        assert!(SmfFile::parse(&pair, &Limits::default()).is_ok());
        let no_events = Limits {
            events: 0,
            ..Limits::default()
        };
        assert!(SmfFile::parse(&bytes, &no_events).is_err());
        assert!(SmfFile::parse(&bytes, &Limits::default()).is_ok());
    }

    #[test]
    fn refuses_a_packet_that_is_not_whole_records() {
        assert!(read_window(&[TAG_NOTE_ON; RECORD_BYTES - 1]).is_err());
        // A well-shaped record naming a channel the format does not have.
        let mut record = vec![TAG_NOTE_ON, 0, 0, 0, 0, 16, 60, 100, 0, 0];
        assert!(read_window(&record).is_err());
        record[5] = 0;
        assert!(read_window(&record).is_ok());
        assert!(read_window(&[9, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }
}
