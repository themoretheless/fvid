//! Transactional ordinary spatial-frame matrix assembly. This prepares QMF
//! matrices; it neither decodes residual audio nor accepts spatial PCM playback.
use super::{
    Result,
    aac_ld_mps_config::{Config, ExtensionValue},
    aac_ld_mps_frame::ParameterId,
    aac_ld_mps_matrix_time::{Layout, MatrixFrame, MatrixTime},
    aac_ld_mps_prepare::{self, Prepared},
    aac_ld_mps_route::Router,
    aac_ld_mps_tree_matrix::{self, Band},
    aac_ld_mps_ttt_tree::{self, Ttt},
    bits::BitReader,
    invalid, unsupported,
};
#[derive(Clone, Debug, PartialEq)]
pub struct Assembled {
    pub prepared: Prepared,
    pub m1_layout: Layout,
    pub m2_layout: Layout,
    pub m1: MatrixFrame,
    pub dry: MatrixFrame,
    pub wet: MatrixFrame,
    pub router: Router,
    /// Per-native-band TTT modes; empty for OTT-only trees.
    pub ttt_modes: Vec<u8>,
    /// OTT cutoffs followed by TTT cutoff, also needed for the M1 input basis.
    pub residual_cutoffs: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Decoder {
    config: Config,
    physical: aac_ld_mps_prepare::Decoder,
    cutoffs: Vec<u8>,
    modes: Vec<u8>,
    m1: MatrixTime,
    dry: MatrixTime,
    wet: MatrixTime,
}
fn layouts(tree: u8) -> (Layout, Layout) {
    let (rows, inputs, outputs) = match tree {
        0 | 1 => (5, 1, 6),
        2 => (6, 3, 6),
        3 | 4 => (8, 3, 8),
        5 | 6 => (8, 6, 8),
        7 => (2, 1, 2),
        _ => unreachable!(),
    };
    (
        Layout {
            planes: 2,
            rows,
            columns: inputs,
        },
        Layout {
            planes: 1,
            rows: outputs,
            columns: rows,
        },
    )
}
fn value(p: &Prepared, id: ParameterId, set: usize, band: u8) -> Result<f64> {
    let region = p
        .parameters
        .iter()
        .find(|r| r.id == id)
        .ok_or_else(|| invalid("missing LD-MPS matrix parameter region"))?;
    let index = band
        .checked_sub(region.start_band)
        .ok_or_else(|| invalid("LD-MPS matrix parameter before region"))?;
    region
        .values
        .get(set)
        .and_then(|row| row.get(usize::from(index)))
        .copied()
        .filter(|v| v.is_finite())
        .ok_or_else(|| invalid("invalid LD-MPS matrix parameter geometry"))
}
impl Decoder {
    pub fn new(config: &Config) -> Result<Self> {
        let physical = aac_ld_mps_prepare::Decoder::new(config)?;
        if config.arbitrary_downmix
            || config.hrtf.is_some()
            || config.extensions.iter().any(|e| {
                matches!(
                    &e.value,
                    ExtensionValue::DownmixResidual { .. } | ExtensionValue::ArbitraryTree { .. }
                )
            })
        {
            return Err(unsupported(
                "LD-MPS arbitrary-downmix/HRTF matrix composition is not yet implemented",
            ));
        }
        let boxes = config.tree.ott_lfe.len() + usize::from(config.tree.ttt_boxes);
        let mut cutoffs = vec![0; boxes];
        let mut seen = false;
        for e in &config.extensions {
            if let ExtensionValue::Residual { bands, .. } = &e.value {
                if seen || bands.len() != boxes {
                    return Err(invalid("ambiguous LD-MPS residual matrix configuration"));
                }
                seen = true;
                cutoffs = bands.iter().map(|b| b.unwrap_or(0)).collect();
            }
        }
        let modes = if let Some(t) = config.ttt.first() {
            (0..config.parameter_bands)
                .map(|b| match t.high {
                    Some((mode, split)) if b >= split => mode,
                    _ => t.low_mode,
                })
                .collect()
        } else {
            vec![]
        };
        let qmf = if config.sample_rate < 27713 {
            32
        } else if config.sample_rate >= 55426 {
            128
        } else {
            64
        };
        Router::new(
            config.tree.code,
            config.parameter_bands,
            qmf,
            cutoffs.clone(),
            modes.clone(),
        )?;
        let (a, b) = layouts(config.tree.code);
        Ok(Self {
            config: config.clone(),
            physical,
            cutoffs,
            modes,
            m1: MatrixTime::new(config.parameter_bands, a)?,
            dry: MatrixTime::new(config.parameter_bands, b)?,
            wet: MatrixTime::new(config.parameter_bands, b)?,
        })
    }
    pub fn reset(&mut self) {
        self.physical.reset();
        self.m1.reset();
        self.dry.reset();
        self.wet.reset();
    }
    /// A later matrix error retains parameter/entropy/smoothing histories, all
    /// three matrix histories and the reader position. Clone is a full checkpoint.
    pub fn decode(&mut self, bits: &mut BitReader<'_>) -> Result<Assembled> {
        let mut next = self.clone();
        let mut reader = bits.clone();
        let prepared = next.physical.decode(&mut reader)?;
        let output = next.assemble(prepared)?;
        *self = next;
        *bits = reader;
        Ok(output)
    }
    fn assemble(&mut self, p: Prepared) -> Result<Assembled> {
        let config = &self.config;
        let tree = config.tree.code;
        let (m1_layout, m2_layout) = layouts(tree);
        let mut m1_raw = Vec::new();
        let mut dry_raw = Vec::new();
        let mut wet_raw = Vec::new();
        for set in 0..p.slots.len() {
            let mut m1 = Vec::new();
            let mut dry = Vec::new();
            let mut wet = Vec::new();
            for band in 0..config.parameter_bands {
                let mut boxes = Vec::new();
                for (b, &lfe) in config.tree.ott_lfe.iter().enumerate() {
                    let active = band < config.ott_bands[b];
                    let cld = if active {
                        value(&p, ParameterId::OttCld(b as u8), set, band)?
                    } else {
                        0.0
                    };
                    let coherence = if lfe {
                        1.0
                    } else {
                        value(
                            &p,
                            ParameterId::OttIcc(if config.one_icc == Some(true) {
                                None
                            } else {
                                Some(b as u8)
                            }),
                            set,
                            band,
                        )?
                    };
                    boxes.push(Band {
                        cld_db: cld,
                        coherence,
                        residual: band < self.cutoffs[b],
                    });
                }
                let lfe_active = config
                    .tree
                    .ott_lfe
                    .iter()
                    .position(|&v| v)
                    .is_some_and(|b| band < config.ott_bands[b]);
                if config.tree.ttt_boxes == 0 {
                    let a = aac_ld_mps_tree_matrix::build(tree, &boxes, lfe_active)?;
                    m1.extend(a.m1.iter().flatten().copied());
                    m1.extend(std::iter::repeat_n(
                        0.0,
                        usize::from(m1_layout.rows) * usize::from(m1_layout.columns),
                    ));
                    dry.extend(a.dry.iter().flatten().copied());
                    wet.extend(a.wet.iter().flatten().copied());
                } else {
                    let t = &config.ttt[0];
                    let high = t.high.is_some_and(|(_, split)| band >= split);
                    let mode = self.modes[usize::from(band)];
                    let id = |component| {
                        if mode < 2 {
                            ParameterId::TttCpc {
                                box_index: 0,
                                high,
                                component,
                            }
                        } else {
                            ParameterId::TttCld {
                                box_index: 0,
                                high,
                                component,
                            }
                        }
                    };
                    let parameters = [value(&p, id(0), set, band)?, value(&p, id(1), set, band)?];
                    let coherence = if mode < 2 {
                        value(&p, ParameterId::TttIcc { box_index: 0, high }, set, band)?
                    } else {
                        1.0
                    };
                    let a = aac_ld_mps_ttt_tree::build(
                        tree,
                        &boxes,
                        Ttt {
                            mode,
                            parameters,
                            coherence,
                            residual: band < self.cutoffs[boxes.len()],
                            inversion: config.matrix_mode == Some(true),
                        },
                        lfe_active,
                    )?;
                    m1.extend(a.m1.iter().flatten().map(|z| z.re));
                    m1.extend(a.m1.iter().flatten().map(|z| z.im));
                    dry.extend(a.dry.iter().flatten().copied());
                    wet.extend(a.wet.iter().flatten().copied());
                }
            }
            m1_raw.push(m1);
            dry_raw.push(dry);
            wet_raw.push(wet);
        }
        let slots = config.frame_slots;
        let m1 = self
            .m1
            .process(slots, &p.slots, &p.smoothing, &m1_raw, p.residual_cutoff)?;
        let dry = self
            .dry
            .process(slots, &p.slots, &p.smoothing, &dry_raw, p.residual_cutoff)?;
        let wet = self
            .wet
            .process(slots, &p.slots, &p.smoothing, &wet_raw, p.residual_cutoff)?;
        let router = Router::new(
            tree,
            config.parameter_bands,
            p.qmf_bands,
            self.cutoffs.clone(),
            self.modes.clone(),
        )?;
        Ok(Assembled {
            prepared: p,
            m1_layout,
            m2_layout,
            m1,
            dry,
            wet,
            router,
            ttt_modes: self.modes.clone(),
            residual_cutoffs: self.cutoffs.clone(),
        })
    }
}

impl Decoder {
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        self.config.visit_retained(f)?;
        self.physical.visit_retained(f)?;
        f.vector(&self.cutoffs)?;
        f.vector(&self.modes)?;
        self.m1.visit_retained(f)?;
        self.dry.visit_retained(f)?;
        self.wet.visit_retained(f)?;
        Ok(())
    }
}
