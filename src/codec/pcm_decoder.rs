//! Uncompressed PCM. The container states the sample width and byte order, so
//! decoding is a widening conversion into the interleaved f32 the pipeline
//! carries - no codec state, nothing to reset.
//!
//! Both containers name the same thing differently: ISO BMFF uses one fourcc per
//! byte order (`sowt` little, `twos` big) with the width in the sample entry,
//! while Matroska has `A_PCM/INT/LIT`, `A_PCM/INT/BIG` and `A_PCM/FLOAT/IEEE`
//! with the width in `BitDepth`.

use crate::audio::{AudioPacket, AudioSpec, SampleFormat};
use crate::{Result, invalid};

/// How the bytes of one sample are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmFormat {
    /// Signed integer of `bits` bits per sample, packed with no padding.
    Int { bits: u8, big_endian: bool },
    /// IEEE float of 32 or 64 bits, little-endian.
    Float { bits: u8 },
}

impl PcmFormat {
    pub fn bits(self) -> u8 {
        match self {
            Self::Int { bits, .. } | Self::Float { bits } => bits,
        }
    }

    /// Bytes of one channel's sample.
    fn sample_bytes(self) -> usize {
        usize::from(self.bits()) / 8
    }

    /// Widths this decoder reads. 8-bit integers are taken as signed, which is
    /// what both containers' integer tags mean; the unsigned QuickTime variants
    /// name themselves differently and stay refused rather than playing noise.
    fn supported(self) -> bool {
        match self {
            Self::Int { bits, .. } => matches!(bits, 8 | 16 | 24 | 32),
            Self::Float { bits } => matches!(bits, 32 | 64),
        }
    }

    /// One sample widened into -1.0..=1.0.
    fn sample_f32(self, bytes: &[u8]) -> f32 {
        match self {
            Self::Int { bits, big_endian } => {
                sign_extended(bytes, big_endian) as f32 / (1i64 << (i64::from(bits) - 1)) as f32
            }
            Self::Float { bits: 32 } => f32::from_le_bytes(bytes[..4].try_into().unwrap_or([0; 4])),
            Self::Float { bits: 64 } => {
                f64::from_le_bytes(bytes[..8].try_into().unwrap_or([0; 8])) as f32
            }
            Self::Float { .. } => 0.0,
        }
    }
}

/// Pack a 1..=4 byte sample into the top of a 32-bit word and arithmetic-shift it
/// back down, which both reads the value and extends its sign in either order.
fn sign_extended(bytes: &[u8], big_endian: bool) -> i32 {
    let mut word = [0u8; 4];
    let offset = 4 - bytes.len();
    for (i, byte) in bytes.iter().enumerate() {
        // Whichever end the container packed at, the sample's most significant
        // byte is the one that ends up at the top of the word.
        let at = offset + if big_endian { i } else { bytes.len() - 1 - i };
        word[at] = *byte;
    }
    let shift = 8 * offset;
    ((u32::from_be_bytes(word) << shift) as i32) >> shift
}

/// PCM decoder: converts container bytes into f32 packets.
pub struct PcmDecoder {
    format: PcmFormat,
    sample_rate: u32,
    channels: u16,
}

impl PcmDecoder {
    pub fn new(format: PcmFormat, sample_rate: u32, channels: u16) -> Result<Self> {
        if !format.supported() {
            return Err(invalid(&format!(
                "unsupported {} PCM at {} bits",
                match format {
                    PcmFormat::Int { .. } => "integer",
                    PcmFormat::Float { .. } => "float",
                },
                format.bits()
            )));
        }
        if channels == 0 || sample_rate == 0 {
            return Err(invalid("PCM track has no sample rate or channel count"));
        }
        Ok(Self {
            format,
            sample_rate,
            channels,
        })
    }

    /// Integer PCM named by width and byte order, as the containers describe it.
    /// A zero width is taken as 16-bit, the default both containers mean.
    pub fn int(bits: u16, big_endian: bool, sample_rate: u32, channels: u16) -> Result<Self> {
        let bits = if bits == 0 { 16 } else { bits as u8 };
        Self::new(PcmFormat::Int { bits, big_endian }, sample_rate, channels)
    }

    /// IEEE float PCM of 32 or 64 bits.
    pub fn float(bits: u16, sample_rate: u32, channels: u16) -> Result<Self> {
        let bits = if bits == 0 { 32 } else { bits as u8 };
        Self::new(PcmFormat::Float { bits }, sample_rate, channels)
    }

