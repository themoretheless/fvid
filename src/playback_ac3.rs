//! Audio playback of AC-3 out of the bare file the coding ships in.
//!
//! A `.ac3` file states nothing before its frames: each syncframe carries its own
//! header, and the geometry the player asks a container for - a codec tag, a rate,
//! a channel count, packets with timestamps - is read out of the frames
//! themselves, exactly as [`crate::playback_mp3`] does for MPEG audio. The tag
//! handed to [`crate::codec::make_audio_decoder`] is the one Matroska and MP4
//! already use for this coding, so the same decoder reads a track out of a
//! container and out of an elementary file.
//!
//! One measured fact shapes the walk beyond the header. A frame's length is not
//! stored in it as a count of bytes: the six-bit frame size code indexes
//! Section 5.18's table of 16-bit words, and the table's row depends on the
//! rate code, so the same code means a longer frame at 48 kHz than at 32 kHz.
//! Every frame of a stream decodes 1536 samples per channel whatever its rate,
//! which is what makes the timestamps a plain running count.
//!
//! The decoder is the work of [`crate::codec::ac3_decoder`], written from the
//! public ATSC A/52:2012 text; this file only lists frames. Nothing here reads
//! E-AC-3, whose syncframe starts with a different syncword and whose
//! [`crate::codec::ac3_decoder::syncframe`] probe refuses it by name.

use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
use crate::codec::ac3_decoder::{Syncframe, syncframe};
use crate::{Result, invalid};
use std::io::Read;
use std::time::Duration;

/// What a limit guards: how much the reader may take in, and how many frames it
/// may agree to list.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Largest file accepted.
    pub file_bytes: usize,
    /// Most frames one stream may hold.
    pub packets: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 128 << 20,
            packets: 1 << 20,
        }
    }
}

/// The tag [`crate::codec::make_audio_decoder`] answers for AC-3, as Matroska
/// names it; the MP4 fourcc `ac-3` reaches the same decoder.
pub const TAG: &str = "A_AC3";

/// Samples every syncframe decodes per channel, for all three rates: six blocks
/// of 256, which Section 5.4.1 fixes and Section 5.18's frame sizes follow.
const SAMPLES_PER_FRAME: u32 = 1536;

/// One frame of the stream: where it sits in the file, how long it is, and the
/// sample it starts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub start: usize,
    pub size: usize,
    pub pts: u64,
    /// The frame's own geometry, kept so a test can name what was read rather
    /// than a copy of the numbers the file states.
    pub rate: u32,
    pub channels: u16,
}

/// A `.ac3` file: the geometry its first frame states and the frames that play.
#[derive(Clone, Debug)]
pub struct Ac3 {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: Vec<Frame>,
    data: Vec<u8>,
}

