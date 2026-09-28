//! Owned AAC-LC individual channel parsing (without common-window stereo yet).
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
use crate::{Result, unsupported};

pub struct ChannelData {
    pub info: IcsInfo,
    pub codebooks: Vec<Vec<u8>>,
    pub scales: Vec<Vec<BandScale>>,
    /// Group/band/window order, with special-band placeholders.
    pub quantized: Vec<i16>,
    pub pulse: Option<PulseData>,
}
impl ChannelData {
    /// Starts at global_gain, after the element tag. Transactional on failure.
    pub fn read(bits: &mut BitReader<'_>, config: &AacConfig) -> Result<Self> {
        let tables = BandTables::for_config(config)?;
        let mut cursor = bits.clone();
        let gain = cursor.read(8)? as u8;
        let info = tables.read_ics(&mut cursor)?;
        let codebooks = info.read_sections(&mut cursor)?;
        let scales = aac_scalefactors::read(&mut cursor, gain, &codebooks)?;
        let pulse = if cursor.bit()? {
            Some(PulseData::read(&mut cursor, info.sequence, tables.long)?)
        } else {
            None
        };
        if cursor.bit()? {
            return Err(unsupported("owned AAC TNS parsing is not implemented"));
        }
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
    fn unsupported_tools_do_not_consume_channel_header() {
        let config = AacConfig::parse(&[0x11, 0x90]).unwrap();
        for (tns, gain) in [(1, 0), (0, 1)] {
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
