//! Audio playback of uncompressed PCM out of the header Sun and NeXT wrote.
//!
//! A `.snd` file states the geometry the player asks a container for in six
//! big-endian words and then holds the samples themselves, in exactly the form
//! [`crate::codec::pcm_decoder`] consumes, so as with [`crate::playback_wav`] and
//! [`crate::playback_aiff`] there is no codec setup to hand over and nothing to
//! render.
//!
//! AU file encoding identifiers are independent of audio-device encoding identifiers.
//! Codes 2..=5 are signed integer PCM, 6/7 are big-endian IEEE float, and
//! 1/27 are G.711 mu-law/A-law. Float byte order is passed to the shared native
//! decoder explicitly rather than inferred from the host architecture.

use crate::audio::{AudioStream, AudioTrack, EncodedPacket};
use crate::codec::pcm_decoder::PcmFormat;
use crate::playback_wav::{Coding, PACKET_FRAMES, Pcm};
use crate::{Result, invalid, unsupported};
use std::io::Read;
use std::time::Duration;

/// What a limit guards: how much the reader may take in, and how wide a track it
/// may agree to lay out.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Largest file accepted.
    pub file_bytes: usize,
    /// Largest sample run accepted, whether or not the header declared it.
    pub data_bytes: usize,
    /// Most channels one track may name.
    pub channels: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 128 << 20,
            data_bytes: 128 << 20,
            channels: 32,
        }
    }
}

/// AU file encoding for signed 16-bit big-endian PCM.
pub const LINEAR_16: u32 = 3;
/// AU file encoding for signed 24-bit big-endian PCM.
pub const LINEAR_24: u32 = 4;
/// AU file encoding for signed 32-bit big-endian PCM.
pub const LINEAR_32: u32 = 5;

/// The width an accepted `encoding` number states, and none for the numbers this
/// reader refuses.
fn linear_bits(encoding: u32) -> Option<u16> {
    match encoding {
        2 => Some(8),
        LINEAR_16 => Some(16),
        LINEAR_24 => Some(24),
        LINEAR_32 => Some(32),
        _ => None,
    }
}

/// Smallest header the format states: six words, no annotation.
const HEADER_BYTES: usize = 24;

/// A `.snd` file: its geometry and the bytes of its sample run.
#[derive(Clone, Debug)]
pub struct Au {
    pub pcm: Pcm,
    /// What the header's own data-length word claims, which a streamed file leaves
    /// at zero or at its maximum and which may simply overstate what the file holds.
    pub declared_bytes: usize,
    /// Frames the reader will hand over, a short tail included.
    pub frames: usize,
    data: Vec<u8>,
}

impl Au {
    /// Read a whole file: the header's six words, whatever annotation it chose to
    /// add, and the sample run they point at.
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Self> {
        if bytes.len() > limits.file_bytes {
            return Err(invalid(&format!(
                "file is over the {} byte limit",
                limits.file_bytes
            )));
        }
        if bytes.len() < HEADER_BYTES || &bytes[..4] != b".snd" {
            return Err(invalid("not a .snd file"));
        }
        let word =
            |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap_or([0xff; 4]));
        let start = usize::try_from(word(4)).unwrap_or(usize::MAX);
        let size_word = word(8);
        let encoding = word(12);
        let sample_rate = word(16);
        let channels = word(20);
        let (coding, bits) = match encoding {
            1 => (Coding::MuLaw, 8),
            27 => (Coding::ALaw, 8),
            6 => (Coding::Pcm(PcmFormat::Float { bits: 32 }), 32),
            7 => (Coding::Pcm(PcmFormat::Float { bits: 64 }), 64),
            _ => {
                let bits = linear_bits(encoding).ok_or_else(|| {
                    unsupported(&format!(
                        "AU encoding {encoding} is not supported PCM or G.711"
                    ))
                })?;
                (
                    Coding::Pcm(PcmFormat::Int {
                        bits: bits as u8,
                        big_endian: true,
                    }),
                    bits,
                )
            }
        };
        if sample_rate == 0 {
            return Err(invalid("sample rate of zero"));
        }
        if channels == 0 || usize::try_from(channels).unwrap_or(usize::MAX) > limits.channels {
            return Err(invalid("channel count is out of range"));
        }
        let channels = u16::try_from(channels)
            .map_err(|_| unsupported("AU channel count exceeds the audio interface range"))?;
        // The header says where the samples begin, and a file with an annotation says
        // so by moving them along; either way nothing before that word offset plays.
        if start < HEADER_BYTES {
            return Err(invalid("a header that points its data inside itself"));
        }
        let start = start.min(bytes.len());
        // A run of unknown length is written as zero or as the largest number the word
        // holds, and both mean "as much as the file carries".
        let declared = if size_word == 0 || size_word == u32::MAX {
            bytes.len() - start
        } else {
            usize::try_from(size_word).unwrap_or(usize::MAX)
        };
        let end = start.saturating_add(declared).min(bytes.len());
        let samples = &bytes[start..end];
        if samples.is_empty() {
            return Err(invalid("a .snd file with no samples in it"));
        }
        if samples.len() > limits.data_bytes {
            return Err(invalid(&format!(
                "sample run is over the {} byte limit",
                limits.data_bytes
            )));
        }
        let pcm = Pcm {
            coding,
            sample_rate,
            channels,
            bits_per_sample: bits,
        };
        if matches!(pcm.coding, Coding::Pcm(_) | Coding::ALaw | Coding::MuLaw)
            && !samples.len().is_multiple_of(pcm.block_align())
        {
            return Err(invalid("AU PCM sample run ends in an incomplete channel frame"));
        }
        Ok(Self {
            declared_bytes: declared,
            frames: samples.len() / pcm.block_align(),
            pcm,
            data: samples.to_vec(),
        })
    }

    /// Packets the sample run is read out as, the last a short one when the run does
    /// not divide evenly.
    pub fn packets(&self) -> usize {
        self.data
            .len()
            .div_ceil(PACKET_FRAMES * self.pcm.block_align())
    }

    /// One packet's bytes, aligned to a frame. A tail shorter than a frame holds no
    /// complete sample for every channel and is not handed over at all.
    pub fn packet(&self, index: usize) -> &[u8] {
        let width = PACKET_FRAMES * self.pcm.block_align();
        let start = index.min(self.packets().saturating_sub(1)) * width;
        let end = start.saturating_add(width).min(self.data.len());
        let whole = (end - start) / self.pcm.block_align() * self.pcm.block_align();
        &self.data[start..start + whole]
    }
}

