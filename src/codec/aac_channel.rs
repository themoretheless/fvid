//! Owned AAC-LC individual channel parsing.
use super::{
    aac_bands::BandTables,
    aac_ics::IcsInfo,
    aac_pulse::PulseData,
    aac_scalefactors::{self, BandScale},
    aac_spectral,
    aac_synthesis::WindowSequence,
    bits::BitReader,
    config::AacConfig,
};
use crate::{Result, invalid, unsupported};

pub struct ChannelData {
    pub info: IcsInfo,
    pub codebooks: Vec<Vec<u8>>,
    pub scales: Vec<Vec<BandScale>>,
    /// Group/band/window order, with special-band placeholders.
    pub quantized: Vec<i16>,
    pub pulse: Option<PulseData>,
    pub tns: Option<super::aac_tns::TnsData>,
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
        let mut ordered = vec![0.0; n];
        self.info.deinterleave(
            offsets,
            &self.quantized.iter().map(|&q| q as f32).collect::<Vec<_>>(),
            &mut ordered,
        )?;
        let mut quantized: Vec<i16> = ordered.iter().map(|&q| q as i16).collect();
        if let Some(pulse) = &self.pulse {
            if self.info.sequence == WindowSequence::EightShort {
                return Err(invalid("pulse on short AAC window"));
            }
            pulse.apply(&mut quantized)?;
            if quantized[offsets[self.info.max_sfb as usize]..]
                .iter()
                .any(|&v| v != 0)
            {
                return Err(unsupported(
                    "AAC pulse above coded bands requires reconstruction",
                ));
            }
        }
        ordered.fill(0.0);
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

