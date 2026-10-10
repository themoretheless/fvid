use super::{aac_channel::ChannelData, bits::BitReader, config::AacConfig};

pub struct ChannelPair {
    pub left: ChannelData,
    pub right: ChannelData,
    /// Group/band mask; absent when channels have independent windows.
    pub mid_side: Option<Vec<Vec<bool>>>,
    /// Only explicit mask mode 1 inverts intensity stereo polarity.
    pub explicit_mask: bool,
}
impl ChannelPair {
    /// Restore ordinary and intensity stereo bands. Noise bands still require
    /// their dedicated reconstruction tool and are rejected.
    pub fn ordinary_spectra(&self, config: &AacConfig) -> Result<(Vec<f32>, Vec<f32>)> {
        let left = self.left.ordinary_spectrum(config)?;
        let right = self
            .right
            .spectrum_with_intensity(config, self.mid_side.is_some())?;
        self.stereo_tools(config, left, right)
    }
    /// Reconstruct PNS and stereo jointly; any channel or geometry error
    /// leaves the caller's noise generator at its original state.
    pub fn spectra_with_noise(
        &self,
        config: &AacConfig,
        noise: &mut super::aac_noise::NoiseState,
    ) -> Result<(Vec<f32>, Vec<f32>)> {
        let mut next = noise.clone();
        let left = self.left.spectrum_tools(config, false, Some(&mut next))?;
        let right = self
            .right
            .spectrum_tools(config, self.mid_side.is_some(), Some(&mut next))?;
        let result = self.stereo_tools(config, left, right)?;
        *noise = next;
        Ok(result)
    }
    pub(crate) fn spectra_with_main_prediction(&self, config: &AacConfig, noise: &mut super::aac_noise::NoiseState, left_bank: &mut super::aac_main_predictor::MainPredictor, right_bank: &mut super::aac_main_predictor::MainPredictor) -> Result<(Vec<f32>, Vec<f32>)> {
        let left = self.left.spectrum_tools(config, false, Some(noise))?;
        let right = self.right.spectrum_tools(config, self.mid_side.is_some(), Some(noise))?;
        let (mut left, right) = self.stereo_phase(config, left, right, true, false)?;
        self.left.predict_main(config, left_bank, &mut left)?;
        let (left, mut right) = self.stereo_phase(config, left, right, false, true)?;
        self.right.predict_main(config, right_bank, &mut right)?;
        Ok((left, right))
    }
    fn stereo_tools(&self, config: &AacConfig, left: Vec<f32>, right: Vec<f32>) -> Result<(Vec<f32>, Vec<f32>)> {
        self.stereo_phase(config, left, right, true, true)
    }
    fn stereo_phase(
        &self,
        config: &AacConfig,
        mut left: Vec<f32>,
        mut right: Vec<f32>,
        mid_side_phase: bool, intensity_phase: bool,
    ) -> Result<(Vec<f32>, Vec<f32>)> {
        if let Some(mask) = &self.mid_side {
            if self.left.info != self.right.info
                || mask.len() != self.left.info.group_lengths.len()
                || mask
                    .iter()
                    .any(|g| g.len() != self.left.info.max_sfb as usize)
            {
                return Err(invalid("AAC mid/side geometry mismatch"));
            }
            let tables = BandTables::for_config(config)?;
            let offsets =
                if self.left.info.sequence == super::aac_synthesis::WindowSequence::EightShort {
                    tables.short
                } else {
                    tables.long
                };
            let size = *offsets.last().unwrap();
            let mut first = 0;
            for (group, &length) in self.left.info.group_lengths.iter().enumerate() {
                for (band, &enabled) in mask[group].iter().enumerate() {
                    let intensity = match self.right.scales[group][band] {
                        super::aac_scalefactors::BandScale::Intensity(position) => Some(position),
                        _ => None,
                    };
                    if !enabled && intensity.is_none() {
                        continue;
                    }
                    for window in first..first + length as usize {
                        for i in window * size + offsets[band]..window * size + offsets[band + 1] {
                            if let Some(position) = intensity {
                                if !intensity_phase { continue; }
                                let positive = self.right.codebooks[group][band] == 15;
                                let invert = self.explicit_mask && enabled;
                                let sign = if positive != invert { 1.0 } else { -1.0 };
                                right[i] = left[i] * sign * 2.0f32.powf(-f32::from(position) / 4.0);
                                continue;
                            }
                            if !mid_side_phase { continue; }
                            let left_noise = self.left.codebooks[group][band] == 13;
                            let right_noise = self.right.codebooks[group][band] == 13;
                            if left_noise || right_noise {
                                if left_noise && right_noise && enabled
                                    && let (
                                        super::aac_scalefactors::BandScale::Noise(le),
                                        super::aac_scalefactors::BandScale::Noise(re),
                                    ) = (
                                        self.left.scales[group][band],
                                        self.right.scales[group][band],
                                    ) {
                                        right[i] = (f64::from(left[i])
                                            * 2.0f64.powf(f64::from(re - le) / 4.0))
                                            as f32;
                                    }
                                continue;
                            }
                            let (mid, side) = (left[i], right[i]);
                            left[i] = mid + side;
                            right[i] = mid - side;
                        }
                    }
                }
                first += length as usize;
            }
        }
        Ok((left, right))
    }

