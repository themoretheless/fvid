//! Residual extension syntax and AAC-LC MDCT reconstruction.
//!
//! Starts at the bounded kind0 extension payload, without raw_data_block element
//! IDs/tags. This stage does not perform MDCT-to-QMF conversion or enable native
//! residual playback. Coefficients remain in AAC spectral units.
use super::{
    Result,
    aac_channel::ChannelData,
    aac_ld_mps_config::{Config, ExtensionValue, Tree},
    aac_synthesis::{WindowSequence, WindowShape},
    bits::BitReader,
    config::AacConfig,
    invalid,
    memory::Footprint,
};
#[derive(Clone, Debug, PartialEq)]
pub struct Spectrum {
    pub sequence: WindowSequence,
    pub shape: WindowShape,
    pub coefficients: Vec<f32>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub first: Spectrum,
    /// Additional short ICS used by the18/24/30-slot MDCT-to-QMF schedules.
    pub extra: Option<Spectrum>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct BoxData {
    /// Per spatial parameter set. None means no ICC correction is transmitted.
    /// TTT boxes never carry these OTT-only presence bits.
    pub icc_differences: Vec<Option<Vec<u8>>>,
    pub blocks: Vec<Block>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub boxes: Vec<Option<BoxData>>,
}
/// ISO's capped unary code consumes an eighth bit even after seven zeros.
/// At value7 either eighth-bit value is accepted. Failed reads are atomic.
pub fn icc_difference(bits: &mut BitReader<'_>) -> Result<u8> {
    let mut b = bits.clone();
    let mut value = 0;
    for count in 0..8 {
        let bit = b.bit()?;
        if bit || count == 7 {
            break;
        }
        value += 1;
    }
    *bits = b;
    Ok(value)
}
fn spectrum(bits: &mut BitReader<'_>, config: &AacConfig) -> Result<Spectrum> {
    let channel = ChannelData::read(bits, config)?;
    // ISO/IEC 23003-1 §4.3.1.2 uses MPEG-2 AAC LC for residuals.
    // PNS belongs to MPEG-4 AAC; book13 is reserved in this syntax.
    if channel.codebooks.iter().flatten().any(|&book| book == 13) {
        return Err(invalid("reserved codebook 13 in MPEG-2 AAC residual"));
    }
    let coefficients = channel.apply_tns(config, channel.ordinary_spectrum(config)?)?;
    if channel.gain.is_some() {
        return Err(invalid("gain control is not a residual AAC-LC tool"));
    }
    Ok(Spectrum {
        sequence: channel.info.sequence,
        shape: channel.info.shape,
        coefficients,
    })
}
/// Transactional self-delimited payload reader. The caller owns extension byte
/// bounds and padding; following bits remain available after the last ICS.
pub fn read(
    bits: &mut BitReader<'_>,
    config: &Config,
    extension: &ExtensionValue,
    parameter_sets: usize,
) -> Result<Frame> {
    let ExtensionValue::Residual {
        rate,
        frames_code,
        bands,
    } = extension
    else {
        return Err(invalid("expected residual AAC extension configuration"));
    };
    let frames = usize::from(*frames_code) + 1;
    if config.tree != Tree::new(config.tree.code)?
        || config.frame_slots == 0
        || config.frame_slots > 32
        || *frames_code > 3
        || parameter_sets == 0
        || parameter_sets > usize::from(config.frame_slots)
        || bands.len() != config.tree.ott_lfe.len() + usize::from(config.tree.ttt_boxes)
        || bands.iter().flatten().any(|&b| b > config.parameter_bands)
        || usize::from(config.frame_slots) < frames
    {
        return Err(invalid("invalid residual AAC frame geometry"));
    }
    let core = AacConfig {
        channel_configuration: 1,
        object_type: 2,
        sample_rate: *rate,
        channels: 1,
        frame_samples: 1024,
        core_coder_delay: None,
        section_data_resilience: false,
        scalefactor_data_resilience: false,
        spectral_data_resilience: false,
    };
    super::aac_geometry::BandTables::for_config(&core)?;
    let update = usize::from(config.frame_slots) / frames;
    let mut b = bits.clone();
    let mut boxes = Vec::with_capacity(bands.len());
    for (row, band) in bands.iter().enumerate() {
        let count = usize::from(band.unwrap_or(0));
        if count == 0 {
            boxes.push(None);
            continue;
        }
        let mut icc_differences = Vec::new();
        if row < config.tree.ott_lfe.len() {
            for _ in 0..parameter_sets {
                icc_differences.push(if b.bit()? {
                    Some(
                        (0..count)
                            .map(|_| icc_difference(&mut b))
                            .collect::<Result<Vec<_>>>()?,
                    )
                } else {
                    None
                });
            }
        }
        let mut blocks = Vec::with_capacity(frames);
        for _ in 0..frames {
            let first = spectrum(&mut b, &core)?;
            let extra =
                if first.sequence == WindowSequence::EightShort && matches!(update, 18 | 24 | 30) {
                    let extra = spectrum(&mut b, &core)?;
                    if extra.sequence != first.sequence {
                        return Err(invalid(
                            "residual AAC extra short block has different window sequence",
                        ));
                    }
                    Some(extra)
                } else {
                    None
                };
            blocks.push(Block { first, extra });
        }
        boxes.push(Some(BoxData {
            icc_differences,
            blocks,
        }));
    }
    *bits = b;
    Ok(Frame { boxes })
}
impl Frame {
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        let mut f = Footprint::new();
        (|| -> std::result::Result<(), String> {
            f.vector(&self.boxes)?;
            for data in self.boxes.iter().flatten() {
                f.vector(&data.icc_differences)?;
                for row in data.icc_differences.iter().flatten() {
                    f.vector(row)?;
                }
                f.vector(&data.blocks)?;
                for b in &data.blocks {
                    f.vector(&b.first.coefficients)?;
                    if let Some(e) = &b.extra {
                        f.vector(&e.coefficients)?;
                    }
                }
            }
            Ok(())
        })()
        .map_err(|e| invalid(&e))?;
        Ok(f.total())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capped_unary_reads_exactly_the_reference_bits_at_all_offsets() {
        for byte in 0..=255u8 {
            for offset in 0..8 {
                let mut data = [0u8; 3];
                for i in 0..8 {
                    data[(offset + i) / 8] |= ((byte >> (7 - i)) & 1) << (7 - (offset + i) % 8);
                }
                let leading = byte.leading_zeros() as usize;
                let used = (leading + 1).min(8);
                let expected = leading.min(7) as u8;
                let mut b = BitReader::new(&data);
                b.skip(offset).unwrap();
                assert_eq!(icc_difference(&mut b).unwrap(), expected);
                assert_eq!(b.position(), offset + used);
                for n in 0..used {
                    let mut truncated = BitReader::with_bit_len(&data, offset + n).unwrap();
                    truncated.skip(offset).unwrap();
                    assert!(icc_difference(&mut truncated).is_err());
                    assert_eq!(truncated.position(), offset);
                }
            }
        }
    }
}
