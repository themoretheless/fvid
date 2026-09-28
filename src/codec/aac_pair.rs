//! AAC-LC channel-pair syntax, prior to stereo spectral reconstruction.
use super::{aac_bands::BandTables, aac_channel::ChannelData, bits::BitReader, config::AacConfig};
use crate::{Result, invalid};

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
    fn stereo_tools(
        &self,
        config: &AacConfig,
        mut left: Vec<f32>,
        mut right: Vec<f32>,
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
                                let positive = self.right.codebooks[group][band] == 15;
                                let invert = self.explicit_mask && enabled;
                                let sign = if positive != invert { 1.0 } else { -1.0 };
                                right[i] = left[i] * sign * 2.0f32.powf(-f32::from(position) / 4.0);
                                continue;
                            }
                            let left_noise = self.left.codebooks[group][band] == 13;
                            let right_noise = self.right.codebooks[group][band] == 13;
                            if left_noise || right_noise {
                                if left_noise && right_noise && enabled {
                                    if let (
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
        let mut cursor = bits.clone();
        let common = if cursor.bit()? {
            Some(BandTables::for_config(config)?.read_ics(&mut cursor)?)
        } else {
            None
        };
        let mut explicit_mask = false;
        let mid_side = if let Some(info) = &common {
            let mode = cursor.read(2)?;
            explicit_mask = mode == 1;
            if mode == 3 {
                return Err(invalid("reserved AAC mid/side mask mode"));
            }
            let mut mask = vec![vec![mode == 2; info.max_sfb as usize]; info.group_lengths.len()];
            if mode == 1 {
                for group in &mut mask {
                    for used in group {
                        *used = cursor.bit()?;
                    }
                }
            }
            Some(mask)
        } else {
            None
        };
        let left = ChannelData::read_common(&mut cursor, config, common.as_ref())?;
        let right = ChannelData::read_common(&mut cursor, config, common.as_ref())?;
        *bits = cursor;
        Ok(Self {
            left,
            right,
            mid_side,
            explicit_mask,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn pack(fields: &[(u32, u8)]) -> (Vec<u8>, usize) {
        let mut data = Vec::new();
        let mut n = 0;
        for &(v, width) in fields {
            for b in (0..width).rev() {
                if n % 8 == 0 {
                    data.push(0);
                }
                *data.last_mut().unwrap() |= ((v >> b & 1) as u8) << (7 - n % 8);
                n += 1;
            }
        }
        (data, n)
    }
    #[test]
    fn common_window_mask_modes_and_two_channel_payloads() {
        let config = AacConfig::parse(&[0x11, 0x90]).unwrap();
        for mode in 0..3 {
            let mut fields = vec![(1, 1), (0, 1), (0, 2), (1, 1), (2, 6), (0, 1), (mode, 2)];
            if mode == 1 {
                fields.extend([(1, 1), (0, 1)]);
            }
            // Zero-codebook section covers two bands, no scale or spectral bits.
            for gain in [100, 120] {
                fields.extend([(gain, 8), (0, 4), (2, 5), (0, 3)]);
            }
            let (data, count) = pack(&fields);
            let mut bits = BitReader::new(&data);
            let pair = ChannelPair::read(&mut bits, &config).unwrap();
            assert_eq!(bits.position(), count);
            assert_eq!(pair.left.info, pair.right.info);
            assert_eq!(pair.mid_side.unwrap(), vec![vec![mode != 0, mode == 2]]);
            assert_eq!(pair.left.quantized, vec![0; 8]);
            for length in 0..data.len() - 1 {
                let mut bits = BitReader::new(&data[..length]);
                assert!(ChannelPair::read(&mut bits, &config).is_err());
                assert_eq!(bits.position(), 0);
            }
        }
    }
    #[test]
    fn real_adts_stereo_all_pairs_parse() {
        let data = include_bytes!("../../tests/fixtures/audio/aac-stereo.aac");
        let mut start = 0;
        let mut frames = 0;
        let mut synth_left = super::super::aac_synthesis::LongSineSynthesis::new(1024).unwrap();
        let mut synth_right = super::super::aac_synthesis::LongSineSynthesis::new(1024).unwrap();
        let mut decoded = Vec::new();
        let mut noise = super::super::aac_noise::NoiseState::default();
        while start < data.len() {
            let data = &data[start..];
            let length = ((data[3] as usize & 3) << 11)
                | ((data[4] as usize) << 3)
                | (data[5] as usize >> 5);
            let mut bits = BitReader::new(&data[7..length]);
            // Encoder metadata may precede the first channel pair in a fill element.
            loop {
                let element = bits.read(3).unwrap();
                if element == 6 {
                    let mut count = bits.read(4).unwrap() as usize;
                    if count == 15 {
                        count += bits.read(8).unwrap() as usize;
                        count -= 1;
                    }
                    bits.skip(count * 8).unwrap();
                } else {
                    assert_eq!(element, 1);
                    bits.read(4).unwrap();
                    break;
                }
            }
            let config = AacConfig::parse(&[0x11, 0x90]).unwrap();
            let pair = ChannelPair::read(&mut bits, &config)
                .unwrap_or_else(|e| panic!("frame {frames}: {e}"));
            assert!(!pair.left.quantized.is_empty());
            let (left, right) = pair.spectra_with_noise(&config, &mut noise).unwrap();
            assert_eq!((left.len(), right.len()), (1024, 1024));
            assert!(left.iter().chain(&right).all(|x| x.is_finite()));
            let mut l = vec![0.0; 1024];
            let mut r = vec![0.0; 1024];
            synth_left
                .synthesize_pcm(pair.left.info.sequence, pair.left.info.shape, &left, &mut l)
                .unwrap();
            synth_right
                .synthesize_pcm(
                    pair.right.info.sequence,
                    pair.right.info.shape,
                    &right,
                    &mut r,
                )
                .unwrap();
            for i in 0..1024 {
                decoded.push(l[i] as f32);
                decoded.push(r[i] as f32);
            }
            assert_eq!(bits.read(3).unwrap(), 7);
            start += length;
            frames += 1;
        }
        assert_eq!(frames, 13);
        let reference = include_bytes!("../../tests/fixtures/audio/aac-stereo-reference.f32le");
        assert_eq!(decoded.len() * 4, reference.len());
        let mut squared_error = 0.0;
        let mut peak_error = 0.0f64;
        for (&actual, bytes) in decoded.iter().zip(reference.chunks_exact(4)) {
            let expected = f32::from_le_bytes(bytes.try_into().unwrap());
            let error = f64::from(actual) - f64::from(expected);
            squared_error += error * error;
            peak_error = peak_error.max(error.abs());
        }
        let rms = (squared_error / decoded.len() as f64).sqrt();
        // PNS random sequences differ. Bound aggregate error and transients,
        // retaining sensitivity to amplitude, window or stereo regressions.
        assert!(rms < 0.00015, "RMS {rms}");
        assert!(peak_error < 0.003, "peak {peak_error}");
    }
    #[test]
    fn mid_side_reconstructs_only_masked_bands_in_each_short_group() {
        use crate::codec::{
            aac_ics::IcsInfo,
            aac_scalefactors::BandScale,
            aac_synthesis::{WindowSequence, WindowShape},
        };
        let channel = |value| ChannelData {
            info: IcsInfo {
                sequence: WindowSequence::EightShort,
                shape: WindowShape::Sine,
                max_sfb: 2,
                group_lengths: vec![3, 5],
            },
            codebooks: vec![vec![5, 5]; 2],
            scales: vec![vec![BandScale::Spectral(100); 2]; 2],
            quantized: vec![value; 64],
            pulse: None,
            tns: None,
        };
        let mut pair = ChannelPair {
            explicit_mask: true,
            left: channel(8),
            right: channel(1),
            mid_side: Some(vec![vec![true, false], vec![false, true]]),
        };
        let config = AacConfig::parse(&[0x11, 0x90]).unwrap();
        let (left, right) = pair.ordinary_spectra(&config).unwrap();
        for w in 0..8 {
            for band in 0..2 {
                for bin in 0..4 {
                    let i = w * 128 + band * 4 + bin;
                    let enabled = (w < 3) == (band == 0);
                    assert_eq!(left[i], if enabled { 17.0 } else { 16.0 });
                    assert_eq!(right[i], if enabled { 15.0 } else { 1.0 });
                }
            }
        }
        pair.mid_side = Some(vec![vec![true]]);
        assert!(pair.ordinary_spectra(&config).is_err());
        pair.mid_side = None;
        let (left, right) = pair.ordinary_spectra(&config).unwrap();
        assert_eq!((left[0], right[0]), (16.0, 1.0));
        pair.right.quantized.fill(0);
        pair.right.scales = vec![vec![BandScale::Intensity(4); 2]; 2];
        for book in [14, 15] {
            pair.right.codebooks = vec![vec![book; 2]; 2];
            for explicit in [false, true] {
                pair.explicit_mask = explicit;
                for enabled in [false, true] {
                    pair.mid_side = Some(vec![vec![enabled; 2]; 2]);
                    let (left, right) = pair.ordinary_spectra(&config).unwrap();
                    let sign = if (book == 15) != (explicit && enabled) {
                        1.0
                    } else {
                        -1.0
                    };
                    for w in 0..8 {
                        for b in 0..8 {
                            assert_eq!(left[w * 128 + b], 16.0);
                            assert_eq!(right[w * 128 + b], sign * 8.0);
                        }
                    }
                }
            }
        }
        pair.mid_side = None;
        assert!(pair.ordinary_spectra(&config).is_err());
    }
    #[test]
    fn stereo_noise_correlation_energy_and_error_rollback() {
        use crate::codec::{
            aac_ics::IcsInfo,
            aac_noise::NoiseState,
            aac_scalefactors::BandScale,
            aac_synthesis::{WindowSequence, WindowShape},
        };
        let channel = |energy| ChannelData {
            info: IcsInfo {
                sequence: WindowSequence::OnlyLong,
                shape: WindowShape::Sine,
                max_sfb: 1,
                group_lengths: vec![1],
            },
            codebooks: vec![vec![13]],
            scales: vec![vec![BandScale::Noise(energy)]],
            quantized: vec![0; 4],
            pulse: None,
            tns: None,
        };
        let mut pair = ChannelPair {
            left: channel(0),
            right: channel(4),
            mid_side: Some(vec![vec![true]]),
            explicit_mask: true,
        };
        let config = AacConfig::parse(&[0x11, 0x90]).unwrap();
        let mut noise = NoiseState::default();
        let (left, right) = pair.spectra_with_noise(&config, &mut noise).unwrap();
        for i in 0..4 {
            assert_eq!(right[i], 2.0 * left[i]);
        }
        let energy: f64 = right.iter().map(|&v| f64::from(v).powi(2)).sum();
        assert!((energy - 4.0).abs() < 1e-6);
        pair.mid_side = Some(vec![vec![false]]);
        noise.reset();
        let (independent_left, independent_right) =
            pair.spectra_with_noise(&config, &mut noise).unwrap();
        assert_eq!(left, independent_left);
        assert_ne!(right, independent_right);
        let saved = noise.clone();
        pair.mid_side = Some(vec![]);
        assert!(pair.spectra_with_noise(&config, &mut noise).is_err());
        assert_eq!(noise, saved);
    }
    #[test]
    fn reserved_mask_is_rejected_before_channels() {
        let config = AacConfig::parse(&[0x11, 0x90]).unwrap();
        let (data, _) = pack(&[(1, 1), (0, 1), (0, 2), (0, 1), (0, 6), (0, 1), (3, 2)]);
        let mut bits = BitReader::new(&data);
        assert!(ChannelPair::read(&mut bits, &config).is_err());
        assert_eq!(bits.position(), 0);
    }
}