    /// Starts after element_instance_tag. A failure in either channel rolls
    /// back the entire pair's input cursor.
    pub fn read(bits: &mut BitReader<'_>, config: &AacConfig) -> Result<Self> {
        Self::read_with_right_span(bits, config).map(|(pair, _)| pair)
    }
    /// Return the second ICS bit span for ADTS error protection. Parsing is transactional.
    pub(crate) fn read_with_right_span(
        bits: &mut BitReader<'_>,
        config: &AacConfig,
    ) -> Result<(Self, std::ops::Range<usize>)> {
        let mut cursor = bits.clone();
        let common = if cursor.bit()? {
            Some(BandTables::for_config(config)?.read_ics(&mut cursor)?)
        } else {
            None
        };
        let (explicit_mask, mid_side) = Self::read_mask(&mut cursor, common.as_ref())?;
        let left = ChannelData::read_common(&mut cursor, config, common.as_ref(), true)?;
        let right_start = cursor.position();
        let right = ChannelData::read_common(&mut cursor, config, common.as_ref(), true)?;
        let right_span = right_start..cursor.position();
        *bits = cursor;
        Ok((
            Self { left, right, mid_side, explicit_mask },
            right_span,
        ))
    }
    fn read_mask(bits: &mut BitReader<'_>, common: Option<&super::aac_ics::IcsInfo>) -> Result<(bool, Option<Vec<Vec<bool>>>)> {
        let mut explicit_mask = false;
        let mid_side = if let Some(info) = common {
            let mode = bits.read(2)?;
            explicit_mask = mode == 1;
            if mode == 3 {
                return Err(invalid("reserved AAC mid/side mask mode"));
            }
            let mut mask = vec![vec![mode == 2; info.max_sfb as usize]; info.group_lengths.len()];
            if mode == 1 {
                for group in &mut mask {
                    for used in group {
                        *used = bits.bit()?;
                    }
                }
            }
            Some(mask)
        } else {
            None
        };
        Ok((explicit_mask, mid_side))
    }
    /// Parse ordinary AOT4 pairs with independent per-channel LTP data.
    /// A failure in either stream restores the complete pair cursor.
    pub fn read_ltp(bits: &mut BitReader<'_>, config: &AacConfig) -> Result<(Self, [Option<super::aac_ltp_syntax::LtpData>; 2])> {
        Self::read_ltp_with_right_span(bits, config).map(|(pair, prediction, _)| (pair, prediction))
    }

    /// LTP-aware second ICS span for ADTS protection, with whole-pair rollback.
    pub(crate) fn read_ltp_with_right_span(
        bits: &mut BitReader<'_>,
        config: &AacConfig,
    ) -> Result<(Self, [Option<super::aac_ltp_syntax::LtpData>; 2], std::ops::Range<usize>)> {
        if config.object_type != 4 { return Err(invalid("AAC LTP pair requires AOT4")); }
        let tables = BandTables::for_config(config)?;
        let mut cursor = bits.clone();
        let common = if cursor.bit()? {
            Some(super::aac_ltp_syntax::LtpIcsInfo::read(&mut cursor,
                ((tables.long.len()-1) as u8,(tables.short.len()-1) as u8), config.frame_samples,true)?)
        } else { None };
        let (explicit_mask, mid_side) = Self::read_mask(&mut cursor, common.as_ref().map(|header| &header.info))?;
        let (left, right, prediction, right_start) = if let Some(header) = common {
            let left = ChannelData::read_common(&mut cursor,config,Some(&header.info),true)?;
            let right_start = cursor.position();
            let right = ChannelData::read_common(&mut cursor,config,Some(&header.info),true)?;
            (left, right, header.channels, right_start)
        } else {
            let (left, before) = ChannelData::read_ltp(&mut cursor,config)?;
            let right_start = cursor.position();
            let (right, after) = ChannelData::read_ltp(&mut cursor,config)?;
            (left,right,[before,after],right_start)
        };
        let right_span = right_start..cursor.position();
        *bits = cursor;
        Ok((Self { left,right,mid_side,explicit_mask },prediction,right_span))
    }

}