    /// Apply TNS after stereo tools, immediately before window synthesis.
    pub fn apply_tns(&self, config: &AacConfig, spectrum: Vec<f32>) -> Result<Vec<f32>> {
        if let Some(tns) = &self.tns {
            let tables = BandTables::for_config(config)?;
            let short = self.info.sequence == WindowSequence::EightShort;
            let offsets = if short { tables.short } else { tables.long };
            let limit =
                BandTables::tns_limit(config.sample_rate, short).min(self.info.max_sfb as usize);
            tns.filter(&spectrum, offsets, limit).map_err(crate::Error::from)
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
            Some(super::aac_tns::read(&mut cursor, info.sequence)?)
        } else {
            None
        };
        if cursor.bit()? {
            return Err(unsupported("owned AAC gain control is not implemented"));
        }
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{aac_huffman_tables::*, aac_quant, aac_synthesis::LongSineSynthesis};
    fn pack(fields: &[(u32, u8)]) -> (Vec<u8>, usize) {
        let mut data = Vec::new();
        let mut n = 0;
        for &(v, width) in fields {
            for bit in (0..width).rev() {
                if n % 8 == 0 {
                    data.push(0);
                }
                *data.last_mut().unwrap() |= ((v >> bit & 1) as u8) << (7 - n % 8);
                n += 1;
            }
        }
        (data, n)
    }
    #[test]
    fn channel_payload_reaches_owned_synthesis() {
        let config = AacConfig::parse(&[0x11, 0x90]).unwrap();
        let (data, count) = pack(&[
            (100, 8), // global gain
            (0, 1),
            (0, 2),
            (0, 1),
            (1, 6),
            (0, 1), // long ICS, one band
            (1, 4),
            (1, 5), // section: one band, book 1
            (SCF_CODEBOOK_CODES[60], SCF_CODEBOOK_LENS[60]),
            (0, 1),
            (0, 1),
            (0, 1), // no pulse, TNS, gain control
            (SPECTRUM_CODEBOOK1_CODES[80], SPECTRUM_CODEBOOK1_LENS[80]),
        ]);
        let mut bits = BitReader::new(&data);
        let channel = ChannelData::read(&mut bits, &config).unwrap();
        assert_eq!(bits.position(), count);
        assert_eq!(channel.quantized, [1, 1, 1, 1]);
        assert_eq!(channel.scales, vec![vec![BandScale::Spectral(100)]]);
        let reconstructed = channel.ordinary_spectrum(&config).unwrap();
        assert_eq!(&reconstructed[..4], &[1.0; 4]);
        assert!(reconstructed[4..].iter().all(|&x| x == 0.0));
        let mut grouped = [0.0; 4];
        aac_quant::inverse_quantize(&channel.quantized, 100, &mut grouped).unwrap();
        let mut spectrum = vec![0.0; 1024];
        let tables = BandTables::for_config(&config).unwrap();
        channel
            .info
            .deinterleave(tables.long, &grouped, &mut spectrum)
            .unwrap();
        let mut pcm = vec![0.0; 1024];
        LongSineSynthesis::new(1024)
            .unwrap()
            .synthesize_shaped(
                channel.info.sequence,
                channel.info.shape,
                &spectrum,
                &mut pcm,
            )
            .unwrap();
        assert!(pcm.iter().all(|x| x.is_finite()));
        assert!(pcm.iter().any(|x| x.abs() > 1e-6));
        for length in 0..data.len() - 1 {
            let mut bits = BitReader::new(&data[..length]);
            assert!(ChannelData::read(&mut bits, &config).is_err());
            assert_eq!(bits.position(), 0);
        }
    }
    #[test]
    fn reconstruction_scales_each_short_group_and_band() {
        let config = AacConfig::parse(&[0x11, 0x90]).unwrap();
        let mut channel = ChannelData {
            info: IcsInfo {
                sequence: WindowSequence::EightShort,
                shape: crate::codec::aac_synthesis::WindowShape::Sine,
                max_sfb: 2,
                group_lengths: vec![3, 5],
            },
            codebooks: vec![vec![5, 5], vec![5, 5]],
            scales: vec![
                vec![BandScale::Spectral(100), BandScale::Spectral(104)],
                vec![BandScale::Spectral(96), BandScale::Spectral(100)],
            ],
            quantized: [vec![8; 12], vec![-8; 12], vec![8; 20], vec![-8; 20]].concat(),
            pulse: None,
            tns: None,
        };
        let spectrum = channel.ordinary_spectrum(&config).unwrap();
        for window in 0..8 {
            assert_eq!(
                &spectrum[window * 128..window * 128 + 4],
                &[if window < 3 { 16.0 } else { 8.0 }; 4]
            );
            assert_eq!(
                &spectrum[window * 128 + 4..window * 128 + 8],
                &[if window < 3 { -32.0 } else { -16.0 }; 4]
            );
            assert!(
                spectrum[window * 128 + 8..(window + 1) * 128]
                    .iter()
                    .all(|&v| v == 0.0)
            );
        }
        channel.scales[0][0] = BandScale::Noise(0);
        assert!(channel.ordinary_spectrum(&config).is_err());
        channel.codebooks[0][0] = 13;
        assert!(matches!(
            channel.ordinary_spectrum(&config),
            Err(crate::Error::Unsupported(_))
        ));
    }
    #[test]
    fn noise_bands_reconstruct_per_window_and_rollback_on_late_error() {
        let config = AacConfig::parse(&[0x11, 0x90]).unwrap();
        let mut channel = ChannelData {
            info: IcsInfo {
                sequence: WindowSequence::EightShort,
                shape: crate::codec::aac_synthesis::WindowShape::Sine,
                max_sfb: 2,
                group_lengths: vec![8],
            },
            codebooks: vec![vec![13, 13]],
            scales: vec![vec![BandScale::Noise(0), BandScale::Noise(4)]],
            quantized: vec![0; 64],
            pulse: None,
            tns: None,
        };
        let mut noise = crate::codec::aac_noise::NoiseState::default();
        let spectrum = channel.spectrum_with_noise(&config, &mut noise).unwrap();
        for w in 0..8 {
            for band in 0..2 {
                let start = w * 128 + band * 4;
                let energy: f64 = spectrum[start..start + 4]
                    .iter()
                    .map(|&x| f64::from(x).powi(2))
                    .sum();
                assert!((energy - if band == 0 { 1.0 } else { 4.0 }).abs() < 1e-6);
            }
        }
        let saved = noise.clone();
        channel.scales[0][1] = BandScale::Noise(156);
        assert!(channel.spectrum_with_noise(&config, &mut noise).is_err());
        assert_eq!(noise, saved);
    }
    #[test]
    fn unsupported_tools_do_not_consume_channel_header() {
        let config = AacConfig::parse(&[0x11, 0x90]).unwrap();
        for (tns, gain) in [(0, 1)] {
            let (data, _) = pack(&[
                (100, 8),
                (0, 1),
                (0, 2),
                (0, 1),
                (0, 6),
                (0, 1),
                (0, 1),
                (tns, 1),
                (gain, 1),
            ]);
            let mut bits = BitReader::new(&data);
            assert!(ChannelData::read(&mut bits, &config).is_err());
            assert_eq!(bits.position(), 0);
        }
    }
}
