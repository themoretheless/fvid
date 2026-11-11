//! Owned PCM packet decoding; sample arithmetic shared with the frontend.
#![forbid(unsafe_code)]
pub use fvid_control::error::{Error, Result};
use fvid_control::error::invalid;
/// How the bytes of one sample are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmFormat {
    /// Unsigned 8-bit integer PCM, as specified by Matroska.
    Unsigned8,
    ALaw,
    MuLaw,
    /// Signed integer of `bits` bits per sample, packed with no padding.
    Int { bits: u8, big_endian: bool },
    /// IEEE float of 32 or 64 bits, little-endian.
    Float { bits: u8 },
}

impl PcmFormat {
    pub fn bits(self) -> u8 {
        match self {
            Self::Unsigned8 | Self::ALaw | Self::MuLaw => 8,
            Self::Int { bits, .. } | Self::Float { bits } => bits,
        }
    }

    /// Bytes of one channel's sample.
    fn sample_bytes(self) -> usize {
        usize::from(self.bits()) / 8
    }

    /// Widths this decoder reads. 8-bit integers are taken as signed, which is
    /// what QuickTime signed tags mean; the unsigned QuickTime variants
    /// name themselves differently and stay refused rather than playing noise.
    fn supported(self) -> bool {
        match self {
            Self::Unsigned8 | Self::ALaw | Self::MuLaw => true,
            Self::Int { bits, .. } => matches!(bits, 8 | 16 | 24 | 32),
            Self::Float { bits } => matches!(bits, 32 | 64),
        }
    }

