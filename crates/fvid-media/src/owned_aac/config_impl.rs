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
    /// ER section syntax: five-bit books and implicit one-band escape sections.
    pub section_data_resilience: bool,
    /// ER reversible scalefactor coding and separately bounded escape words.
    pub scalefactor_data_resilience: bool,
    /// ER Huffman codeword reordering for spectral coefficients.
    pub spectral_data_resilience: bool,
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
        if parsed.separate_esc {return Err(invalid("AAC separate ESC epConfig requires the extension-aware decoder"));}
        if parsed.protection.is_some() {return Err(invalid("AAC error protection requires the extension-aware decoder"));}
        Ok((parsed.core, parsed.program))
    }
    fn read_core(
        b: &mut BitReader<'_>,
        object_type: u32,
        sample_rate: u32,
        config: u32,
    ) -> Result<(Self, Option<super::aac_pce::ProgramConfig>, Option<super::aac_eld_sbr::Config>, Option<super::aac_ep::Config>, bool, bool, Option<u32>)> {
        if object_type == 39 {
            let channels=match config {1..=6=>config as u8,7|12|14=>8,11=>7,_=>return Err(invalid("unsupported AAC ELD channel configuration"))};
            let frame_samples=if b.bit()?{480}else{512};
            let section_data_resilience=b.bit()?;
            let scalefactor_data_resilience=b.bit()?;
            let spectral_data_resilience=b.bit()?;
            let eld_sbr=if b.bit()? {let (value,used)=super::aac_eld_sbr::Config::read_prefix(b.source_bytes(),b.position(),config as u8)?;b.skip(used)?;Some(value)}else{None};
            let mut eld_downscale_sample_rate=None;
            loop {
                let kind=b.read(4)?;
                if kind==0 {break;}
                let mut bytes=b.read(4)? as usize;
                if bytes==15 {let add=b.read(8)? as usize;bytes+=add;if add==255 {bytes+=b.read(16)? as usize;}}
                // Reserved descriptors are length delimited. Spatial extensions
                // require a separate renderer; adaptation metadata is distinct.
                if matches!(kind,1|2) {return Err(invalid("AAC ELD extension tool is not yet implemented"));}
                if kind==3 {
                    // downscaleInfo is adaptation metadata. The default decoder
                    // retains the signaled full-rate transform/output; expose the
                    // hint separately rather than silently changing PCM clocks.
                    let count=bytes.checked_mul(8).ok_or_else(||invalid("AAC ELD extension length overflow"))?;
                    if count>b.remaining() {return Err(invalid("truncated AAC ELD extension payload"));}
                    let end=b.position().checked_add(count).ok_or_else(||invalid("AAC ELD extension length overflow"))?;
                    let mut payload=BitReader::with_bit_len(b.source_bytes(),end)?;
                    payload.skip(b.position())?;
                    let rate=sampling_frequency(&mut payload)?;
                    if payload.read(4)?!=0 {return Err(invalid("nonzero AAC ELD downscale fill nibble"));}
                    eld_downscale_sample_rate=Some(rate);
                }
                for _ in 0..bytes {b.read(8)?;}
            }
            let ep_config=b.read(2)?;
            let protection=match ep_config {
                0|1=>None,2|3=>{let (ep,used)=super::aac_ep::Config::read_prefix(b.source_bytes(),b.position())?;b.skip(used)?;Some(ep)},
                _=>return Err(invalid("AAC ELD epConfig separate ESC stream mapping is not yet implemented")),
            };
            let direct_mapping=ep_config==3;
            if direct_mapping && !b.bit()? {return Err(invalid("reserved AAC EP3 directMapping value"));}
            super::aac_ld_bands::LdBands::for_eld(sample_rate,frame_samples as usize).map_err(|e|invalid(&e.0))?;
            return Ok((Self{channel_configuration:config as u8,object_type,sample_rate,channels,frame_samples,core_coder_delay:None,section_data_resilience,scalefactor_data_resilience,spectral_data_resilience},None,eld_sbr,protection,direct_mapping,ep_config==1,eld_downscale_sample_rate));
        }
        if !matches!(object_type, 1 | 2 | 3 | 4 | 17 | 19 | 23) {
            return Err(invalid(
                "only AAC Main, LC, SSR, LTP, ER-LC, ER-LTP and ER-LD core configurations are implemented",
            ));
        }
        let mut channels = match config {
            0 => 0,
            1..=6 => config as u8,
            7 | 12 | 14 => 8,
            11 => 7,
            _ => return Err(invalid("unsupported AAC channel configuration")),
        };
        let mut frame_samples = if b.bit()? { 960 } else { 1024 };
        if object_type == 23 {
            frame_samples /= 2;
            super::aac_ld_bands::LdBands::new(sample_rate,frame_samples as usize).map_err(|e|invalid(&e.0))?;
        }
        if object_type == 3 && frame_samples != 1024 {
            return Err(invalid("AAC SSR requires frameLengthFlag zero"));
        }
        let core_coder_delay = if b.bit()? {
            Some(b.read(14)? as u16)
        } else {
            None
        };
        let extension_flag = b.bit()?;
        let program = if config == 0 {
            let mut program = super::aac_pce::ProgramConfig::read(b, 0)?;
            let matches_profile = match object_type {
                1..=4 => u32::from(program.object_type) == object_type,
                17 | 23 => program.object_type == 2,
                19 => matches!(program.object_type, 2 | 4),
                _ => false,
            };
            if !matches_profile || program.sample_rate != sample_rate {
                return Err(invalid("AAC PCE disagrees with AudioSpecificConfig"));
            }
            program.object_type = object_type as u8;
            channels = program.channels() as u8;
            Some(program)
        } else {
            None
        };
        // GASpecificConfig carries extensionFlag3 after any PCE. For LC/SSR
        // there are no ER/sub-frame fields before it; this future-use bit must
        // be zero. Do not mistake extensionFlag itself for unsupported audio.
        let (section_data_resilience, scalefactor_data_resilience, spectral_data_resilience) =
            if extension_flag && matches!(object_type,17|19|23) {
                (b.bit()?, b.bit()?, b.bit()?)
            } else { (false, false, false) };
        if extension_flag && b.bit()? {
            return Err(invalid("AAC extensionFlag3 must be zero"));
        }
        let mut direct_mapping = false;
        let mut separate_esc = false;
        let protection = if matches!(object_type,17|19|23) {
            match b.read(2)? {
                0 => None,
                1 => { separate_esc = true; None },
                // EP2 output reassembles the epConfig=0 access unit for
                // every ER core; entropy resilience remains core metadata.
                2 => {
                    let (ep, used) = super::aac_ep::Config::read_prefix(b.source_bytes(), b.position())?;
                    b.skip(used)?;
                    Some(ep)
                }
                3 => {
                    let (ep, used) = super::aac_ep::Config::read_prefix(b.source_bytes(), b.position())?;
                    b.skip(used)?;
                    if !b.bit()? {return Err(invalid("reserved AAC EP3 directMapping value"));}
                    direct_mapping = true;
                    Some(ep)
                }
                _ => return Err(invalid(if object_type == 23 {"ER AAC LD epConfig is not yet implemented"} else if object_type == 19 {"ER AAC LTP epConfig is not yet implemented"} else {"ER AAC LC epConfig is not yet implemented"})),
            }
        } else { None };
        Ok((
            Self {
                channel_configuration: config as u8,
                object_type,
                sample_rate,
                channels,
                frame_samples,
                core_coder_delay,
                section_data_resilience,
                scalefactor_data_resilience,
                spectral_data_resilience,
            },
            program,
            None,
            protection,
            direct_mapping,
            separate_esc,
            None,
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
    pub eld_sbr: Option<super::aac_eld_sbr::Config>,
    /// ELD extension-3 adaptation hint. Full-rate native reconstruction keeps
    /// the ASC core/SBR clock; optimized downscaled output is not implied.
    pub eld_downscale_sample_rate: Option<u32>,
    pub protection: Option<super::aac_ep::Config>,
    /// EP3 one-to-one EP-class/ESC-instance mapping; EP2 is false.
    pub direct_mapping: bool,
    /// EP1: separately transported AUs carrying consecutive ESC instances,
    /// optionally grouped within each AU.
    pub separate_esc: bool,
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
    /// Core-clock samples represented by one compressed packet. EP source
    /// concatenation changes this clock, not the core transform geometry.
    pub fn core_packet_samples(&self) -> u32 {
        u32::from(self.core.frame_samples) * u32::from(self.protection.as_ref().map_or(1, |ep| ep.frames))
    }
    pub fn output_sample_rate(&self) -> u32 {
        self.extension_sample_rate.unwrap_or(self.core.sample_rate)
    }
    /// Supported native ELD-core output adaptation, separate from LD-SBR.
    pub fn eld_core_downscale_factor(&self, declared:u32) -> Option<u8> {
        if self.eld_sbr.is_some() {None} else {self.eld_downscale_factor(declared)}
    }
    /// downscaleInfo describes the adapted core clock; LD-SBR may double it.
    pub fn eld_downscale_factor(&self, declared:u32) -> Option<u8> {
        let hint=self.eld_downscale_sample_rate?;
        let multiplier=if self.eld_sbr.as_ref().is_some_and(|s|s.double_rate){2}else{1};
        if self.core.object_type!=39 || hint==0 || hint.checked_mul(multiplier)!=Some(declared) || !self.core.sample_rate.is_multiple_of(hint) {return None;}
        match self.core.sample_rate/hint {2=>Some(2),3 if self.eld_sbr.is_none() && self.core.frame_samples.is_multiple_of(3)=>Some(3),4=>Some(4),8=>Some(8),_=>None}
    }
    /// AAC-LD has no ELD downscaleInfo: callers explicitly request a lower
    /// container/output clock, while entropy and prediction retain the source.
    pub fn ld_downscale_factor(&self,declared:u32)->Option<u8> {
        if self.core.object_type!=23 || declared==0 || self.sbr_present==Some(true) || self.ps_present==Some(true) || !self.core.sample_rate.is_multiple_of(declared) {return None;}
        let factor=self.core.sample_rate/declared;
        if matches!(factor,2|3|4|8) && u32::from(self.core.frame_samples).is_multiple_of(factor) {Some(factor as u8)}else{None}
    }
    pub fn output_downscale_factor(&self,declared:u32)->Option<u8> {
        self.eld_downscale_factor(declared).or_else(||self.ld_downscale_factor(declared))
    }
    /// Resolve explicit output clocks without guessing SBR or downscale modes.
    pub fn resolve_output_rate(&self, declared: u32) -> Result<u32> {
        if declared == self.output_sample_rate() || self.output_downscale_factor(declared).is_some() {
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
        let (core, program, eld_sbr, protection, direct_mapping, separate_esc, eld_downscale_sample_rate) =
            AacConfig::read_core(&mut b, core_object_type, sample_rate, configuration)?;
        if core.object_type == 39 && !explicit {
            // ELD's own ldSbrPresentFlag was zero. It does not permit the
            // ordinary implicit SBR/PS discovery route or a doubled clock.
            sbr_present=Some(eld_sbr.is_some());
            ps_present=Some(false);
            if let Some(ld)=&eld_sbr {extension_sample_rate=Some(ld.output_rate(core.sample_rate)?);}
        }
        if !explicit && eld_sbr.is_none() && b.remaining() >= 16 {
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
        if core.object_type == 39 && (explicit || sbr_present == Some(true) && eld_sbr.is_none() || ps_present == Some(true)) {
            return Err(invalid("AAC ELD requires low-delay SBR signalling"));
        }
        if core.object_type == 23 && (sbr_present == Some(true) || ps_present == Some(true)) {
            return Err(invalid(if core.object_type == 23 {"ER AAC LD SBR/PS is not yet implemented"} else if core.object_type == 19 {"ER AAC LTP SBR/PS is not yet implemented"} else {"ER AAC LC SBR/PS is not yet implemented"}));
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
            eld_sbr,
            eld_downscale_sample_rate,
            protection,
            direct_mapping,
            separate_esc,
        })
    }
}
