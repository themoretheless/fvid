//! Audio playback of a tracker module through the player's own pipeline.
//!
//! A module states its own tempo, its own samples and its own cells, but not the
//! geometry a listener is fed: it names no output rate and no speaker layout, and
//! it holds no stream of encoded audio packets to hand over. So this reader
//! supplies both - 44.1 kHz, stereo, the player's own choice - and cuts the
//! performance into packets the way the player's clock counts it, one per row of
//! the timeline, which is what [`crate::codec::xm_decoder`] turns into sound.
//!
//! The packets are the rows themselves rather than sample data, so the decoder
//! behind them is stateful: it holds which channel is sounding what, and a row
//! only says what changed. That is why the reader hands over the whole file as
//! its setup data, and why a rewind or a seek resets the decoder with it.

use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
use crate::container::xm::{self, Module};
use crate::{Result, invalid};
use std::io::Read;
use std::time::Duration;

/// The rate the module is rendered at. A performance states none of its own, so
/// this is the rate the player's output is built for.
pub const SAMPLE_RATE: u32 = 44_100;
/// One left and one right channel. A module plays any number of channels into
/// them, so this is the layout of what is heard rather than a count of anything
/// in the file.
pub const CHANNELS: u16 = 2;

/// A module read as an audio track: the file's own bytes, what they parse to, and
/// how far through its timeline the cursor has got.
pub struct XmAudioReader {
    bytes: Vec<u8>,
    module: Module,
    packet: usize,
}

impl XmAudioReader {
    /// Read a whole file. The rows a module plays into are decided by its tempo
    /// column, which only the finished timeline can lay out, and the decoder
    /// behind these packets needs the samples too, so there is no streaming
    /// half-read form of this.
    pub fn open<R: Read>(mut reader: R, limits: xm::Limits) -> Result<Self> {
        let mut bytes = Vec::new();
        let cap = u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader.by_ref().take(cap + 1).read_to_end(&mut bytes)?;
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "module is over the {} byte limit",
                limits.file_bytes
            )));
        }
        Ok(Self {
            module: Module::parse(&bytes, &limits)?,
            bytes,
            packet: 0,
        })
    }

    /// The file's own shape, for a caller that wants to say what it opened.
    pub fn module(&self) -> &Module {
        &self.module
    }

    /// Packets this module is read out as: one per row of its timeline, plus the
    /// trailing one that lets a note still sounding at the end be heard letting
    /// go.
    pub fn packet_count(&self) -> usize {
        self.module.packets()
    }

    /// Where a packet starts, in the milliseconds its timestamps are in. The
    /// module's own microsecond starts are divided rather than its millisecond
    /// lengths added up, so a row whose length rounds twice in a row cannot walk
    /// the timeline away from the clock [`AudioStream::time_of`] reports.
    fn pts_of(&self, index: usize) -> i64 {
        let microseconds = self.module.packet_time(index).0;
        i64::try_from(microseconds / u64::from(xm::PACKET_TIMESCALE)).unwrap_or(i64::MAX)
    }

    /// How long a packet lasts: the distance to the row that follows it on the
    /// timeline, and for the trailing packet the length of the row it trails.
    fn duration_of(&self, index: usize) -> i64 {
        let next = index + 1;
        if next < self.packet_count() {
            self.pts_of(next) - self.pts_of(index)
        } else {
            let microseconds = self.module.packet_time(index).1;
            i64::try_from(microseconds / u64::from(xm::PACKET_TIMESCALE)).unwrap_or(0)
        }
    }
}

impl AudioStream for XmAudioReader {
    fn codec(&self) -> &str {
        "xm"
    }

    fn timescale(&self) -> u32 {
        xm::PACKET_TIMESCALE
    }

    fn sample_rate(&self) -> u32 {
        SAMPLE_RATE
    }

    fn channels(&self) -> u16 {
        CHANNELS
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.module.duration())
    }

    fn extra_data(&self) -> &[u8] {
        &self.bytes
    }

    fn audio_tracks(&self) -> Vec<AudioTrack> {
        vec![AudioTrack {
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS,
            // One track to hear, named after the module rather than a channel of
            // it. The tracker that wrote it goes into the name because two modules
            // of the same song carry the same title, and the channel count tells
            // them apart when nothing else does.
            name: format!("{} · {} ch", self.module.name, self.module.channels),
            language: String::new(),
        }]
    }

    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        if self.packet >= self.packet_count() {
            return Ok(None);
        }
        let packet = EncodedPacket {
            data: self.module.packet(self.packet),
            pts: self.pts_of(self.packet),
            duration: self.duration_of(self.packet),
        };
        self.packet += 1;
        Ok(Some(packet))
    }

    fn rewind(&mut self) {
        self.packet = 0;
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        self.packet = self
            .module
            .packet_at_microsecond(pts.max(0) as u64 * u64::from(xm::PACKET_TIMESCALE));
        self.pts_of(self.packet)
    }
}

#[cfg(test)]
mod tests {
    use super::{CHANNELS, SAMPLE_RATE, XmAudioReader};
    use crate::audio::{AudioDecode, AudioStream};
    use crate::codec::xm_decoder::XmDecoder;
    use crate::container::xm;
    use std::time::Duration;
    /// The committed fixture: FastTracker II's own defaults, one pattern of three
    /// rows over two channels - the two notes, a row that holds them, the key-off
    /// - and two instruments, a looping 16-bit sine and an 8-bit square that does
    /// not loop.
    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/xm/two-notes.xm");

    fn open() -> XmAudioReader {
        XmAudioReader::open(FIXTURE, xm::Limits::default()).expect("opens")
    }

