use super::{

    aac_ics::IcsInfo,
    aac_pulse::PulseData,
    aac_scalefactors::{self, BandScale},
    aac_spectral,
    aac_synthesis::WindowSequence,
    bits::BitReader,
    config::AacConfig,
};

pub struct ChannelData {
    pub info: IcsInfo,
    pub codebooks: Vec<Vec<u8>>,
    pub scales: Vec<Vec<BandScale>>,
    /// Group/band/window order, with special-band placeholders.
    pub quantized: Vec<i16>,
    pub pulse: Option<PulseData>,
    pub tns: Option<tns_syntax::TnsData>,
    pub gain: Option<super::aac_gain_control::GainControl>,
}
impl ChannelData {
    /// Reconstruct ordinary spectral bands into per-window order. Special
    /// noise/intensity bands need separate tools and are rejected here.
    /// Output uses AAC spectral units; PCM normalization is not applied.
    pub fn ordinary_spectrum(&self, config: &AacConfig) -> Result<Vec<f32>> {
        self.spectrum_with_intensity(config, false)
    }
    pub(crate) fn spectrum_with_intensity(
        &self,
        config: &AacConfig,
        allow: bool,
    ) -> Result<Vec<f32>> {
        self.spectrum_tools(config, allow, None)
    }
    /// Reconstruct a single channel including noise substitution. Generator
    /// state commits only on success, allowing packet-level rollback.
    pub fn spectrum_with_noise(
        &self,
        config: &AacConfig,
        noise: &mut super::aac_noise::NoiseState,
    ) -> Result<Vec<f32>> {
        let mut next = noise.clone();
        let output = self.spectrum_tools(config, false, Some(&mut next))?;
        *noise = next;
        Ok(output)
    }
    pub(crate) fn spectrum_tools(
        &self,
        config: &AacConfig,
        allow: bool,
        mut noise: Option<&mut super::aac_noise::NoiseState>,
    ) -> Result<Vec<f32>> {
        let tables = BandTables::for_config(config)?;
        let offsets = if self.info.sequence == WindowSequence::EightShort {
            tables.short
        } else {
            tables.long
        };
        let n = config.frame_samples as usize;
        if self.scales.len() != self.info.group_lengths.len()
            || self.codebooks.len() != self.scales.len()
            || self
                .scales
                .iter()
                .any(|g| g.len() != self.info.max_sfb as usize)
            || self
                .codebooks
                .iter()
                .any(|g| g.len() != self.info.max_sfb as usize)
        {
            return Err(invalid("AAC reconstruction band layout mismatch"));
        }
        let mut quantized = vec![0; n];
        self.info.deinterleave_quantized(offsets, &self.quantized, &mut quantized)?;
        if let Some(pulse) = &self.pulse {
            if self.info.sequence == WindowSequence::EightShort {
                return Err(invalid("pulse on short AAC window"));
            }
            pulse.apply_where(&mut quantized, |position| {
                let band = offsets.partition_point(|&start| start <= position) - 1;
                band < self.info.max_sfb as usize
                    && matches!(self.codebooks[0][band], 1..=11)
                    && matches!(self.scales[0][band], BandScale::Spectral(_))
            })?;
        }
        let mut ordered = vec![0.0; n];
        let size = *offsets.last().unwrap();
        let mut first_window = 0;
        for (group, &length) in self.info.group_lengths.iter().enumerate() {
            for (band, scale) in self.scales[group].iter().enumerate() {
                let book = self.codebooks[group][band];
                match (book, scale) {
                    (0, BandScale::Zero) | (1..=11, BandScale::Spectral(_)) => {}
                    (14..=15, BandScale::Intensity(_)) if allow => {}
                    (13, BandScale::Noise(_)) if noise.is_some() => {}
                    (13, BandScale::Noise(_)) | (14..=15, BandScale::Intensity(_)) => {
                        return Err(unsupported(
                            "AAC noise/intensity reconstruction is not implemented",
                        ));
                    }
                    _ => return Err(invalid("AAC codebook/scalefactor mismatch")),
                }
                for window in first_window..first_window + length as usize {
                    let range = window * size + offsets[band]..window * size + offsets[band + 1];
                    if let BandScale::Spectral(scale) = scale {
                        super::aac_quant::inverse_quantize(
                            &quantized[range.clone()],
                            i16::from(*scale),
                            &mut ordered[range],
                        )?;
                    } else if quantized[range.clone()].iter().any(|&q| q != 0) {
                        return Err(invalid("nonzero AAC zero-codebook band"));
                    } else if let BandScale::Noise(energy) = scale {
                        noise
                            .as_deref_mut()
                            .unwrap()
                            .band(*energy, &mut ordered[range])?;
                    }
                }
            }
            first_window += length as usize;
        }
        Ok(ordered)
    }

