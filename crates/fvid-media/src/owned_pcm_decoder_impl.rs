/// How the bytes of one sample are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmFormat {
    /// Unsigned 8-bit integer PCM, as specified by Matroska.
    Unsigned8,
    /// Signed integer of `bits` bits per sample, packed with no padding.
    Int { bits: u8, big_endian: bool },
    /// IEEE float of 32 or 64 bits, little-endian.
    Float { bits: u8 },
}

impl PcmFormat {
    pub fn bits(self) -> u8 {
        match self {
            Self::Unsigned8 => 8,
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
            Self::Unsigned8 => true,
            Self::Int { bits, .. } => matches!(bits, 8 | 16 | 24 | 32),
            Self::Float { bits } => matches!(bits, 32 | 64),
        }
    }

    /// One sample widened into -1.0..=1.0.
    fn sample_f32(self, bytes: &[u8]) -> f32 {
        match self {
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
    pub fn new(format: PcmFormat, sample_rate: u32, channels: u16) -> Result<Self> {
        if !format.supported() {
            return Err(invalid(&format!(
                "unsupported {} PCM at {} bits",
                match format {
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
            data[..frames * frame].chunks_exact(self.format.sample_bytes())
                .flat_map(|sample| self.sample(sample).to_le_bytes()),
        );
        out
    }

    /// Strict packet conversion for container export: reject incomplete frames.
    pub fn decode_pcm(&self, data: &[u8]) -> Result<Vec<f32>> {
        let frame = self.format.sample_bytes() * usize::from(self.channels);
        if !data.len().is_multiple_of(frame) {
            return Err(invalid("PCM packet ends in an incomplete channel frame"));
        }
        let samples: Vec<f32> = data.chunks_exact(self.format.sample_bytes())
            .map(|s| self.sample(s)).collect();
        if samples.iter().any(|s| !s.is_finite()) {
            return Err(invalid("PCM packet contains non-finite samples"));
        }
        Ok(samples)
    }

}