    #[test]
    fn a_module_supplies_the_geometry_it_does_not_state() {
        let reader = open();
        assert_eq!(reader.codec(), "xm");
        assert_eq!(reader.timescale(), 1_000);
        assert_eq!(
            (reader.sample_rate(), reader.channels()),
            (SAMPLE_RATE, CHANNELS)
        );
        // Six ticks a row at 125 a minute is 120 ms, over three rows.
        assert_eq!(reader.duration(), Some(Duration::from_millis(360)));
        // The setup data is the whole file, because the renderer behind these
        // packets needs the samples the rows only name.
        assert_eq!(reader.extra_data(), FIXTURE);
        let tracks = reader.audio_tracks();
        assert_eq!(tracks.len(), 1);
        assert_eq!(
            tracks[0].label(),
            format!("fvid two notes · 2 ch · {CHANNELS} ch {SAMPLE_RATE} Hz")
        );
    }

    #[test]
    fn packets_are_the_rows_and_one_more() {
        let mut reader = open();
        let mut stamps = Vec::new();
        let mut bodies = Vec::new();
        while let Some(packet) = reader.next_packet().expect("packet") {
            stamps.push((packet.pts, packet.duration));
            bodies.push(packet.data);
        }
        // Three rows plus the tail, and the tail trails rather than adding time the
        // module does not state.
        assert_eq!(stamps, vec![(0, 120), (120, 120), (240, 120), (360, 120)]);
        assert_eq!(
            stamps.iter().map(|(pts, duration)| pts + duration).last(),
            Some(480)
        );
        // Every body is a row of the module's own width, so the decoder behind them
        // sees one cell per channel and never a part of one.
        let rows = bodies
            .iter()
            .map(|body| xm::read_row(body).expect("a whole row"))
            .collect::<Vec<_>>();
        assert_eq!(
            rows.iter()
                .map(|row| (row.bpm, row.speed, row.cells.len()))
                .collect::<Vec<_>>(),
            vec![(125, 6, 2); 4]
        );
        // The notes, then nothing, then the key-off, then the tail's nothing.
        assert_eq!(
            rows.iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .map(|cell| (cell.note, cell.instrument))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>(),
            vec![
                vec![(58, 1), (73, 1)],
                vec![(0, 0), (0, 0)],
                vec![(97, 0), (97, 0)],
                vec![(0, 0), (0, 0)],
            ]
        );
    }

    #[test]
    fn the_stamps_add_up_to_the_timeline_without_drift() {
        let reader = open();
        // A row's length is rounded from the tracker's own tick count, so adding the
        // millisecond lengths twice would drift by a millisecond over a long module.
        // The stamps come from the timeline's starts instead, and agree with it.
        let mut pts = 0i64;
        for index in 0..reader.packet_count() {
            assert_eq!(reader.pts_of(index), pts);
            pts += reader.duration_of(index);
        }
        assert_eq!(pts, 480, "three rows and the tail");
    }

    #[test]
    fn the_cursor_rewinds_and_lands_on_a_time() {
        let mut reader = open();
        for _ in 0..2 {
            reader.next_packet().expect("packet");
        }
        assert_eq!(reader.seek_to(250), 240);
        let packet = reader.next_packet().expect("packet").expect("a packet");
        // The seek landed on the row the time falls in, and that row is the one the
        // cursor hands over next.
        assert_eq!((packet.pts, packet.duration), (240, 120));
        assert_eq!(packet.data, reader.module().packet(2));
        // Before the start lands on the first row, and past the module's own end on
        // the tail that lets the last note go.
        assert_eq!(reader.seek_to(-10), 0);
        assert_eq!(reader.seek_to(9_999), 360);
        assert!(
            reader
                .next_packet()
                .expect("packet")
                .expect("the tail")
                .data
                .len()
                > xm::ROW_HEADER_BYTES
        );
        assert!(reader.next_packet().expect("no more").is_none());
        reader.rewind();
        assert_eq!(reader.next_packet().expect("packet").expect("first").pts, 0);
    }

    #[test]
    fn the_whole_module_renders_at_the_rate_the_track_promises() {
        let mut reader = open();
        let mut decoder =
            XmDecoder::new(reader.extra_data(), reader.sample_rate(), reader.channels())
                .expect("the fixture is playable");
        let mut frames = 0u64;
        let mut sounding = 0usize;
        let mut loudest = 0.0f32;
        while let Some(packet) = reader.next_packet().expect("packet") {
            let audio = decoder
                .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                .expect("renders")
                .expect("a packet of PCM");
            assert_eq!((audio.timebase_num, audio.timebase_den), (1, SAMPLE_RATE));
            // Every packet is as long as its row, so the decode thread's pacing
            // matches what the reader stamped.
            assert_eq!(
                audio.data.len() / 4 / usize::from(CHANNELS),
                usize::try_from(
                    u64::try_from(packet.duration).expect("positive") * u64::from(SAMPLE_RATE)
                        / 1_000
                )
                .expect("fits")
            );
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
            frames += (samples.len() / usize::from(CHANNELS)) as u64;
        }
        // 480 ms of timeline at 44.1 kHz, and the two rows the notes sound on are
        // the two that are heard; the key-off row lets go and the tail is silent.
        assert_eq!(frames, 4 * 5_292);
        assert_eq!(sounding, 3);
        assert!(loudest > 0.01 && loudest <= 1.0, "loudest frame {loudest}");
    }

    #[test]
    fn refuses_anything_that_is_not_a_module() {
        assert!(XmAudioReader::open(&b"RIFF...."[..], xm::Limits::default()).is_err());
        let limits = xm::Limits {
            file_bytes: FIXTURE.len() - 1,
            ..xm::Limits::default()
        };
        assert!(XmAudioReader::open(FIXTURE, limits).is_err());
    }
}