impl Ac3 {
    /// Read a whole file: the run of frames, from wherever the first syncframe
    /// starts to wherever the last one ends.
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Self> {
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "file is over the {} byte limit",
                limits.file_bytes
            )));
        }
        // The first header may sit anywhere in front of it, since a file whose head
        // was chopped off still names its frames from there on. After the first
        // header the run must be contiguous: every frame starts where the last one
        // ended, with no gap to scan across. That is what a real AC-3 stream looks
        // like end to end, and what a file of some other shape that happens to hold
        // two bytes resembling a syncword does not.
        let mut found: Vec<Syncframe> = Vec::new();
        let mut starts: Vec<usize> = Vec::new();
        let mut pos = 0usize;
        while pos + 2 <= bytes.len() && syncframe(&bytes[pos..]).is_none() {
            pos += 1;
        }
        let first_start = pos;
        while let Some(at) = syncframe(bytes.get(pos..).unwrap_or_default()) {
            if pos + at.frame_bytes > bytes.len() {
                // A frame reaching past what the file holds is a truncated tail.
                break;
            }
            if found.len() >= limits.packets {
                return Err(invalid(&format!(
                    "stream is over the {} packet limit",
                    limits.packets
                )));
            }
            found.push(at);
            starts.push(pos);
            pos += at.frame_bytes;
        }
        if found.is_empty() {
            return Err(invalid("no AC-3 syncframes in the file"));
        }
        // A run that stops well short of the end of the file was never the file: a
        // stream of frames fills it, and anything else is another container whose
        // bytes this reader happened to make a syncframe out of.
        if (pos - first_start) * 10 < (bytes.len() - first_start) * 9 {
            return Err(invalid(&format!(
                "the frames run to byte {pos} of a file {} bytes long",
                bytes.len()
            )));
        }
        let first = found[0];
        let mut frames = Vec::with_capacity(found.len());
        let mut pts = 0u64;
        for (at, start) in found.iter().zip(&starts) {
            if at.sample_rate != first.sample_rate || at.channels != first.channels {
                return Err(invalid(&format!(
                    "stream changes geometry at frame {}: {} Hz {} ch after {} Hz {} ch",
                    frames.len(),
                    at.sample_rate,
                    at.channels,
                    first.sample_rate,
                    first.channels
                )));
            }
            frames.push(Frame {
                start: *start,
                size: at.frame_bytes,
                pts,
                rate: at.sample_rate,
                channels: at.channels,
            });
            pts += u64::from(SAMPLES_PER_FRAME);
        }
        Ok(Self {
            sample_rate: first.sample_rate,
            channels: first.channels,
            frames,
            data: bytes.to_vec(),
        })
    }

    /// Frames the stream hands over as packets.
    pub fn packets(&self) -> usize {
        self.frames.len()
    }

    /// One frame's bytes, header included, as the decoder expects them.
    pub fn packet(&self, index: usize) -> &[u8] {
        let at = self.frames[index.min(self.frames.len() - 1)];
        &self.data[at.start..at.start + at.size]
    }

    /// Samples the stream runs to, which is its length for a file that states no
    /// duration of its own.
    pub fn samples(&self) -> u64 {
        self.frames
            .last()
            .map(|at| at.pts + u64::from(SAMPLES_PER_FRAME))
            .unwrap_or(0)
    }
}

/// A `.ac3` file read as an audio track.
pub struct Ac3AudioReader {
    ac3: Ac3,
    packet: usize,
}

impl Ac3AudioReader {
    /// Read a whole file. The frames lie at lengths only their own headers state,
    /// so all of them are walked before the first one plays.
    pub fn open<R: Read>(mut reader: R, limits: Limits) -> Result<Self> {
        let mut bytes = Vec::new();
        let cap = u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader.by_ref().take(cap + 1).read_to_end(&mut bytes)?;
        Ok(Self {
            ac3: Ac3::parse(&bytes, &limits)?,
            packet: 0,
        })
    }

    /// The file's own shape, for a caller that wants to say what it opened.
    pub fn ac3(&self) -> &Ac3 {
        &self.ac3
    }
}

impl AudioStream for Ac3AudioReader {
    fn codec(&self) -> &str {
        TAG
    }

    /// Packets are stamped in samples, the same timescale the decoder reports.
    fn timescale(&self) -> u32 {
        self.ac3.sample_rate
    }

    fn sample_rate(&self) -> u32 {
        self.ac3.sample_rate
    }

    /// The frame's own native layout, which is the one the decoder answers with
    /// when a container states none: five full-bandwidth channels plus an LFE for
    /// a 5.1 stream, two for stereo, one for mono.
    fn channels(&self) -> u16 {
        self.ac3.channels
    }

    fn duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(
            self.ac3.samples() as f64 / f64::from(self.ac3.sample_rate),
        ))
    }

    /// A frame describes itself: this container stores no setup block.
    fn extra_data(&self) -> &[u8] {
        &[]
    }

    fn audio_tracks(&self) -> Vec<AudioTrack> {
        vec![AudioTrack {
            sample_rate: self.ac3.sample_rate,
            channels: self.ac3.channels,
            name: String::new(),
            language: String::new(),
        }]
    }

    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        let Some(at) = self.ac3.frames.get(self.packet) else {
            return Ok(None);
        };
        let packet = EncodedPacket {
            data: self.ac3.packet(self.packet).to_vec(),
            pts: at.pts as i64,
            duration: i64::from(SAMPLES_PER_FRAME),
        };
        self.packet += 1;
        Ok(Some(packet))
    }

    fn rewind(&mut self) {
        self.packet = 0;
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        let target = pts.max(0) as u64;
        let index = self
            .ac3
            .frames
            .partition_point(|at| at.pts <= target)
            .saturating_sub(1);
        self.packet = index;
        self.ac3.frames[index].pts as i64
    }
}

