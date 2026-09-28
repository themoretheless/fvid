//! AAC-LC channel-pair syntax, prior to stereo spectral reconstruction.
use super::{aac_bands::BandTables, aac_channel::ChannelData, bits::BitReader, config::AacConfig};
use crate::{Result, invalid};

pub struct ChannelPair {
    pub left: ChannelData,
    pub right: ChannelData,
    /// Group/band mask; absent when channels have independent windows.
    pub mid_side: Option<Vec<Vec<bool>>>,
}
impl ChannelPair {
    /// Restore ordinary stereo spectral bands. Noise/intensity bands are
    /// rejected by channel reconstruction until their dedicated tools exist.
    pub fn ordinary_spectra(&self, config: &AacConfig) -> Result<(Vec<f32>, Vec<f32>)> {
        let mut left = self.left.ordinary_spectrum(config)?;
        let mut right = self.right.ordinary_spectrum(config)?;
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
                    if !enabled {
                        continue;
                    }
                    for window in first..first + length as usize {
                        for i in window * size + offsets[band]..window * size + offsets[band + 1] {
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
        let mid_side = if let Some(info) = &common {
            let mode = cursor.read(2)?;
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
            assert_eq!(bits.read(3).unwrap(), 7);
            start += length;
            frames += 1;
        }
        assert_eq!(frames, 13);
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
        };
        let mut pair = ChannelPair {
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
