//! Physical LD-MPS parameters and matrix-smoothing controls, before upmix.
//! This does not produce spatial PCM or accept opaque residual audio.
use super::{
    Result,
    aac_ld_mps_config::{Config, ExtensionValue},
    aac_ld_mps_entropy::Kind,
    aac_ld_mps_frame::{self as frame, Frame, ParameterId, Smoothing, TemporalShape},
    aac_ld_mps_prepare_facts as facts,
    bits::BitReader,
    invalid, unsupported,
};

pub fn dequantize(kind: Kind, index: i16) -> Result<f64> {
    let (grid, offset): (&[f64], i32) = match kind {
        Kind::Cld => (&facts::CLD, 15),
        Kind::Icc => (&facts::ICC, 0),
        Kind::Cpc => (&facts::CPC, 20),
    };
    grid.get((i32::from(index) + offset) as usize)
        .copied()
        .ok_or_else(|| invalid("LD-MPS physical index outside quantization grid"))
}
/// Native parameter-band assignment for each QMF bin. The 23-band expansion
/// used by optional stereo downmix is a separate map, not a substitute here.
pub fn qmf_map(parameter_bands: u8, qmf_bands: usize) -> Result<Vec<u8>> {
    let map = match parameter_bands {
        4 => &facts::QMF4,
        5 => &facts::QMF5,
        7 => &facts::QMF7,
        9 => &facts::QMF9,
        12 => &facts::QMF12,
        15 => &facts::QMF15,
        23 => &facts::QMF23,
        _ => return Err(invalid("invalid LD-MPS parameter-band resolution")),
    };
    if !matches!(qmf_bands, 32 | 64 | 128) {
        return Err(invalid("invalid LD-MPS QMF geometry"));
    }
    Ok(map[..qmf_bands].to_vec())
}
pub fn expansion23(parameter_bands: u8) -> Result<&'static [u8; 23]> {
    Ok(match parameter_bands {
        4 => &facts::MAP4,
        5 => &facts::MAP5,
        7 => &facts::MAP7,
        9 => &facts::MAP9,
        12 => &facts::MAP12,
        15 => &facts::MAP15,
        23 => &facts::MAP23,
        _ => return Err(invalid("invalid LD-MPS parameter-band resolution")),
    })
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnergySplit {
    pub index: i16,
    pub cld_db: f64,
    pub left_db: f64,
    pub right_db: f64,
}
/// Index-domain energy quantization and stable log-domain energy propagation.
/// Raw entropy history must never be overwritten with the adjusted index.
pub fn energy_split(index: i16, input_db: f64, mode: u8) -> Result<EnergySplit> {
    dequantize(Kind::Cld, index)?;
    if !input_db.is_finite() || input_db > 0.0 || mode > 2 {
        return Err(invalid("invalid LD-MPS energy quantization input"));
    }
    let factor = match mode {
        0 => 1.0,
        1 => (1.0 + (-input_db - 1.0).max(0.0) / 5.0).min(5.0),
        2 => (1.0 + (-input_db - 1.0).max(0.0) * 7.0 / 24.0).min(8.0),
        _ => unreachable!(),
    };
    let adjusted = ((f64::from(index) * factor + 15.5).floor() - 15.0).clamp(-15.0, 15.0) as i16;
    let cld_db = dequantize(Kind::Cld, adjusted)?;
    let z = cld_db * std::f64::consts::LN_10 / 10.0;
    let softplus = |x: f64| x.max(0.0) + (-x.abs()).exp().ln_1p();
    let scale = 10.0 / std::f64::consts::LN_10;
    Ok(EnergySplit {
        index: adjusted,
        cld_db,
        left_db: input_db - scale * softplus(-z),
        right_db: input_db - scale * softplus(z),
    })
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmoothControl {
    pub time: u16,
    pub mask: Vec<bool>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmoothState {
    previous: SmoothControl,
}
impl SmoothState {
    pub fn new(bands: u8) -> Result<Self> {
        expansion23(bands)?;
        // A leading keep has no active smoothing, and does not divide by this
        // zero initial time. Mode zero explicitly selects 256/inactive instead.
        Ok(Self {
            previous: SmoothControl {
                time: 0,
                mask: vec![false; usize::from(bands)],
            },
        })
    }
    pub fn reset(&mut self) {
        self.previous.time = 0;
        self.previous.mask.fill(false);
    }
    pub fn decode(&mut self, syntax: &[Smoothing]) -> Result<Vec<SmoothControl>> {
        if !(1..=2).contains(&syntax.len()) {
            return Err(invalid("invalid LD-MPS smoothing set count"));
        }
        let bands = self.previous.mask.len();
        let mut previous = self.previous.clone();
        let mut rows = Vec::new();
        for s in syntax {
            let next = match s.mode {
                0 if s.time.is_none() && s.stride.is_none() && s.groups.is_empty() => {
                    SmoothControl {
                        time: 256,
                        mask: vec![false; bands],
                    }
                }
                1 if s.time.is_none() && s.stride.is_none() && s.groups.is_empty() => {
                    previous.clone()
                }
                2 | 3 => {
                    let time = *facts::SMOOTH_TIMES
                        .get(usize::from(
                            s.time
                                .ok_or_else(|| invalid("missing LD-MPS smoothing time"))?,
                        ))
                        .ok_or_else(|| invalid("invalid LD-MPS smoothing time"))?;
                    let mask = if s.mode == 2 {
                        if s.stride.is_some() || !s.groups.is_empty() {
                            return Err(invalid("unexpected LD-MPS smoothing mask"));
                        }
                        vec![true; bands]
                    } else {
                        let stride = usize::from(
                            s.stride
                                .ok_or_else(|| invalid("missing LD-MPS smoothing stride"))?,
                        );
                        if ![1, 2, 5, 23].contains(&stride)
                            || s.groups.len() != bands.div_ceil(stride)
                        {
                            return Err(invalid("invalid LD-MPS smoothing mask geometry"));
                        }
                        let count = s.groups.len();
                        let short = count * stride - bands;
                        s.groups
                            .iter()
                            .enumerate()
                            .flat_map(|(i, &on)| {
                                std::iter::repeat_n(
                                    on,
                                    stride - short / count - usize::from(i < short % count),
                                )
                            })
                            .collect()
                    };
                    SmoothControl { time, mask }
                }
                _ => return Err(invalid("invalid LD-MPS smoothing fields")),
            };
            previous = next.clone();
            rows.push(next);
        }
        self.previous = previous;
        Ok(rows)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Levels {
    pub id: ParameterId,
    pub kind: Kind,
    pub start_band: u8,
    pub indices: Vec<Vec<i16>>,
    pub values: Vec<Vec<f64>>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Disabled,
    Stp(Vec<bool>),
    Ges(Vec<Option<Vec<f64>>>),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    pub syntax: Frame,
    /// Includes a held final endpoint when the encoded last slot is nonfinal.
    pub slots: Vec<u8>,
    pub parameters: Vec<Levels>,
    pub smoothing: Vec<SmoothControl>,
    pub temporal_shape: Shape,
    pub residual_cutoff: u8,
    pub qmf_bands: usize,
    pub qmf_parameter_bands: Vec<u8>,
    pub surround_gain: f64,
    pub lfe_gain: f64,
    pub clip_protection_gain: f64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoder {
    config: Config,
    frames: frame::Decoder,
    smoothing: SmoothState,
}
impl Decoder {
    pub fn new(config: &Config) -> Result<Self> {
        if config.sample_rate == 0 {
            return Err(invalid("zero LD-MPS physical sample rate"));
        }
        let frames = frame::Decoder::new(config)?;
        // The reference qualifies EBQ on the two 1->5.1 trees. The 2->5.1
        // predictive TTT path does not apply EBQ to OTTs; other nonzero-quant
        // combinations need their normative semantics before renderer use.
        let qualified_tree = config.tree.code <= 1
            || config.tree.code == 2
                && config
                    .ttt
                    .iter()
                    .all(|t| t.low_mode < 2 && t.high.is_none_or(|(m, _)| m < 2));
        let active_arbitrary = config.extensions.iter().any(|e| matches!(&e.value, ExtensionValue::ArbitraryTree { ott, .. } if ott.iter().any(|o| o.bands != 0)));
        if config.quantization_mode != 0 && (!qualified_tree || active_arbitrary) {
            return Err(unsupported(
                "LD-MPS energy quantization for this tree/TTT/arbitrary-tree configuration is not yet qualified",
            ));
        }
        for (index, grid) in [
            (config.surround_gain, &facts::SURROUND[..]),
            (config.lfe_gain, &facts::LFE[..]),
            (Some(config.downmix_gain), &facts::DOWNMIX[..]),
        ] {
            if index.is_some_and(|i| usize::from(i) >= grid.len()) {
                return Err(invalid("invalid LD-MPS physical gain index"));
            }
        }
        Ok(Self {
            config: config.clone(),
            frames,
            smoothing: SmoothState::new(config.parameter_bands)?,
        })
    }
    pub fn reset(&mut self) {
        self.frames.reset();
        self.smoothing.reset();
    }
    /// Parse and prepare in a single transaction. A later physical validation
    /// failure cannot advance entropy histories, smoothing state or the cursor.
    pub fn decode(&mut self, bits: &mut BitReader<'_>) -> Result<Prepared> {
        let mut next = self.clone();
        let mut b = bits.clone();
        let syntax = next.frames.decode(&mut b)?;
        let output = next.prepare(syntax)?;
        *self = next;
        *bits = b;
        Ok(output)
    }
    fn prepare(&mut self, syntax: Frame) -> Result<Prepared> {
        let mut parameters = Vec::new();
        for p in syntax
            .parameters
            .iter()
            .chain(syntax.extensions.iter().flat_map(|e| &e.parameters))
        {
            let values = p
                .values
                .iter()
                .map(|row| row.iter().map(|&v| dequantize(p.kind, v)).collect())
                .collect::<Result<_>>()?;
            parameters.push(Levels {
                id: p.id.clone(),
                kind: p.kind,
                start_band: p.start_band,
                indices: p.values.clone(),
                values,
            });
        }
        if self.config.quantization_mode != 0 && self.config.tree.code <= 1 {
            let sets = syntax.slots.len();
            let bands = usize::from(self.config.parameter_bands);
            let mut energy = vec![vec![vec![0.0; bands]; sets]; 5];
            // Edges identify the left/right child of an earlier split. LFE is
            // independently rooted; it must not inherit another branch's level.
            let parents = if self.config.tree.code == 0 {
                [
                    None,
                    Some((0, true)),
                    Some((0, false)),
                    Some((1, true)),
                    None,
                ]
            } else {
                [
                    None,
                    Some((0, true)),
                    None,
                    Some((1, true)),
                    Some((1, false)),
                ]
            };
            let mut right = energy.clone();
            for box_index in 0..5 {
                let p = parameters
                    .iter_mut()
                    .find(|p| p.id == ParameterId::OttCld(box_index as u8))
                    .ok_or_else(|| invalid("missing LD-MPS OTT physical region"))?;
                for ps in 0..sets {
                    for band in 0..p.indices[ps].len() {
                        let parent = parents[box_index].map_or(0.0, |(node, left)| {
                            if left {
                                energy[node][ps][band]
                            } else {
                                right[node][ps][band]
                            }
                        });
                        let split = energy_split(
                            p.indices[ps][band],
                            parent,
                            self.config.quantization_mode,
                        )?;
                        p.indices[ps][band] = split.index;
                        p.values[ps][band] = split.cld_db;
                        energy[box_index][ps][band] = split.left_db;
                        right[box_index][ps][band] = split.right_db;
                    }
                }
            }
        }
        let mut smoothing = self.smoothing.decode(&syntax.smoothing)?;
        let mut slots = syntax.slots.clone();
        if slots.last().copied() != Some(self.config.frame_slots - 1) {
            slots.push(self.config.frame_slots - 1);
            smoothing.push(smoothing.last().unwrap().clone());
            for p in &mut parameters {
                p.indices.push(p.indices.last().unwrap().clone());
                p.values.push(p.values.last().unwrap().clone());
            }
        }
        let temporal_shape = match &syntax.temporal_shape {
            TemporalShape::Disabled => Shape::Disabled,
            TemporalShape::Stp(on) => Shape::Stp(on.clone()),
            TemporalShape::Ges(ch) => Shape::Ges(
                ch.iter()
                    .map(|row| {
                        row.as_ref().map(|row| {
                            row.iter()
                                .map(|&v| 2.0f64.powf(f64::from(v) / 2.0 - 1.0))
                                .collect()
                        })
                    })
                    .collect(),
            ),
        };
        let residual_cutoff = self
            .config
            .extensions
            .iter()
            .map(|e| match &e.value {
                ExtensionValue::Residual { bands, .. } => {
                    bands.iter().flatten().copied().max().unwrap_or(0)
                }
                ExtensionValue::DownmixResidual { bands, .. } => *bands,
                _ => 0,
            })
            .max()
            .unwrap_or(0);
        let qmf_bands = if self.config.sample_rate < 27713 {
            32
        } else if self.config.sample_rate >= 55426 {
            128
        } else {
            64
        };
        Ok(Prepared {
            slots,
            parameters,
            smoothing,
            temporal_shape,
            residual_cutoff,
            qmf_bands,
            qmf_parameter_bands: qmf_map(self.config.parameter_bands, qmf_bands)?,
            surround_gain: self
                .config
                .surround_gain
                .map_or(1.0, |i| facts::SURROUND[usize::from(i)]),
            lfe_gain: self
                .config
                .lfe_gain
                .map_or(1.0, |i| facts::LFE[usize::from(i)]),
            clip_protection_gain: facts::DOWNMIX[usize::from(self.config.downmix_gain)],
            syntax,
        })
    }
}
