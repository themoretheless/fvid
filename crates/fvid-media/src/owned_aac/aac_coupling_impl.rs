use super::{aac_channel::ChannelData, bits::BitReader, config::AacConfig};
pub struct Target {
    pub pair: bool,
    pub tag: u8,
    pub channel: u8,
    pub gain: f32,
    pub bands: Vec<Vec<f32>>,
}
pub struct Coupling {
    pub tag: u8,
    /// 0 before target TNS, 1 after target TNS, 3 after IMDCT.
    pub point: u8,
    pub channel: ChannelData,
    pub targets: Vec<Target>,
}
impl Coupling {
    pub fn read(bits: &mut BitReader<'_>, config: &AacConfig) -> Result<Self> {
        Self::read_payload(bits, config, false).map(|(coupling, _)| coupling)
    }
    /// Ordinary AOT4 CCE, preserving prediction data separately from gains.
    /// Whole-element transactional parsing; state/profile admission is separate.
    pub fn read_ltp(bits: &mut BitReader<'_>, config: &AacConfig) -> Result<(Self, Option<super::aac_ltp_syntax::LtpData>)> {
        if config.object_type != 4 { return Err(invalid("AAC LTP coupling requires AOT4")); }
        Self::read_payload(bits, config, true)
    }
    fn read_payload(bits: &mut BitReader<'_>, config: &AacConfig, ltp: bool) -> Result<(Self, Option<super::aac_ltp_syntax::LtpData>)> {
        let mut input = bits.clone();
        let tag = input.read(4)? as u8;
        let independent = input.bit()?;
        let count = input.read(3)? + 1;
        let mut selected = Vec::new();
        for _ in 0..count {
            let pair = input.bit()?;
            let target = input.read(4)? as u8;
            let selection = if pair { input.read(2)? as u8 } else { 2 };
            selected.push((pair, target, selection));
        }
        let after = input.bit()?;
        let point = if independent { 3 } else { u8::from(after) };
        let sign = input.bit()?;
        let scale = input.read(2)?;
        let (channel, prediction) = if ltp {
            ChannelData::read_ltp(&mut input, config)?
        } else { (ChannelData::read(&mut input, config)?, None) };
        let mut targets = Vec::new();
        let mut gain_index = 0;
        for (pair, tag, selection) in selected {
            let lists = if selection == 3 { 2 } else { 1 };
            for list in 0..lists {
                let common = gain_index == 0 || independent || input.bit()?;
                let mut delta = if gain_index != 0 && common {
                    super::aac_huffman::scalefactor(&mut input)? as i32
                } else {
                    0
                };
                let factor = [0.125, 0.25, 0.5, 1.0][scale as usize];
                let mut gain = 2f32.powf(-(delta as f32) * factor);
                let mut bands = Vec::new();
                if !independent {
                    for books in &channel.codebooks {
                        let mut values = Vec::new();
                        for &book in books {
                            if book != 0 && !common {
                                let change = super::aac_huffman::scalefactor(&mut input)? as i32;
                                if change != 0 {
                                    delta += change;
                                    let exponent = if sign { delta >> 1 } else { delta };
                                    let polarity = if sign && delta & 1 != 0 { -1.0 } else { 1.0 };
                                    gain = polarity * 2f32.powf(-(exponent as f32) * factor);
                                }
                            }
                            values.push(if book == 0 { 0.0 } else { gain });
                        }
                        bands.push(values);
                    }
                }
                if !gain.is_finite() || bands.iter().flatten().any(|v| !v.is_finite()) {
                    return Err(invalid("AAC coupling gain overflow"));
                }
                let channels: &[u8] = match (pair, selection, list) {
                    (false, _, _) => &[0],
                    (true, 0, _) => &[0, 1],
                    (true, 1, _) => &[1],
                    (true, 2, _) => &[0],
                    (true, 3, 0) => &[0],
                    (true, 3, _) => &[1],
                    _ => unreachable!(),
                };
                for &channel in channels {
                    targets.push(Target {
                        pair,
                        tag,
                        channel,
                        gain,
                        bands: bands.clone(),
                    });
                }
                gain_index += 1;
            }
        }
        *bits = input;
        Ok((Self {
            tag,
            point,
            channel,
            targets,
        }, prediction))
    }
}

impl Coupling {
    pub(crate) fn mix_spectrum(
        &self,
        target: &Target,
        config: &AacConfig,
        source: &[f32],
        destination: &mut [f32],
    ) -> Result<()> {
        let tables = BandTables::for_config(config)?;
        let offsets =
            if self.channel.info.sequence == super::aac_synthesis::WindowSequence::EightShort {
                tables.short
            } else {
                tables.long
            };
        coupling_mix::mix_spectrum(
            source,
            destination,
            offsets,
            &self.channel.info.group_lengths,
            &target.bands,
        )
        .map_err(Error::from)
    }
}
