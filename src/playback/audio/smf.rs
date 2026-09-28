//! Audio playback of a Standard MIDI File through the player's own pipeline.
//!
//! A MIDI file is not a container the rest of this player can use: it states no
//! sample rate, no channel count and no track of encoded samples, because a
//! performance has none of those things. So this reader supplies the geometry the
//! audio thread asks for - 44.1 kHz, stereo, the player's own choice - and hands
//! over the file's events in slices of its timeline, which is what
//! [`crate::codec::midi_decoder`] turns into sound.

use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
use crate::container::smf::{self, SmfFile, WINDOW_MILLISECONDS, WINDOW_TIMESCALE};
use crate::{Result, invalid};
use std::io::Read;
use std::time::Duration;

/// The rate the synthesiser renders at. A performance states none of its own, so
/// this is the rate the player's output is built for.
pub const SAMPLE_RATE: u32 = 44_100;
/// One left and one right channel, which is what a stereo output takes.
pub const CHANNELS: u16 = 2;

/// A MIDI file read as an audio track: the parsed performance, and how far
/// through its timeline the cursor has got.
pub struct SmfAudioReader {
    file: SmfFile,
    window: usize,
}

impl SmfAudioReader {
    /// Read a whole file. A performance is small data and its own timeline needs
    /// to be complete before a single event can be placed on it, so there is no
    /// streaming half-read form of this.
    pub fn open<R: Read>(mut reader: R, limits: smf::Limits) -> Result<Self> {
        let mut bytes = Vec::new();
        let cap = u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader.by_ref().take(cap + 1).read_to_end(&mut bytes)?;
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "SMF is over the {} byte limit",
                limits.file_bytes
            )));
        }
        Ok(Self {
            file: SmfFile::parse(&bytes, &limits)?,
            window: 0,
        })
    }

    /// The file's own shape, for a caller that wants to say what it opened.
    pub fn file(&self) -> &SmfFile {
        &self.file
    }

    /// Packets this file is read out as, including the trailing silent one that
    /// lets a held key be heard releasing.
    pub fn packet_count(&self) -> usize {
        self.file.windows()
    }

    /// The window index a packet timestamp names, which is where a seek lands:
    /// the packet that covers the time asked for, never one before it.
    fn window_of(&self, pts: i64) -> usize {
        let millis = pts.max(0) as u64;
        let index = (millis / WINDOW_MILLISECONDS) as usize;
        index.min(self.packet_count() - 1)
    }
}

impl AudioStream for SmfAudioReader {
    fn codec(&self) -> &str {
        "midi"
    }

    fn timescale(&self) -> u32 {
        WINDOW_TIMESCALE
    }

    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    fn channels(&self) -> u16 {
        CHANNELS
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.file.duration())
    }

    fn extra_data(&self) -> &[u8] {
        &[]
    }

    fn audio_tracks(&self) -> Vec<AudioTrack> {
        vec![AudioTrack {
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            // One track to hear, named after the performance rather than a
            // channel of it.
            name: self.file.name.clone(),
            language: String::new(),
        }]
    }

    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        if self.window >= self.packet_count() {
            return Ok(None);
        }
        let packet = EncodedPacket {
            data: self.file.window_packet(self.window),
            pts: self.window as i64 * WINDOW_MILLISECONDS as i64,
            duration: WINDOW_MILLISECONDS as i64,
        };
        self.window += 1;
        Ok(Some(packet))
    }

    fn rewind(&mut self) {
        self.window = 0;
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        self.window = self.window_of(pts);
        self.window as i64 * WINDOW_MILLISECONDS as i64
    }
}

#[cfg(test)]
mod tests {
    use super::{CHANNELS, SAMPLE_RATE, SmfAudioReader};
    use crate::audio::{AudioDecode, AudioStream};
    use crate::codec::midi_decoder::MidiDecoder;
    use crate::container::smf;
    use std::time::Duration;

    /// A two-voice performance, written to spec by hand rather than by an
    /// encoder: format 1 at 480 ticks a quarter, a conductor track at 120 bpm
    /// that runs four quarters, and a melody track that strikes A4 for one of
    /// them, rests for one, strikes E5 for one and ends with the conductor.
    ///
    /// ```python
    /// def vlq(v): parts = [v & 0x7F]; v >>= 7
    ///   while v: parts.append((v & 0x7F) | 0x80); v >>= 7
    ///   return bytes(reversed(parts))
    /// conductor = vlq(0) + b"\xff\x51\x03" + (500_000).to_bytes(3, "big")
    /// conductor += vlq(0) + b"\xff\x03\x01\x41" + vlq(1920) + b"\xff\x2f\x00"
    /// melody = vlq(0) + b"\xff\x03\x06Melody" + vlq(0) + b"\xc0\x00"
    /// melody += vlq(0) + b"\x90\x45\x64" + vlq(480) + b"\x80\x45\x40"
    /// melody += vlq(480) + b"\x90\x4c\x64" + vlq(480) + b"\x80\x4c\x40"
    /// melody += vlq(480) + b"\xff\x2f\x00"
    /// MThd(1, 2, 480) + MTrk(conductor) + MTrk(melody)
    /// ```
    const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/midi/two-notes.mid");

    fn open() -> SmfAudioReader {
        SmfAudioReader::open(FIXTURE, smf::Limits::default()).expect("opens")
    }