#[cfg(test)]
mod tests {
    use super::{Ac3, Ac3AudioReader, Limits, SAMPLES_PER_FRAME, Syncframe, TAG};
    use crate::audio::{AudioStream, EncodedPacket};
    use crate::codec::{ac3_decoder::syncframe, make_audio_decoder};

    /// The four files under `tests/fixtures/audio`, each an AC-3 elementary stream
    /// written by this machine's FFmpeg and covering the three rates and the channel
    /// layouts the coding reaches: 48 kHz stereo at 192 and 256 kbps (the second is
    /// the coupled frame the decoder's Section 7.3.4 work is measured on), 48 kHz
    /// 5.1 at 448 kbps, and mono at 32 kHz and 96 kbps. The frame lists asserted
    /// below are ffprobe's, packet for packet, and its reported bit rates and
    /// durations match the counts and lengths here: eight frames of 768, 1024 and
    /// 1792 bytes and six of 576, each file ending where its frames do. The signal
    /// inside them is a tone, but the encoder's input command was not written down
    /// when these files were cut, so nothing here claims a byte-for-byte
    /// reproduction of them - only the geometry they state is asserted.
    const STEREO: &[u8] = include_bytes!("../tests/fixtures/audio/ac3-stereo.ac3");
    const COUPLED: &[u8] = include_bytes!("../tests/fixtures/audio/ac3-coupled.ac3");
    const SURROUND: &[u8] = include_bytes!("../tests/fixtures/audio/ac3-51.ac3");
    const MONO_32K: &[u8] = include_bytes!("../tests/fixtures/audio/ac3-mono-32k.ac3");

    fn parse(bytes: &[u8]) -> Ac3 {
        Ac3::parse(bytes, &Limits::default()).expect("parses")
    }

    /// The frame walk is a run of lengths the headers state, so the whole-file
    /// assertions are about coverage: the frames must end where the file does.
    #[test]
    fn a_real_file_is_exactly_the_frames_its_header_counts() {
        for (bytes, rate, channels, count) in [
            (STEREO, 48_000, 2, 8),
            (COUPLED, 48_000, 2, 8),
            (SURROUND, 48_000, 6, 8),
            (MONO_32K, 32_000, 1, 6),
        ] {
            let ac3 = parse(bytes);
            assert_eq!(ac3.sample_rate, rate);
            assert_eq!(ac3.channels, channels);
            assert_eq!(ac3.packets(), count, "frames at {rate} Hz {channels} ch");
            assert_eq!(
                ac3.frames.last().map(|at| at.start + at.size),
                Some(bytes.len()),
                "the stream ends where the file does"
            );
            assert!(
                ac3.frames
                    .iter()
                    .all(|at| at.rate == rate && at.channels == channels)
            );
        }
    }

    /// Section 5.18 puts the same frame size code at different byte lengths for
    /// the three rates, and the walk trusts the table it is read out of.
    #[test]
    fn a_frames_length_comes_from_its_rate_and_size_code() {
        let stereo = parse(STEREO);
        assert_eq!(stereo.frames[0].size, 768, "192 kbps at 48000 Hz");
        let surround = parse(SURROUND);
        assert_eq!(surround.frames[0].size, 1792, "448 kbps at 48000 Hz");
        let mono = parse(MONO_32K);
        assert_eq!(mono.frames[0].size, 576, "96 kbps at 32000 Hz");
        // Every frame of the file states the same length, which is what makes the
        // walk a stride rather than a search.
        assert!(stereo.frames.iter().all(|at| at.size == 768));
    }

    #[test]
    fn timestamps_count_the_samples_a_frame_always_holds() {
        let ac3 = parse(SURROUND);
        for (index, at) in ac3.frames.iter().enumerate() {
            assert_eq!(at.pts, index as u64 * u64::from(SAMPLES_PER_FRAME));
        }
        assert_eq!(ac3.samples(), 8 * 1536);
        let reader = Ac3AudioReader::open(SURROUND, Limits::default()).expect("opens");
        assert_eq!(reader.timescale(), 48_000);
        assert_eq!(
            reader.duration(),
            Some(std::time::Duration::from_micros(256_000)),
            "eight frames of 1536 samples at 48000 Hz"
        );
    }