/// A `.snd` file read as an audio track.
pub struct AuAudioReader {
    au: Au,
    packet: usize,
    frame_skip: usize,
    presentation_floor: u64,
}

impl AuAudioReader {
    /// Read a whole file. The sample run is one contiguous block and the packets are
    /// slices of it, so there is nothing here to produce incrementally.
    pub fn open<R: Read>(mut reader: R, limits: Limits) -> Result<Self> {
        let mut bytes = Vec::new();
        let cap = u64::try_from(limits.file_bytes).unwrap_or(u64::MAX);
        reader.by_ref().take(cap.saturating_add(1)).read_to_end(&mut bytes)?;
        Ok(Self {
            au: Au::parse(&bytes, &limits)?,
            packet: 0,
            frame_skip: 0,
            presentation_floor: 0,
        })
    }

    /// The file's own shape, for a caller that wants to say what it opened.
    pub fn au(&self) -> &Au {
        &self.au
    }
}

impl AudioStream for AuAudioReader {
    fn codec(&self) -> &str {
        match self.au.pcm.coding {
            Coding::Pcm(PcmFormat::Int { bits: 8, .. }) => "twos",
            Coding::Pcm(PcmFormat::Float { bits: 32 }) => "fl32",
            Coding::Pcm(PcmFormat::Float { bits: 64 }) => "fl64",
            _ => self.au.pcm.codec(),
        }
    }

    fn timescale(&self) -> u32 {
        self.au.pcm.sample_rate
    }

    fn sample_rate(&self) -> u32 {
        self.au.pcm.sample_rate
    }

    fn channels(&self) -> u16 {
        self.au.pcm.channels
    }

    fn bits_per_sample(&self) -> u16 {
        self.au.pcm.bits_per_sample
    }

    fn duration(&self) -> Option<Duration> {
        Some(crate::audio::sample_duration(self.au.frames as u64, self.au.pcm.sample_rate))
    }

    /// PCM needs no setup: the header's words have already gone into the decoder's
    /// construction.
    fn validate_eof(&self) -> Result<()> {
        let declared = self.au.declared_bytes;
        let available = self.au.data.len();
        if available < declared {
            return Err(invalid(&format!("AU data payload is truncated: declared {declared} bytes, available {available}")));
        }
        Ok(())
    }

    fn extra_data(&self) -> &[u8] {
        if matches!(self.au.pcm.coding, Coding::Pcm(PcmFormat::Float { .. })) {
            &[0] // Big-endian marker consumed by the shared native float decoder.
        } else {
            &[]
        }
    }

    fn audio_tracks(&self) -> Vec<AudioTrack> {
        vec![AudioTrack {
            sample_rate: self.au.pcm.sample_rate,
            channels: self.au.pcm.channels,
            name: String::new(),
            language: String::new(),
        }]
    }