    /// Widen `data` into interleaved f32 little-endian bytes. A tail short of one
    /// frame is dropped: it holds no complete sample for every channel.
    fn convert(&self, data: &[u8]) -> Vec<u8> {
        let frame = self.format.sample_bytes() * usize::from(self.channels);
        let frames = data.len() / frame;
        let mut out = Vec::with_capacity(frames * usize::from(self.channels) * 4);
        out.extend(
            data.chunks_exact(self.format.sample_bytes())
                .flat_map(|sample| self.format.sample_f32(sample).to_le_bytes()),
        );
        out
    }

    pub fn spec(&self) -> AudioSpec {
        AudioSpec {
            sample_rate: self.sample_rate,
            channels: self.channels,
            format: SampleFormat::F32,
        }
    }
}

impl crate::audio::AudioDecode for PcmDecoder {
    /// Nothing is decoded: the packet's timestamp is the caller's and its bytes
    /// become output in the same order, so a packet always yields a packet.
    fn decode_encoded(
        &mut self,
        data: &[u8],
        pts: u64,
        _duration: u64,
    ) -> crate::Result<Option<AudioPacket>> {
        Ok(Some(AudioPacket {
            data: self.convert(data),
            pts,
            timebase_num: 1,
            timebase_den: self.sample_rate,
        }))
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::{PcmDecoder, PcmFormat, sign_extended};
    use crate::audio::AudioDecode;

    fn decode(format: PcmFormat, channels: u16, bytes: &[u8]) -> Vec<f32> {
        let mut decoder = PcmDecoder::new(format, 48_000, channels).expect("decoder");
        let packet = decoder
            .decode_encoded(bytes, 0, 0)
            .expect("convert")
            .expect("PCM always yields a packet");
        assert_eq!((packet.timebase_num, packet.timebase_den), (1, 48_000));
        packet
            .data
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
            .collect()
    }

    #[test]
    fn sign_extension_reads_both_byte_orders() {
        for (bytes, big, expected) in [
            (&[0x80][..], false, -128),
            (&[0x7f][..], false, 127),
            (&[0x01, 0x00][..], true, 256),
            (&[0xff, 0xff][..], true, -1),
            (&[0x80, 0x00, 0x00][..], true, -8_388_608),
            (&[0x00, 0x00, 0x80][..], false, -8_388_608),
            (&[0xff, 0xff, 0xff, 0xff][..], false, -1),
            // The same value written at either end.
            (&[0x00, 0x01, 0x00, 0x00][..], true, 65_536),
            (&[0x00, 0x00, 0x01, 0x00][..], false, 65_536),
        ] {
            assert_eq!(sign_extended(bytes, big), expected, "{bytes:?} big={big}");
        }
    }

    #[test]
    fn integers_of_every_width_land_on_the_same_scale() {
        // Full scale is the negative extreme, the way both containers hold it.
        assert_eq!(
            decode(
                PcmFormat::Int {
                    bits: 8,
                    big_endian: false
                },
                1,
                &[0x80, 0x7f, 0x00]
            ),
            vec![-1.0, 127.0 / 128.0, 0.0]
        );
        assert_eq!(
            decode(
                PcmFormat::Int {
                    bits: 16,
                    big_endian: false
                },
                1,
                &[0x00, 0x80, 0xff, 0x7f]
            ),
            vec![-1.0, 32767.0 / 32768.0]
        );
        assert_eq!(
            decode(
                PcmFormat::Int {
                    bits: 24,
                    big_endian: true
                },
                1,
                &[0x80, 0x00, 0x00, 0x7f, 0xff, 0xff]
            ),
            vec![-1.0, (8_388_607.0 / 8_388_608.0) as f32]
        );
        let full = decode(
            PcmFormat::Int {
                bits: 32,
                big_endian: true,
            },
            1,
            &[0x80, 0x00, 0x00, 0x00],
        );
        assert_eq!(full, vec![-1.0]);
    }

    #[test]
    fn a_24_bit_sample_keeps_its_value_across_the_wrap() {
        // One least significant byte of a 24-bit big-endian sample is 2^-23 of
        // full scale; a decoder that dropped a byte would lose the resolution.
        let one_step = decode(
            PcmFormat::Int {
                bits: 24,
                big_endian: true,
            },
            1,
            &[0x00, 0x00, 0x01],
        );
        assert!(one_step[0] > 1.0e-7 && one_step[0] < 1.2e-7, "{one_step:?}");
    }

    #[test]
    fn interleaved_channels_stay_interleaved() {
        // Stereo little-endian 16-bit: near-full-scale, -1, 0, -1 in channel order.
        let bytes: Vec<u8> = vec![0xff, 0x7f, 0x00, 0x80, 0x00, 0x00, 0xff, 0xff];
        let out = decode(
            PcmFormat::Int {
                bits: 16,
                big_endian: false,
            },
            2,
            &bytes,
        );
        assert_eq!(out.len(), 4);
        assert_eq!(&out[..2], &[32767.0 / 32768.0, -1.0]);
        assert_eq!(&out[2..], &[0.0, -1.0 / 32768.0]);
    }

    #[test]
    fn floats_pass_through_in_both_widths() {
        let mut bytes = 0.5f32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&(-0.25f32).to_le_bytes());
        assert_eq!(
            decode(PcmFormat::Float { bits: 32 }, 2, &bytes),
            vec![0.5, -0.25]
        );
        let mut wide = 0.75f64.to_le_bytes().to_vec();
        wide.extend_from_slice(&0.25f64.to_le_bytes());
        assert_eq!(
            decode(PcmFormat::Float { bits: 64 }, 2, &wide),
            vec![0.75, 0.25]
        );
    }