    pub(crate) fn predict_main(&self, config: &AacConfig, bank: &mut super::aac_main_predictor::MainPredictor, spectrum: &mut [f32]) -> Result<()> {
        if self.info.sequence == WindowSequence::EightShort { bank.short_window(); return Ok(()); }
        let tables = BandTables::for_config(config)?;
        let count = tables.prediction_limit.ok_or_else(|| invalid("AAC Main predictor requires Main configuration"))?;
        let offsets = &tables.long[..=count];
        let mut used = self.info.prediction.as_ref().map_or_else(Vec::new, |p| p.used.clone());
        let mut source = spectrum.to_vec();
        let mut pns = Vec::new();
        for band in 0..usize::from(self.info.max_sfb).min(count) {
            if self.codebooks[0][band] >= 13 {
                if let Some(flag) = used.get_mut(band) { *flag = false; }
                if self.codebooks[0][band] == 13 {
                    source[offsets[band]..offsets[band+1]].fill(0.0);
                    pns.push(offsets[band]..offsets[band+1]);
                }
            }
        }
        bank.process(&mut source, offsets, &used, self.info.prediction.as_ref().and_then(|p| p.reset_group))?;
        for range in pns { source[range.clone()].copy_from_slice(&spectrum[range.clone()]); bank.reset_lines(range)?; }
        spectrum.copy_from_slice(&source);
        Ok(())
    }
    /// Apply TNS after stereo tools, immediately before window synthesis.
    pub fn apply_tns(&self, config: &AacConfig, spectrum: Vec<f32>) -> Result<Vec<f32>> {
        if let Some(tns) = &self.tns {
            let tables = BandTables::for_config(config)?;
            let short = self.info.sequence == WindowSequence::EightShort;
            let offsets = if short { tables.short } else { tables.long };
            let limit =
                (if config.object_type == 3 { BandTables::ssr_tns_limit(config.sample_rate, short) } else { BandTables::tns_limit(config.sample_rate, short) }).min(self.info.max_sfb as usize);
            tns.filter_owned(spectrum, offsets, limit).map_err(Error::from)
        } else {
            Ok(spectrum)
        }
    }
    /// Starts at global_gain, after the element tag. Transactional on failure.
    pub fn read(bits: &mut BitReader<'_>, config: &AacConfig) -> Result<Self> {
        Self::read_common(bits, config, None)
    }
    pub(crate) fn read_common(
        bits: &mut BitReader<'_>,
        config: &AacConfig,
        common: Option<&IcsInfo>,
    ) -> Result<Self> {
        let tables = BandTables::for_config(config)?;
        let mut cursor = bits.clone();
        let gain = cursor.read(8)? as u8;
        let info = match common {
            Some(info) => info.clone(),
            None => tables.read_ics(&mut cursor)?,
        };
        let codebooks = info.read_sections(&mut cursor)?;
        let scales = aac_scalefactors::read(&mut cursor, gain, &codebooks)?;
        let pulse = if cursor.bit()? {
            Some(PulseData::read(&mut cursor, info.sequence, tables.long)?)
        } else {
            None
        };
        let tns = if cursor.bit()? {
            Some(tns_syntax::read_profile(&mut cursor, info.sequence, config.object_type == 1)?)
        } else {
            None
        };
        let gain_control = if cursor.bit()? {
            let gain = super::aac_gain_control::GainControl::read(&mut cursor, info.sequence)?;
            if config.object_type != 3 && !gain.is_empty() {
                return Err(unsupported("owned AAC active gain control synthesis is not implemented"));
            }
            Some(gain)
        } else { None };
        let offsets = if info.sequence == WindowSequence::EightShort {
            tables.short
        } else {
            tables.long
        };
        let quantized = aac_spectral::read(
            &mut cursor,
            &info,
            offsets,
            &codebooks,
            config.frame_samples as usize,
        )?;
        *bits = cursor;
        Ok(Self {
            info,
            codebooks,
            scales,
            quantized,
            pulse,
            tns,
            gain: gain_control,
        })
    }
}