    fn next_packet(&mut self) -> Result<Option<EncodedPacket>> {
        if self.packet >= self.au.packets() {
            return Ok(None);
        }
        let skip = self.frame_skip;
        let data = self.au.packet(self.packet)[skip * self.au.pcm.block_align()..].to_vec();
        self.frame_skip = 0;
        let packet = EncodedPacket {
            duration: (data.len() / self.au.pcm.block_align()) as i64,
            pts: (self.packet * PACKET_FRAMES + skip) as i64,
            data,
        };
        self.packet += 1;
        Ok(Some(packet))
    }

    fn present_decoded(
        &self,
        mut packet: crate::audio::AudioPacket,
        source_pts: i64,
    ) -> Result<Option<crate::audio::AudioPacket>> {
        let stride = usize::from(self.channels()) * 4;
        if stride == 0 || !packet.data.len().is_multiple_of(stride) {
            return Err(invalid("invalid decoded audio geometry"));
        }
        packet.pts = u64::try_from(source_pts)
            .map_err(|_| invalid("decoded audio precedes its source timeline"))?;
        packet.timebase_num = 1;
        packet.timebase_den = self.sample_rate();
        let frames = packet.data.len() / stride;
        let skip = self
            .presentation_floor
            .saturating_sub(packet.pts)
            .min(frames as u64) as usize;
        if skip == frames {
            return Ok(None);
        }
        if skip != 0 {
            packet.data.copy_within(skip * stride.., 0);
            packet.data.truncate((frames - skip) * stride);
            packet.pts += skip as u64;
        }
        Ok(Some(packet))
    }

    fn rewind(&mut self) {
        self.packet = 0;
        self.frame_skip = 0;
        self.presentation_floor = 0;
    }