    /// A frame cannot be split: half a stereo sample would shift every later
    /// channel into the wrong place, so the leftover byte is dropped.
    #[test]
    fn a_partial_frame_is_dropped_not_shifted() {
        let mut bytes = (-32768i16).to_le_bytes().to_vec();
        bytes.extend_from_slice(&0x40u8.to_le_bytes());
        let out = decode(
            PcmFormat::Int {
                bits: 16,
                big_endian: false,
            },
            1,
            &bytes,
        );
        assert_eq!(out, vec![-1.0]);
    }

    #[test]
    fn widths_with_no_definition_are_refused_at_open() {
        for format in [
            PcmFormat::Int {
                bits: 12,
                big_endian: false,
            },
            PcmFormat::Float { bits: 16 },
            PcmFormat::Int {
                bits: 0,
                big_endian: false,
            },
        ] {
            let error = PcmDecoder::new(format, 48_000, 2)
                .err()
                .expect("unsupported width");
            assert!(error.to_string().contains("unsupported"), "{error}");
        }
        assert!(
            PcmDecoder::new(
                PcmFormat::Int {
                    bits: 16,
                    big_endian: false
                },
                48_000,
                0
            )
            .is_err()
        );
    }

    #[test]
    fn a_missing_width_falls_back_to_the_containers_default() {
        let decoder = PcmDecoder::int(0, false, 44_100, 1).expect("16 bit default");
        assert_eq!(decoder.spec().sample_rate, 44_100);
        assert_eq!(
            PcmDecoder::float(0, 44_100, 1)
                .expect("32 bit float default")
                .format
                .bits(),
            32
        );
    }

    /// The two containers divide the description differently: an ISO BMFF fourcc
    /// names the byte order and leaves the width to the sample entry, a Matroska
    /// codec ID names the order and leaves the width to `BitDepth`. Each ID has
    /// to reach the layout its own partner supplies, and `fl32`/`fl64` settle the
    /// width in the name so that an entry stating something else cannot misread
    /// the stream.
    #[test]
    fn every_container_name_reaches_its_own_layout() {
        for (codec, bits, bytes, expected) in [
            ("sowt", 16u16, &[0x00, 0x80][..], -1.0),
            ("A_PCM/INT/LIT", 16, &[0x00, 0x80], -1.0),
            ("twos", 16, &[0x80, 0x00], -1.0),
            ("A_PCM/INT/BIG", 24, &[0x80, 0x00, 0x00], -1.0),
            ("A_PCM/INT/LIT", 0, &[0xff, 0x7f], 32_767.0 / 32_768.0),
            ("fl32", 16, &[0x00, 0x00, 0x00, 0xbf], -0.5),
            (
                "A_PCM/FLOAT/IEEE",
                64,
                &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xd0, 0x3f],
                0.25,
            ),
            (
                "fl64",
                0,
                &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xd0, 0x3f],
                0.25,
            ),
        ] {
            let mut decoder = crate::codec::make_audio_decoder(codec, &[], 48_000, 1, bits)
                .unwrap_or_else(|error| panic!("{codec}: {error}"));
            let packet = decoder
                .decode_encoded(bytes, 0, 0)
                .expect("convert")
                .expect("PCM always yields a packet");
            let samples: Vec<f32> = packet
                .data
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                .collect();
            assert_eq!(samples, vec![expected], "{codec} at {bits} bits");
        }
    }
}
