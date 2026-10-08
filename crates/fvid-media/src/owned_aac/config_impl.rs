const AAC_RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AacConfig {
    /// MPEG-4 channel_configuration, zero for an explicit PCE.
    pub channel_configuration: u8,
    pub object_type: u32,
    pub sample_rate: u32,
    pub channels: u8,
    pub frame_samples: u16,
    pub core_coder_delay: Option<u16>,
}
fn audio_object_type(b: &mut BitReader<'_>) -> Result<u32> {
    let n = b.read(5)?;
    if n == 31 { Ok(32 + b.read(6)?) } else { Ok(n) }
}
impl AacConfig {
    /// Parse AAC-LC metadata, including the count from an explicit program.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let (config, _) = Self::parse_with_program(data)?;
        Ok(config)
    }
    /// Preserve an explicit tagged program rather than guessing a layout from its count.
    pub fn parse_with_program(
        data: &[u8],
    ) -> Result<(Self, Option<super::aac_pce::ProgramConfig>)> {
        let mut b = BitReader::new(data);
        let object_type = audio_object_type(&mut b)?;
        let index = b.read(4)? as usize;
        let sample_rate = if index == 15 {
            b.read(24)?
        } else {
            *AAC_RATES
                .get(index)
                .ok_or_else(|| invalid("reserved AAC frequency index"))?
        };
        if sample_rate == 0 {
            return Err(invalid("zero AAC frequency"));
        }
        let config = b.read(4)?;
        if object_type != 2 {
            return Err(invalid("only AAC-LC configuration is implemented"));
        }
        let mut channels = match config {
            0 => 0,
            1..=6 => config as u8,
            7 | 12 | 14 => 8,
            11 => 7,
            _ => return Err(invalid("unsupported AAC channel configuration")),
        };
        let frame_samples = if b.bit()? { 960 } else { 1024 };
        let core_coder_delay = if b.bit()? {
            Some(b.read(14)? as u16)
        } else {
            None
        };
        if b.bit()? {
            return Err(invalid("AAC extension flag is not yet supported"));
        }
        let program = if config == 0 {
            let program = super::aac_pce::ProgramConfig::read(&mut b, 0)?;
            if u32::from(program.object_type) != object_type || program.sample_rate != sample_rate {
                return Err(invalid("AAC PCE disagrees with AudioSpecificConfig"));
            }
            channels = program.channels() as u8;
            Some(program)
        } else {
            None
        };
        // Explicitly consume the common backward-compatible SBR sync extension.
        if b.remaining() >= 16 {
            if b.read(11)? != 0x2b7 {
                return Err(invalid("unsupported AAC trailing extension"));
            }
            if audio_object_type(&mut b)? != 5 || b.bit()? {
                return Err(invalid("AAC SBR decoding is not yet implemented"));
            }
        }
        while b.remaining() > 0 {
            if b.bit()? {
                return Err(invalid("nonzero AAC trailing bits"));
            }
        }
        Ok((
            Self {
                channel_configuration: config as u8,
                object_type,
                sample_rate,
                channels,
                frame_samples,
                core_coder_delay,
            },
            program,
        ))
    }
}