    fn seek_to(&mut self, pts: i64) -> i64 {
        self.frame_skip = 0;
        self.presentation_floor = (pts.max(0) as u64).min(self.au.frames as u64);
        if pts.max(0) as u64 >= self.au.frames as u64 {
            self.packet = self.au.packets();
            return self.au.frames as i64;
        }
        if matches!(
            self.au.pcm.coding,
            Coding::ALaw
                | Coding::MuLaw
                | Coding::Pcm(_)
        ) {
            let target = (pts.max(0) as usize).min(self.au.frames);
            if target == self.au.frames {
                self.packet = self.au.packets();
            } else {
                self.packet = target / PACKET_FRAMES;
                self.frame_skip = target % PACKET_FRAMES;
            }
            return target as i64;
        }
        let last = self.au.packets().saturating_sub(1);
        self.packet = (pts.max(0) as usize / PACKET_FRAMES).min(last);
        (self.packet * PACKET_FRAMES) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::{Au, AuAudioReader, Limits, PACKET_FRAMES};
    use crate::audio::{AudioDecode, AudioStream, EncodedPacket};
    use crate::codec::pcm_decoder::PcmDecoder;
    use std::time::Duration;

    /// A header for the file its caller describes: the magic and the six words, with
    /// samples left to follow at the offset given them.
    fn header(data_start: u32, data_size: u32, encoding: u32, rate: u32, channels: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b".snd");
        for word in [data_start, data_size, encoding, rate, channels] {
            out.extend_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn au(samples: &[u8], channels: u32) -> Vec<u8> {
        let mut bytes = header(24, samples.len() as u32, 3, 48_000, channels);
        bytes.extend_from_slice(samples);
        bytes
    }

    fn be16(frames: &[i16]) -> Vec<u8> {
        frames.iter().flat_map(|s| s.to_be_bytes()).collect()
    }

    fn open(bytes: &[u8]) -> AuAudioReader {
        AuAudioReader::open(bytes, Limits::default()).expect("opens")
    }

    #[test]
    fn a_real_headers_words_are_read_as_the_muxer_meant_them() {
        // The 32-byte head of the `.snd` file FFmpeg wrote for the coverage gate's own
        // fixture - a header of 32 bytes rather than 24 because it left an annotation
        // word in - then four samples of the ramp its samples start with.
        let mut bytes = vec![
            0x2e, 0x73, 0x6e, 0x64, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x4b, 0x00, 0x00, 0x00,
            0x00, 0x03, 0x00, 0x00, 0xbb, 0x80, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];
        bytes.extend_from_slice(&be16(&[0, 0x00eb, 0x01d6, 0x02c0]));
        let reader = open(&bytes);
        assert_eq!(reader.codec(), "A_PCM/INT/BIG");
        assert_eq!((reader.sample_rate(), reader.channels()), (48_000, 1));
        assert_eq!(reader.bits_per_sample(), 16);
        assert!(reader.extra_data().is_empty());
        assert_eq!(reader.audio_tracks()[0].label(), "1 ch 48000 Hz");
        let au = reader.au();
        // The header promises 19 200 bytes and the file carries eight; eight is what
        // plays, and the promise stays visible to a caller that wants to say so.
        assert_eq!((au.declared_bytes, au.frames), (19_200, 4));
        assert_eq!(
            reader.duration(),
            Some(Duration::from_secs_f64(4.0 / 48_000.0))
        );
    }

    #[test]
    fn the_two_wider_linear_slots_are_read_as_the_muxer_wrote_them() {
        // The 32-byte heads of the 24-bit and 32-bit mono `.snd` files FFmpeg wrote,
        // each with the first four samples of its own run after it. The muxer fills a
        // wider word by left-aligning the 16-bit ramp it was given, so the same four
        // numbers come out of both widths - which is what lets one expected list serve
        // both.
        let cases: [(&[u8], &[u8], u32, u16); 2] = [
            (
                &[
                    0x2e, 0x73, 0x6e, 0x64, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x02, 0xd0, 0x00,
                    0x00, 0x00, 0x04, 0x00, 0x00, 0xbb, 0x80, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                ],
                &[
                    0x00, 0x00, 0x00, 0x00, 0xeb, 0x00, 0x01, 0xd6, 0x00, 0x02, 0xc0, 0x00,
                ],
                4,
                24,
            ),
            (
                &[
                    0x2e, 0x73, 0x6e, 0x64, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x03, 0xc0, 0x00,
                    0x00, 0x00, 0x05, 0x00, 0x00, 0xbb, 0x80, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                ],
                &[
                    0x00, 0x00, 0x00, 0x00, 0x00, 0xeb, 0x00, 0x00, 0x01, 0xd6, 0x00, 0x00, 0x02,
                    0xc0, 0x00, 0x00,
                ],
                5,
                32,
            ),
        ];
        for (head, data, encoding, bits) in cases {
            let mut bytes = head.to_vec();
            bytes.extend_from_slice(data);
            let mut reader = open(&bytes);
            assert_eq!(reader.codec(), "A_PCM/INT/BIG");
            assert_eq!(reader.bits_per_sample(), bits, "encoding {encoding}");
            assert_eq!(reader.au().frames, 4);
            let mut decoder = PcmDecoder::int(bits, true, reader.sample_rate(), reader.channels())
                .expect("the header's own geometry");
            let packet = reader.next_packet().expect("packet").expect("one packet");
            let audio = decoder
                .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                .expect("convert")
                .expect("PCM always yields a packet");
            // What FFmpeg's decoder prints for these same bytes.
            let expected = [0i32, 60_160, 120_320, 180_224]
                .iter()
                .map(|&scaled| scaled as f32 / (1 << 23) as f32)
                .collect::<Vec<_>>();
            assert_eq!(
                audio
                    .data
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                    .collect::<Vec<_>>(),
                expected,
                "encoding {encoding}"
            );
        }
    }

    #[test]
    fn the_samples_are_twos_complement_big_endian() {
        // The bytes across the zero crossing of the tone FFmpeg wrote: 129, -107, -342
        // in order, which is a sine and not the click a wrong byte order would make.
        let raw = [0x00, 0x00, 0x00, 0x81, 0xff, 0x95, 0xfe, 0xaa];
        let mut reader = open(&au(&raw, 1));
        let mut decoder = PcmDecoder::int(
            reader.bits_per_sample(),
            true,
            reader.sample_rate(),
            reader.channels(),
        )
        .expect("the header's own geometry");
        let packet = reader.next_packet().expect("packet").expect("one packet");
        let audio = decoder
            .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
            .expect("convert")
            .expect("PCM always yields a packet");
        let scale = 32_768.0f32;
        assert_eq!(
            audio
                .data
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                .collect::<Vec<_>>(),
            vec![0.0, 129.0 / scale, -107.0 / scale, -342.0 / scale]
        );
    }

    #[test]
    fn a_run_of_unknown_length_is_the_file_and_a_known_one_is_capped() {
        let samples = be16(&[1i16, 2, 3, 4]);
        for unknown in [0u32, u32::MAX] {
            let reader = open(&au_at(&samples, 24, unknown));
            assert_eq!(reader.au().frames, 4, "a header claiming {unknown} bytes");
        }
        // A length shorter than the run is the writer's own bound, and the reader keeps
        // to it rather than playing what the header did not count.
        let reader = open(&au_at(&samples, 24, 4));
        assert_eq!(reader.au().frames, 2);
        // And a length past the end of the file is taken as far as the file goes.
        let reader = open(&au_at(&samples, 24, 1 << 20));
        assert_eq!(
            (reader.au().declared_bytes, reader.au().frames),
            (1 << 20, 4)
        );
    }

    fn au_at(samples: &[u8], data_start: u32, data_size: u32) -> Vec<u8> {
        let mut bytes = header(data_start, data_size, 3, 48_000, 1);
        let start = usize::try_from(data_start)
            .unwrap_or(usize::MAX)
            .min(bytes.len() + 4096);
        if start > bytes.len() {
            // Whatever stands between the six words and the samples is annotation.
            bytes.resize(start, 0xa5);
        }
        bytes.extend_from_slice(samples);
        bytes
    }

    #[test]
    fn an_annotation_moves_where_the_samples_begin() {
        // The six words, then twenty bytes of annotation, then the samples: they start
        // after all of it, and the annotation itself is not audio.
        let bytes = au_at(&be16(&[7i16, -7]), 44, 4);
        let mut reader = open(&bytes);
        let packet = reader.next_packet().expect("packet").expect("one packet");
        assert_eq!(packet.data, be16(&[7, -7]));
        assert_eq!(reader.au().frames, 2);
    }

    #[test]
    fn a_second_stereo_frame_is_the_width_the_header_states() {
        let raw = be16(&[1i16, -1, 2, -2]);
        let mut reader = open(&au(&raw, 2));
        assert_eq!(reader.au().pcm.block_align(), 4);
        let packet = reader.next_packet().expect("packet").expect("one packet");
        assert_eq!(packet.duration, 2);
        assert_eq!(packet.data.len(), raw.len());
    }

    #[test]
    fn unsupported_au_codings_remain_refused() {
        let samples = be16(&[0i16; 8]);
        for encoding in [0, 8, 9, 10] {
            let bytes = {
                let mut bytes = header(24, samples.len() as u32, encoding, 48_000, 1);
                bytes.extend_from_slice(&samples);
                bytes
            };
            assert_eq!(
                Au::parse(&bytes, &Limits::default())
                    .err()
                    .map(|error| error.to_string()),
                Some(format!(
                    "AU encoding {encoding} is not supported PCM or G.711"
                )),
                "encoding {encoding}"
            );
        }
    }

    #[test]
    fn a_file_that_is_not_a_pcm_run_is_refused() {
        // Not a .snd file at all, and one too short to hold its six words.
        assert!(Au::parse(b"FORM\0\0\0\0AIFF", &Limits::default()).is_err());
        assert!(Au::parse(b".snd\0\0\0\0", &Limits::default()).is_err());
        // A header that points at samples inside itself, one that names no channels,
        // one that names a rate of zero, and one whose samples are missing.
        assert!(Au::parse(&au_at(&be16(&[0i16; 4]), 8, 8), &Limits::default()).is_err());
        assert!(Au::parse(&au(&be16(&[0i16; 4]), 0), &Limits::default()).is_err());
        assert!(
            Au::parse(
                &{
                    let mut bytes = header(24, 8, 3, 0, 1);
                    bytes.extend_from_slice(&be16(&[0i16; 4]));
                    bytes
                },
                &Limits::default()
            )
            .is_err()
        );
        assert!(Au::parse(&au_at(&[], 24, 0), &Limits::default()).is_err());
    }

    #[test]
    fn the_sample_run_is_cut_into_windows_and_the_cursor_lands_on_one() {
        let raw = vec![0u8; (PACKET_FRAMES * 2 + 2) * 2];
        let mut reader = open(&au(&raw, 1));
        let mut stamps = Vec::new();
        while let Some(EncodedPacket {
            pts,
            duration,
            data,
        }) = reader.next_packet().expect("packet")
        {
            stamps.push((pts, duration, data.len()));
        }
        assert_eq!(
            stamps,
            vec![(0, 2_048, 4_096), (2_048, 2_048, 4_096), (4_096, 2, 4)]
        );
        assert_eq!(reader.seek_to(2_500), 2_500);
        let suffix=reader.next_packet().unwrap().unwrap();assert_eq!((suffix.pts,suffix.duration),(2500,1596));
        assert_eq!(reader.seek_to(-1), 0);
        assert_eq!(reader.seek_to(999_999), 4098);
        assert!(reader.next_packet().expect("packet").is_none());
        assert!(reader.next_packet().expect("no more").is_none());
        reader.rewind();
        assert_eq!(reader.next_packet().expect("packet").expect("first").pts, 0);
    }
}