    /// The reader is only useful if the decoder accepts what it hands over, and
    /// the tag is the one the dispatch answers for.
    #[test]
    fn the_packets_it_hands_over_decode_as_the_tag_says() {
        let mut reader = Ac3AudioReader::open(STEREO, Limits::default()).expect("opens");
        assert_eq!(reader.codec(), TAG);
        assert_eq!((reader.sample_rate(), reader.channels()), (48_000, 2));
        assert_eq!(
            reader.bits_per_sample(),
            0,
            "a compressed stream says no width"
        );
        assert!(reader.extra_data().is_empty());
        assert_eq!(reader.audio_tracks()[0].label(), "2 ch 48000 Hz");
        let mut decoder =
            make_audio_decoder(TAG, &[], 48_000, 2, 0).expect("the dispatch answers this tag");
        let mut heard = 0usize;
        let mut stamps = Vec::new();
        while let Some(EncodedPacket {
            data,
            pts,
            duration,
        }) = reader.next_packet().expect("packet")
        {
            stamps.push((pts, duration));
            let audio = decoder
                .decode_encoded(&data, pts as u64, duration as u64)
                .expect("a frame of a real file decodes")
                .expect("and sounds");
            assert_eq!(
                audio.data.len() / 4,
                1536 * 2,
                "one frame of interleaved f32 per channel"
            );
            assert_eq!((audio.pts, audio.timebase_den), (pts as u64, 48_000));
            heard += audio.data.len() / 4;
        }
        assert_eq!(stamps.len(), reader.ac3().packets());
        assert_eq!(stamps[1], (1536, 1536), "pts is samples, not bytes");
        assert_eq!(heard, 8 * 1536 * 2, "every frame sounded its samples");
        reader.rewind();
        assert_eq!(reader.next_packet().expect("packet").expect("first").pts, 0);
    }

    #[test]
    fn a_seek_lands_on_the_frame_at_or_before_the_sample_asked_for() {
        let mut reader = Ac3AudioReader::open(SURROUND, Limits::default()).expect("opens");
        assert_eq!(reader.seek_to(4000), 3072, "the third frame starts at 3072");
        assert_eq!(reader.seek_to(3072), 3072, "a frame start stays there");
        assert_eq!(reader.seek_to(0), 0);
        assert_eq!(
            reader.seek_to(1 << 30),
            10_752,
            "past the end is the last frame"
        );
        let packet = reader.next_packet().expect("packet").expect("frame");
        assert_eq!(packet.pts, 10_752);
        // The last of the eight frames of 1792 bytes.
        assert_eq!(packet.data, &SURROUND[7 * 1792..]);
        reader.rewind();
        assert_eq!(reader.seek_to(-5), 0, "a negative sample is the start");
    }

    /// A file of another shape must be refused rather than half-read: the player
    /// offers every reader in turn, so a false accept costs the file its real
    /// codec. Both guards below were checked against the bytes of files this
    /// machine holds.
    #[test]
    fn a_file_that_is_not_a_run_of_syncframes_is_refused() {
        let mp3 = include_bytes!("../tests/fixtures/mp3/tone.mp3");
        let error = Ac3::parse(mp3, &Limits::default()).expect_err("an MP3 is not AC-3");
        assert!(error.to_string().contains("no AC-3 syncframes"), "{error}");
        // Bytes that name a syncword but run out mid-frame are a stream that stops
        // short of the file, and one frame of a real run is not a false accept: a
        // truncated tail is the file's own damage, not another container's.
        let mut chopped = STEREO.to_vec();
        chopped.truncate(3 * 768 + 100);
        assert_eq!(parse(&chopped).packets(), 3, "only the frames that fit");
        // A tail that reaches into a file of another shape is the case the coverage
        // guard is for: fill the rest with bytes no header survives.
        let mut padded = STEREO[..768].to_vec();
        padded.extend(vec![0u8; 4000]);
        let error = Ac3::parse(&padded, &Limits::default()).expect_err("padding is not audio");
        assert!(error.to_string().contains("run to byte"), "{error}");
        // The header probe reads the geometry out of the frame's own bytes, so the
        // walk never guesses a length it cannot find in the table.
        assert_eq!(
            syncframe(&STEREO[768..]),
            Some(Syncframe {
                sample_rate: 48_000,
                channels: 2,
                frame_bytes: 768,
            })
        );
        assert_eq!(syncframe(&[0x0b, 0x77, 0, 0]), None, "a stub is no header");
    }
}
