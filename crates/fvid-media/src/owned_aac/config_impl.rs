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
        let parsed = AudioSpecificConfig::parse(data)?;
        if parsed.sbr_present == Some(true) || parsed.ps_present == Some(true) {
            return Err(invalid(
                "AAC SBR configuration requires the extension-aware decoder",
            ));
        }
        Ok((parsed.core, parsed.program))
    }
    fn read_core(
        b: &mut BitReader<'_>,
        object_type: u32,
        sample_rate: u32,
        config: u32,
    ) -> Result<(Self, Option<super::aac_pce::ProgramConfig>)> {
        if !matches!(object_type, 2 | 3) {
            return Err(invalid("only AAC-LC and AAC-SSR core configurations are implemented"));
        }
        let mut channels = match config {
            0 => 0,
            1..=6 => config as u8,
            7 | 12 | 14 => 8,
            11 => 7,
            _ => return Err(invalid("unsupported AAC channel configuration")),
        };
        let frame_samples = if b.bit()? { 960 } else { 1024 };
        if object_type == 3 && frame_samples != 1024 {
            return Err(invalid("AAC SSR requires frameLengthFlag zero"));
        }
        let core_coder_delay = if b.bit()? {
            Some(b.read(14)? as u16)
        } else {
            None
        };
        if b.bit()? {
            return Err(invalid("AAC extension flag is not yet supported"));
        }
        let program = if config == 0 {
            let program = super::aac_pce::ProgramConfig::read(b, 0)?;
            if u32::from(program.object_type) != object_type || program.sample_rate != sample_rate {
                return Err(invalid("AAC PCE disagrees with AudioSpecificConfig"));
            }
            channels = program.channels() as u8;
            Some(program)
        } else {
            None
        };
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

/// Complete ASC metadata. `core` retains AAC clock/geometry; the extension
/// clock is separate so SBR does not change the core scale-factor band tables.
/// None preserves the standard's unspecified (-1) SBR/PS signalling state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioSpecificConfig {
    pub signaled_object_type: u32,
    pub core: AacConfig,
    pub program: Option<super::aac_pce::ProgramConfig>,
    pub sbr_present: Option<bool>,
    pub ps_present: Option<bool>,
    pub extension_sample_rate: Option<u32>,
}
fn sampling_frequency(b: &mut BitReader<'_>) -> Result<u32> {
    let index = b.read(4)? as usize;
    let rate = if index == 15 {
        b.read(24)?
    } else {
        *AAC_RATES
            .get(index)
            .ok_or_else(|| invalid("reserved AAC frequency index"))?
    };
    if rate == 0 {
        return Err(invalid("zero AAC frequency"));
    }
    Ok(rate)
}
impl AudioSpecificConfig {
    pub fn output_sample_rate(&self) -> u32 {
        self.extension_sample_rate.unwrap_or(self.core.sample_rate)
    }
    /// Resolve a container output clock without turning an explicit SBR=false
    /// into implicit signalling. Unspecified SBR may use double core rate.
    pub fn resolve_output_rate(&self, declared: u32) -> Result<u32> {
        if declared == self.output_sample_rate() {
            return Ok(declared);
        }
        if self.sbr_present.is_none() && self.core.sample_rate.checked_mul(2) == Some(declared) {
            return Ok(declared);
        }
        Err(invalid("AAC output clock disagrees with configuration"))
    }
    pub fn output_channels(&self) -> u8 {
        if self.ps_present == Some(true) {
            2
        } else {
            self.core.channels
        }
    }
    /// Parse LC core plus explicit AOT5/AOT29 or backward-compatible SBR/PS
    /// metadata. Recognizing PS metadata does not imply PS audio synthesis.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut b = BitReader::new(data);
        let signaled_object_type = audio_object_type(&mut b)?;
        let sample_rate = sampling_frequency(&mut b)?;
        let configuration = b.read(4)?;
        let explicit = matches!(signaled_object_type, 5 | 29);
        let mut sbr_present = explicit.then_some(true);
        let mut ps_present = (signaled_object_type == 29).then_some(true);
        let mut extension_sample_rate = None;
        let core_object_type = if explicit {
            extension_sample_rate = Some(sampling_frequency(&mut b)?);
            audio_object_type(&mut b)?
        } else {
            signaled_object_type
        };
        let (core, program) =
            AacConfig::read_core(&mut b, core_object_type, sample_rate, configuration)?;
        if !explicit && b.remaining() >= 16 {
            if b.read(11)? != 0x2b7 {
                return Err(invalid("unsupported AAC trailing extension"));
            }
            if audio_object_type(&mut b)? != 5 {
                return Err(invalid("unsupported AAC trailing extension object type"));
            }
            let present = b.bit()?;
            sbr_present = Some(present);
            if present {
                extension_sample_rate = Some(sampling_frequency(&mut b)?);
                if b.remaining() >= 12 {
                    if b.read(11)? != 0x548 {
                        return Err(invalid("unsupported AAC trailing PS extension"));
                    }
                    ps_present = Some(b.bit()?);
                }
            }
        }
        if ps_present == Some(true) && core.channels != 1 {
            return Err(invalid("AAC parametric stereo requires a mono core"));
        }
        while b.remaining() > 0 {
            if b.bit()? {
                return Err(invalid("nonzero AAC trailing bits"));
            }
        }
        Ok(Self {
            signaled_object_type,
            core,
            program,
            sbr_present,
            ps_present,
            extension_sample_rate,
        })
    }
}