    #[test]
    fn a_midi_file_supplies_the_geometry_it_does_not_state() {
        let reader = open();
        assert_eq!(reader.codec(), "midi");
        assert_eq!(reader.timescale(), 1_000);
        assert_eq!(
            (reader.sample_rate(), reader.channels()),
            (SAMPLE_RATE, CHANNELS)
        );
        assert_eq!(reader.extra_data(), &[] as &[u8]);
        // Four quarters at 120 bpm is two seconds, which is where the longer of
        // the two tracks stopped.
        assert_eq!(reader.duration(), Some(Duration::from_secs(2)));
        let tracks = reader.audio_tracks();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].name, "A");
        assert_eq!(
            tracks[0].label(),
            format!("A · {CHANNELS} ch {SAMPLE_RATE} Hz")
        );
    }

    #[test]
    fn packets_cover_the_timeline_and_one_more_window() {
        let mut reader = open();
        let mut pts = Vec::new();
        let mut bodies = Vec::new();
        while let Some(packet) = reader.next_packet().expect("packet") {
            pts.push(packet.pts);
            assert_eq!(packet.duration, 100);
            bodies.push(packet.data);
        }
        // Two seconds in 100 ms packets, plus the silent one that ends them.
        assert_eq!(pts.first(), Some(&0));
        assert_eq!(pts.last(), Some(&2100));
        assert_eq!(pts.len(), 22);
        // The notes are at the top of the timeline and a second in, so two of the
        // packets carry events and the rest of them are empty.
        let with_events = bodies
            .iter()
            .filter(|data| !data.is_empty())
            .map(|data| smf::read_window(data).expect("records"))
            .collect::<Vec<_>>();
        assert_eq!(with_events.len(), 4);
        // A4 struck, A4 lifted, E5 struck, E5 lifted, each in the window its own
        // tempo places it in, and the melody's own instrument change riding with
        // the first note.
        assert_eq!(
            with_events
                .iter()
                .map(|events| events.iter().map(|event| event.event).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            vec![
                vec![
                    smf::Event::Program {
                        channel: 0,
                        number: 0
                    },
                    smf::Event::NoteOn {
                        channel: 0,
                        key: 69,
                        velocity: 100
                    },
                ],
                vec![smf::Event::NoteOff {
                    channel: 0,
                    key: 69
                }],
                vec![smf::Event::NoteOn {
                    channel: 0,
                    key: 76,
                    velocity: 100
                }],
                vec![smf::Event::NoteOff {
                    channel: 0,
                    key: 76
                }],
            ]
        );
    }

    #[test]
    fn the_cursor_rewinds_and_lands_on_a_time() {
        let mut reader = open();
        for _ in 0..5 {
            reader.next_packet().expect("packet");
        }
        assert_eq!(reader.seek_to(1250), 1200);
        let packet = reader.next_packet().expect("packet").expect("a packet");
        // The seek landed on the packet that covers the time asked for, and that
        // is the one the cursor hands over next.
        assert_eq!(packet.pts, 1200);
        // Before the start, and past the end, are both answered with a packet that
        // exists rather than a cursor that has run off.
        assert_eq!(reader.seek_to(-10), 0);
        assert_eq!(reader.seek_to(99_999), 2100);
        assert!(
            reader
                .next_packet()
                .expect("packet")
                .expect("the last packet")
                .data
                .is_empty()
        );
        assert!(reader.next_packet().expect("no more").is_none());
        reader.rewind();
        assert_eq!(
            reader
                .next_packet()
                .expect("packet")
                .expect("first again")
                .pts,
            0
        );
    }

    #[test]
    fn the_whole_file_renders_at_the_rate_the_track_promises() {
        let mut reader = open();
        let mut decoder = MidiDecoder::new(reader.sample_rate(), reader.channels()).expect("synth");
        let mut frames = 0u64;
        let mut loudest = 0.0f32;
        let mut sounding = 0usize;
        while let Some(packet) = reader.next_packet().expect("packet") {
            let audio = decoder
                .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                .expect("renders")
                .expect("a packet of PCM");
            assert_eq!((audio.timebase_num, audio.timebase_den), (1, SAMPLE_RATE));
            assert_eq!(audio.data.len() / 4 / usize::from(CHANNELS), 4410);
            frames += (audio.data.len() / 4 / usize::from(CHANNELS)) as u64;
            let samples = audio
                .data
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                .collect::<Vec<_>>();
            let peak = samples
                .iter()
                .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
            loudest = loudest.max(peak);
            if peak > 0.0 {
                sounding += 1;
            }
        }
        // 22 windows of 4410 frames: two seconds of timeline and the release that
        // follows it.
        assert_eq!(frames, 22 * 4410);
        // Both notes sound and the two windows after them do not.
        assert_eq!(sounding, 12);
        assert!(loudest > 0.1 && loudest <= 1.0, "loudest frame {loudest}");
    }

    #[test]
    fn refuses_anything_that_is_not_a_performance() {
        assert!(SmfAudioReader::open(&b"RIFF...."[..], smf::Limits::default()).is_err());
        let limits = smf::Limits {
            file_bytes: FIXTURE.len() - 1,
            ..smf::Limits::default()
        };
        assert!(SmfAudioReader::open(FIXTURE, limits).is_err());
    }
}