    /// One sample widened into -1.0..=1.0.
    fn sample_f32(self, bytes: &[u8]) -> f32 {
        match self {
            Self::ALaw => f32::from(fvid_codecs::g711::Law::ALaw.decode(bytes[0]))/32768.,
            Self::MuLaw => f32::from(fvid_codecs::g711::Law::MuLaw.decode(bytes[0]))/32768.,
            Self::Unsigned8 => (f32::from(bytes[0]) - 128.0) / 128.0,
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
    float_big_endian: bool,
    sample_rate: u32,
    channels: u16,
}

impl PcmDecoder {
    /// Stored bytes in one complete interleaved source frame.
    pub fn encoded_frame_bytes(&self) -> Result<usize> {
        usize::from(self.channels).checked_mul(self.format.sample_bytes())
            .ok_or_else(||invalid("PCM source frame size overflow"))
    }
    pub fn new(format: PcmFormat, sample_rate: u32, channels: u16) -> Result<Self> {
        if !format.supported() {
            return Err(invalid(&format!(
                "unsupported {} PCM at {} bits",
                match format {
                    PcmFormat::ALaw | PcmFormat::MuLaw => "G.711",
                    PcmFormat::Unsigned8 | PcmFormat::Int { .. } => "integer",
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
            float_big_endian: false,
            sample_rate,
            channels,
        })
    }

    pub fn set_float_big_endian(&mut self, big: bool) { self.float_big_endian=big; }
    fn sample(&self, bytes:&[u8]) -> f32 {
        if self.float_big_endian {
            match self.format {
                PcmFormat::Float {bits:32} => return f32::from_be_bytes(bytes.try_into().unwrap()),
                PcmFormat::Float {bits:64} => return f64::from_be_bytes(bytes.try_into().unwrap()) as f32,
                _=>{},
            }
        }
        self.format.sample_f32(bytes)
    }

    /// Integer PCM named by width and byte order, as the containers describe it.
    /// A zero width is taken as 16-bit, the default both containers mean.
    pub fn int(bits: u16, big_endian: bool, sample_rate: u32, channels: u16) -> Result<Self> {
        let bits = if bits == 0 { 16 } else {
            u8::try_from(bits).map_err(|_| invalid(&format!("unsupported integer PCM at {bits} bits")))?
        };
        Self::new(PcmFormat::Int { bits, big_endian }, sample_rate, channels)
    }

    /// IEEE float PCM of 32 or 64 bits.
    pub fn float(bits: u16, sample_rate: u32, channels: u16) -> Result<Self> {
        let bits = if bits == 0 { 32 } else {
            u8::try_from(bits).map_err(|_| invalid(&format!("unsupported float PCM at {bits} bits")))?
        };
        Self::new(PcmFormat::Float { bits }, sample_rate, channels)
    }

    /// Widen complete finite channel frames into interleaved f32 bytes.
    /// Refuse malformed packets before publishing any samples to playback.
    pub fn convert(&self, data: &[u8]) -> Result<Vec<u8>> {
        let sample_bytes = self.format.sample_bytes();
        let frame = sample_bytes * usize::from(self.channels);
        if !data.len().is_multiple_of(frame) {
            return Err(invalid("PCM packet ends in an incomplete channel frame"));
        }
        let bytes = (data.len() / sample_bytes).checked_mul(4)
            .ok_or_else(|| invalid("decoded PCM size overflow"))?;
        let mut out = Vec::new();
        out.try_reserve_exact(bytes).map_err(|_| invalid("decoded PCM allocation failed"))?;
        for sample in data.chunks_exact(sample_bytes) {
            let sample = self.sample(sample);
            if !sample.is_finite() {
                return Err(invalid("PCM packet contains non-finite samples"));
            }
            out.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(out)
    }

    /// Strict packet conversion for container export: reject incomplete frames.
    pub fn decode_pcm(&self, data: &[u8]) -> Result<Vec<f32>> {
        let sample_bytes = self.format.sample_bytes();
        let frame = sample_bytes * usize::from(self.channels);
        if !data.len().is_multiple_of(frame) {
            return Err(invalid("PCM packet ends in an incomplete channel frame"));
        }
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(data.len() / sample_bytes)
            .map_err(|_| invalid("PCM output allocation failed"))?;
        for s in data.chunks_exact(sample_bytes) {
            let sample = self.sample(s);
            if !sample.is_finite() {
                return Err(invalid("PCM packet contains non-finite samples"));
            }
            samples.push(sample);
        }
        Ok(samples)
    }

    /// Preserve all stored integer bits and IEEE float64 precision in normalized f64.
    pub fn decode_pcm_f64(&self, data: &[u8]) -> Result<Vec<f64>> {
        let sample_bytes = self.format.sample_bytes();
        let frame = sample_bytes * usize::from(self.channels);
        if !data.len().is_multiple_of(frame) {
            return Err(invalid("PCM packet ends in an incomplete channel frame"));
        }
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(data.len() / sample_bytes)
            .map_err(|_| invalid("PCM output allocation failed"))?;
        for bytes in data.chunks_exact(sample_bytes) {
            let sample = match self.format {
                PcmFormat::ALaw => f64::from(fvid_codecs::g711::Law::ALaw.decode(bytes[0]))/32768.,
                PcmFormat::MuLaw => f64::from(fvid_codecs::g711::Law::MuLaw.decode(bytes[0]))/32768.,
                PcmFormat::Unsigned8 => (f64::from(bytes[0]) - 128.0) / 128.0,
                PcmFormat::Int { bits, big_endian } => f64::from(sign_extended(bytes, big_endian)) / (1u64 << (bits - 1)) as f64,
                PcmFormat::Float { bits: 32 } => f64::from(if self.float_big_endian { f32::from_be_bytes(bytes.try_into().unwrap()) } else { f32::from_le_bytes(bytes.try_into().unwrap()) }),
                PcmFormat::Float { bits: 64 } => if self.float_big_endian { f64::from_be_bytes(bytes.try_into().unwrap()) } else { f64::from_le_bytes(bytes.try_into().unwrap()) },
                PcmFormat::Float { .. } => unreachable!(),
            };
            if !sample.is_finite() {
                return Err(invalid("PCM packet contains non-finite samples"));
            }
            samples.push(sample);
        }
        Ok(samples)
    }

}
impl PcmDecoder {
    pub fn format(&self) -> PcmFormat {
        self.format
    }
    pub fn spec(&self) -> fvid_media_info::AudioSpec {
        fvid_media_info::AudioSpec {
            sample_rate: self.sample_rate,
            channels: self.channels,
            format: fvid_media_info::SampleFormat::F32,
        }
    }
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn channels(&self) -> u16 {
        self.channels
    }
    /// Validate Matroska PCM geometry and byte order before packet decoding.
    pub fn from_matroska(track: &crate::owned_webm::Track) -> Result<Self> {
        if track.kind != 2 {
            return Err(invalid("selected stream is not audio"));
        }
        let rate = u32::try_from(track.sample_rate)
            .map_err(|_| invalid("Matroska audio rate overflow"))?;
        let channels = u16::try_from(track.channels)
            .map_err(|_| invalid("Matroska audio channel count overflow"))?;
        if channels == 0 || channels > 64 {
            return Err(invalid("PCM channels must be 1..64"));
        }
        let bits = u8::try_from(track.bit_depth).map_err(|_| invalid("PCM bit depth overflow"))?;
        match track.codec.as_str() {
            "A_PCM/FLOAT/IEEE" => Self::float(u16::from(bits),rate,channels),
            "A_PCM/INT/LIT" | "A_PCM/INT/BIG" if bits == 8 => Self::new(PcmFormat::Unsigned8,rate,channels),
            "A_PCM/INT/LIT" | "A_PCM/INT/BIG" => Self::int(u16::from(bits),track.codec=="A_PCM/INT/BIG",rate,channels),
            _ => Err(invalid("selected Matroska audio stream is not PCM")),
        }
    }
}

impl PcmDecoder {
    /// Validate QuickTime PCM width and normalized `enda` byte order.
    pub(crate) fn from_mp4(track: &crate::owned_mp4::Track) -> Result<Self> {
        if track.handler != *b"soun" || !(1..=64).contains(&track.channels) {
            return Err(invalid("PCM requires an audio track with 1..64 channels"));
        }
        let format = match &track.codec {
            b"alaw" => PcmFormat::ALaw,
            b"ulaw" => PcmFormat::MuLaw,
            b"raw " if track.bit_depth == 8 => PcmFormat::Unsigned8,
            b"fl32" => PcmFormat::Float { bits: 32 },
            b"fl64" => PcmFormat::Float { bits: 64 },
            b"sowt" | b"twos" | b"in24" | b"in32" => PcmFormat::Int {
                bits: u8::try_from(track.bit_depth).map_err(|_| invalid("PCM bit depth overflow"))?,
                big_endian: track.codec == *b"twos"
                    || (matches!(&track.codec, b"in24" | b"in32")
                        && track.configuration.first() != Some(&1)),
            },
            _ => return Err(invalid("selected MP4 audio stream is not supported PCM")),
        };
        let mut decoder = Self::new(format, track.sample_rate, track.channels)?;
        decoder.set_float_big_endian(track.configuration.first() != Some(&1));
        Ok(decoder)
    }
}

#[cfg(test)]
mod precision_tests {
    use super::*;
    #[test]
    fn quicktime_float64_byte_orders_preserve_bits_beyond_f32() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/audio/pcm-screen.mov");
        let reader = crate::owned_mp4::Mp4Reader::open(
            std::fs::File::open(path).unwrap(),
            Default::default(),
        )
        .unwrap();
        let mut track = reader
            .tracks()
            .iter()
            .find(|t| t.handler == *b"soun")
            .unwrap()
            .clone();
        track.codec = *b"fl64";
        track.bit_depth = 64;
        track.channels = 1;
        let values = [0.123456789012345, -0.987654321012345, 2f64.powi(-40)];
        assert!(values.iter().any(|&x| f64::from(x as f32) != x));
        for little in [false, true] {
            track.configuration = vec![u8::from(little)];
            let packet: Vec<_> = values
                .iter()
                .flat_map(|x| {
                    if little {
                        x.to_le_bytes()
                    } else {
                        x.to_be_bytes()
                    }
                })
                .collect();
            let actual = PcmDecoder::from_mp4(&track)
                .unwrap()
                .decode_pcm_f64(&packet)
                .unwrap();
            assert_eq!(actual, values);
        }
    }
}

#[cfg(test)]
mod constructor_width_regressions {
    use super::PcmDecoder;
    #[test]
    fn public_width_constructors_keep_defaults_and_refuse_narrowing() {
        for bits in [0,8,16,24,32] {assert!(PcmDecoder::int(bits,false,8000,1).is_ok());}
        for bits in [0,32,64] {assert!(PcmDecoder::float(bits,8000,1).is_ok());}
        for bits in [256,264,272,280,288,512,u16::MAX] {
            let error=PcmDecoder::int(bits,false,8000,1).err().expect("unsupported integer width accepted");
            assert!(error.to_string().contains(&bits.to_string()));
        }
        for bits in [288,320,544,576,u16::MAX] {
            let error=PcmDecoder::float(bits,8000,1).err().expect("unsupported float width accepted");
            assert!(error.to_string().contains(&bits.to_string()));
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use super::{PcmDecoder, PcmFormat, sign_extended};

    fn decode(format: PcmFormat, channels: u16, bytes: &[u8]) -> Vec<f32> {
        let decoder = PcmDecoder::new(format, 48_000, channels).expect("decoder");
        decoder
            .convert(bytes)
            .expect("convert")
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| f32::from_le_bytes(*chunk))
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
            (&[0x00, 0x01, 0x00, 0x00][..], true, 65_536),
            (&[0x00, 0x00, 0x01, 0x00][..], false, 65_536),
        ] {
            assert_eq!(sign_extended(bytes, big), expected, "{bytes:?} big={big}");
        }
    }

    #[test]
    fn integers_of_every_width_land_on_the_same_scale() {
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

    #[test]
    fn a_partial_frame_is_refused_before_publication() {
        let decoder = PcmDecoder::int(16, false, 48000, 1).unwrap();
        let bytes = [0, 128, 64];
        let error = decoder.convert(&bytes).err().expect("partial frame accepted");
        assert!(error.to_string().contains("incomplete channel frame"));
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
                .format()
                .bits(),
            32
        );
    }
}
