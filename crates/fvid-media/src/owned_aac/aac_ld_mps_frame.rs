//! Owned LD-MPS spatial-frame syntax and stateful parameter dispatch.
//! Fine indices and tool metadata are not a spatial PCM rendering result.
use super::{
    Result,
    aac_ld_mps_config::{Config, ExtensionValue, Tree},
    aac_ld_mps_entropy::{self as entropy, Kind},
    aac_ld_mps_frame_facts::RESHAPE,
    aac_ld_mps_parameters::Parameters,
    bits::BitReader,
    invalid,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParameterId {
    OttCld(u8),
    /// None is the shared one-ICC region.
    OttIcc(Option<u8>),
    TttCpc {
        box_index: u8,
        high: bool,
        component: u8,
    },
    TttCld {
        box_index: u8,
        high: bool,
        component: u8,
    },
    TttIcc {
        box_index: u8,
        high: bool,
    },
    DownmixGain(u8),
    ArbitraryCld {
        extension: usize,
        box_index: usize,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParameterRows {
    pub id: ParameterId,
    pub kind: Kind,
    pub start_band: u8,
    pub values: Vec<Vec<i16>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Smoothing {
    pub mode: u8,
    pub time: Option<u8>,
    pub stride: Option<u8>,
    /// Encoded group mask; application/expansion is a later rendering step.
    pub groups: Vec<bool>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemporalShape {
    Disabled,
    Stp(Vec<bool>),
    /// Enabled channels carry one bounded reshape index per time slot.
    Ges(Vec<Option<Vec<u8>>>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionFrame {
    pub kind: u8,
    /// Residual AAC and unknown payloads remain opaque; this does not decode
    /// or accept their audio. Arbitrary-tree parameters are decoded separately.
    pub payload: Vec<u8>,
    pub parameters: Vec<ParameterRows>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub slots: Vec<u8>,
    pub independent: bool,
    pub parameters: Vec<ParameterRows>,
    pub smoothing: Vec<Smoothing>,
    pub temporal_shape: TemporalShape,
    pub extensions: Vec<ExtensionFrame>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Region {
    id: ParameterId,
    kind: Kind,
    start: u8,
    state: Parameters,
    forbid_default: bool,
}
impl Region {
    fn new(
        id: ParameterId,
        kind: Kind,
        start: u8,
        stop: u8,
        default: i16,
        forbid_default: bool,
    ) -> Result<Self> {
        Ok(Self {
            id,
            kind,
            start,
            state: Parameters::new(kind, usize::from(stop - start), default)?,
            forbid_default,
        })
    }
    fn read(
        &mut self,
        bits: &mut BitReader<'_>,
        slots: &[u8],
        frame_slots: u8,
        independent: bool,
        residual: bool,
    ) -> Result<ParameterRows> {
        // Restrictions are checked before advancing any parameter state. The
        // outer frame transaction also protects all previously parsed regions.
        let mut probe = bits.clone();
        for _ in slots {
            let mode = probe.read(2)?;
            if residual && mode == 2 {
                return Err(invalid(
                    "LD-MPS residual coding forbids interpolated parameters",
                ));
            }
            if self.forbid_default && mode == 0 {
                return Err(invalid(
                    "predictive LD-MPS TTT ICC requires non-default data",
                ));
            }
        }
        let values = self.state.decode(bits, slots, frame_slots, independent)?;
        Ok(ParameterRows {
            id: self.id.clone(),
            kind: self.kind,
            start_band: self.start,
            values,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoder {
    config: Config,
    regions: Vec<Region>,
    main_regions: usize,
    extension_regions: Vec<Vec<Region>>,
    residual: bool,
}
impl Decoder {
    pub fn new(config: &Config) -> Result<Self> {
        if config.tree != Tree::new(config.tree.code)?
            || !(1..=32).contains(&config.frame_slots)
            || ![4, 5, 7, 9, 12, 15, 23].contains(&config.parameter_bands)
            || config.temporal_shape > 2
            || config.quantization_mode > 2
            || config.ott_bands.len() != config.tree.ott_lfe.len()
            || config.ttt.len() != usize::from(config.tree.ttt_boxes)
            || config
                .ott_bands
                .iter()
                .zip(&config.tree.ott_lfe)
                .any(|(&n, &lfe)| n > config.parameter_bands || !lfe && n != config.parameter_bands)
            || config.ttt.iter().any(|t| {
                t.low_mode > 5
                    || t.high
                        .is_some_and(|(mode, split)| mode > 5 || split > config.parameter_bands)
            })
            || (config.tree.code == 7) != config.one_icc.is_none()
        {
            return Err(invalid("inconsistent LD-MPS frame configuration"));
        }
        let defaults: &[i16] = match config.tree.code {
            0 => &[15, 15, 0, 0, 15],
            1 => &[15, 0, 15, 15, 15],
            2 => &[15, 15, 15],
            3 | 4 => &[15, 15, 15, 15, 15],
            5 | 6 => &[15, 15],
            7 => &[15],
            _ => unreachable!(),
        };
        let mut regions = Vec::new();
        for (i, &stop) in config.ott_bands.iter().enumerate() {
            regions.push(Region::new(
                ParameterId::OttCld(i as u8),
                Kind::Cld,
                0,
                stop,
                defaults[i],
                false,
            )?);
        }
        if config.one_icc == Some(true) {
            regions.push(Region::new(
                ParameterId::OttIcc(None),
                Kind::Icc,
                0,
                config.parameter_bands,
                0,
                false,
            )?);
        } else {
            for (i, &lfe) in config.tree.ott_lfe.iter().enumerate() {
                if !lfe {
                    regions.push(Region::new(
                        ParameterId::OttIcc(Some(i as u8)),
                        Kind::Icc,
                        0,
                        config.ott_bands[i],
                        0,
                        false,
                    )?);
                }
            }
        }
        for (i, t) in config.ttt.iter().enumerate() {
            let low_stop = t.high.map_or(config.parameter_bands, |(_, split)| split);
            for (high, start, stop, mode) in [
                (false, 0, low_stop, t.low_mode),
                (
                    true,
                    low_stop,
                    config.parameter_bands,
                    t.high.map_or(t.low_mode, |(m, _)| m),
                ),
            ] {
                if high && start == stop {
                    continue;
                }
                for component in 0..2 {
                    let (id, kind, default) = if mode < 2 {
                        (
                            ParameterId::TttCpc {
                                box_index: i as u8,
                                high,
                                component,
                            },
                            Kind::Cpc,
                            10,
                        )
                    } else {
                        (
                            ParameterId::TttCld {
                                box_index: i as u8,
                                high,
                                component,
                            },
                            Kind::Cld,
                            if component == 0 { 15 } else { 0 },
                        )
                    };
                    regions.push(Region::new(id, kind, start, stop, default, false)?);
                }
                if mode < 2 {
                    regions.push(Region::new(
                        ParameterId::TttIcc {
                            box_index: i as u8,
                            high,
                        },
                        Kind::Icc,
                        start,
                        stop,
                        0,
                        true,
                    )?);
                }
            }
        }
        let mut extension_regions = Vec::new();
        let mut residual = false;
        let mut downmix = config.arbitrary_downmix;
        for (i, ext) in config.extensions.iter().enumerate() {
            if ext.kind > 15
                || Config::extension(ext.kind, &ext.payload, &config.tree, config.parameter_bands)?
                    != ext.value
            {
                return Err(invalid("inconsistent LD-MPS frame extension configuration"));
            }
            let mut ext_regions = Vec::new();
            match &ext.value {
                ExtensionValue::Residual { .. } => residual = true,
                ExtensionValue::DownmixResidual { .. } => downmix = true,
                ExtensionValue::ArbitraryTree { ott, .. } => {
                    for (j, o) in ott.iter().enumerate() {
                        ext_regions.push(Region::new(
                            ParameterId::ArbitraryCld {
                                extension: i,
                                box_index: j,
                            },
                            Kind::Cld,
                            0,
                            o.bands,
                            i16::from(o.default_cld),
                            false,
                        )?);
                    }
                }
                ExtensionValue::Unknown => {}
            }
            extension_regions.push(ext_regions);
        }
        let main_regions = regions.len();
        if downmix {
            for ch in 0..config.tree.input_channels {
                regions.push(Region::new(
                    ParameterId::DownmixGain(ch),
                    Kind::Cld,
                    0,
                    config.parameter_bands,
                    0,
                    false,
                )?);
            }
        }
        Ok(Self {
            config: config.clone(),
            regions,
            main_regions,
            extension_regions,
            residual,
        })
    }
    pub fn reset(&mut self) {
        for r in self
            .regions
            .iter_mut()
            .chain(self.extension_regions.iter_mut().flatten())
        {
            r.state.reset();
        }
    }
    /// Self-delimited frame syntax. Alignment is relative to the beginning of
    /// this frame, including when its parent begins at a non-byte offset.
    /// The caller must bound the reader to its enclosing payload. A failure in
    /// any later tool/extension leaves all histories and the cursor unchanged.
    pub fn decode(&mut self, bits: &mut BitReader<'_>) -> Result<Frame> {
        let mut next = self.clone();
        let mut b = bits.clone();
        let frame = next.read(&mut b)?;
        *self = next;
        *bits = b;
        Ok(frame)
    }
    fn read(&mut self, b: &mut BitReader<'_>) -> Result<Frame> {
        let origin = b.position();
        let variable = b.bit()?;
        let sets = usize::from(b.bit()?) + 1;
        let time = self.config.frame_slots;
        let width = (u8::BITS - (time - 1).leading_zeros()) as u8;
        let mut slots = Vec::with_capacity(sets);
        for i in 0..sets {
            let slot = if variable {
                b.read(width)? as u8
            } else {
                (usize::from(time) * (i + 1)).div_ceil(sets) as u8 - 1
            };
            if slot >= time || slots.last().is_some_and(|&last| last >= slot) {
                return Err(invalid("invalid LD-MPS frame parameter slots"));
            }
            slots.push(slot);
        }
        let independent = b.bit()?;
        let mut parameters = Vec::new();
        for r in &mut self.regions[..self.main_regions] {
            parameters.push(r.read(b, &slots, time, independent, self.residual)?);
        }
        let mut smoothing = Vec::with_capacity(sets);
        for _ in 0..sets {
            let mode = b.read(2)? as u8;
            let time = if mode >= 2 {
                Some(b.read(2)? as u8)
            } else {
                None
            };
            let stride = if mode == 3 {
                Some([1, 2, 5, 23][b.read(2)? as usize])
            } else {
                None
            };
            let groups = if let Some(stride) = stride {
                (0..usize::from(self.config.parameter_bands).div_ceil(stride as usize))
                    .map(|_| b.bit())
                    .collect::<Result<_>>()?
            } else {
                Vec::new()
            };
            smoothing.push(Smoothing {
                mode,
                time,
                stride,
                groups,
            });
        }
        let temporal_shape = if self.config.temporal_shape == 0 || !b.bit()? {
            TemporalShape::Disabled
        } else {
            let count = [[5, 5, 4, 6, 6, 4, 4, 2], [5, 5, 5, 7, 7, 4, 4, 2]]
                [usize::from(self.config.temporal_shape - 1)][usize::from(self.config.tree.code)];
            let enabled = (0..count).map(|_| b.bit()).collect::<Result<Vec<_>>>()?;
            if self.config.temporal_shape == 1 {
                TemporalShape::Stp(enabled)
            } else {
                let mut channels = Vec::new();
                for on in enabled {
                    channels.push(if on { Some(reshape(b, time)?) } else { None });
                }
                TemporalShape::Ges(channels)
            }
        };
        for r in &mut self.regions[self.main_regions..] {
            parameters.push(r.read(b, &slots, time, independent, self.residual)?);
        }
        b.skip((8 - (b.position() - origin) % 8) % 8)?;
        let mut extensions = Vec::new();
        for (i, ext) in self.config.extensions.iter().enumerate() {
            let mut payload = Vec::new();
            let mut rows = Vec::new();
            if ext.kind < 12 {
                let mut length = b.read(8)? as usize;
                if length == 255 {
                    length += b.read(16)? as usize;
                }
                if length > b.remaining() / 8 {
                    return Err(invalid("truncated LD-MPS frame extension"));
                }
                payload = (0..length)
                    .map(|_| b.read(8).map(|v| v as u8))
                    .collect::<Result<_>>()?;
                let mut child = BitReader::new(&payload);
                for r in &mut self.extension_regions[i] {
                    rows.push(r.read(&mut child, &slots, time, independent, self.residual)?);
                }
            }
            extensions.push(ExtensionFrame {
                kind: ext.kind,
                payload,
                parameters: rows,
            });
        }
        Ok(Frame {
            slots,
            independent,
            parameters,
            smoothing,
            temporal_shape,
            extensions,
        })
    }
}
fn reshape(bits: &mut BitReader<'_>, slots: u8) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(usize::from(slots));
    while out.len() < usize::from(slots) {
        let symbol = entropy::symbol(bits, RESHAPE)? as u8;
        let value = symbol >> 4;
        let count = usize::from(symbol & 15) + 1;
        if count > usize::from(slots) - out.len() {
            return Err(invalid("LD-MPS reshape run exceeds frame slots"));
        }
        out.extend(std::iter::repeat_n(value, count));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_reshape_value_run_codewords_preserve_following_bits_at_every_offset() {
        assert_eq!(RESHAPE.len(), 40);
        for &(width, code, symbol) in RESHAPE {
            let count = (symbol as u8 & 15) + 1;
            let value = symbol as u8 >> 4;
            for at in 0..8 {
                let n = at + usize::from(width);
                let mut bytes = vec![0; (n + 8).div_ceil(8)];
                for i in 0..usize::from(width) {
                    bytes[(at + i) / 8] |=
                        (((code >> (usize::from(width) - i - 1)) & 1) as u8) << (7 - (at + i) % 8);
                }
                for i in 0..8 {
                    bytes[(n + i) / 8] |= ((0x5au8 >> (7 - i)) & 1) << (7 - (n + i) % 8);
                }
                let mut b = BitReader::new(&bytes);
                b.skip(at).unwrap();
                assert_eq!(
                    reshape(&mut b, count).unwrap(),
                    vec![value; usize::from(count)]
                );
                assert_eq!(b.position(), n);
                assert_eq!(b.read(8).unwrap(), 0x5a);
            }
        }
    }
}

impl Decoder {
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        self.config.visit_retained(f)?;
        f.vector(&self.regions)?;
        for region in &self.regions {
            region.state.visit_retained(f)?;
        }
        f.vector(&self.extension_regions)?;
        for regions in &self.extension_regions {
            f.vector(regions)?;
            for region in regions {
                region.state.visit_retained(f)?;
            }
        }
        Ok(())
    }
}
