//! Transactional spatial-frame -> M1 -> decorrelation -> W -> M2 QMF chain.
//! Decoded residual QMF must be supplied. GES acts on dry rows here; fixed
//! output gains, STP and PCM synthesis are downstream stages.
use super::{
    Result, aac_ld_mps_assemble,
    aac_ld_mps_config::Config,
    aac_ld_mps_decor::{Decorrelator, Mode},
    aac_ld_mps_matrix_apply::{self, Samples},
    aac_ld_mps_prepare::Prepared,
    aac_sbr_qmf::Complex,
    bits::BitReader,
    invalid,
};
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    /// GES configurations include guided shaping; prepared retains its signalling.
    pub dry: Samples,
    pub wet: Samples,
    pub prepared: Prepared,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Decoder {
    config: Config,
    ges: Option<super::aac_ld_mps_ges::Decoder>,
    matrices: aac_ld_mps_assemble::Decoder,
    decorrelators: Vec<Decorrelator>,
    qmf_bands: usize,
    direct: usize,
}
/// Full-complex reference seed schedule (distinct from partially-complex
/// alias-lock schedules). Tree2's TTT branch uses seed3; stereo uses seed2.
pub fn seeds(tree: u8) -> Result<&'static [u8]> {
    Ok(match tree {
        0 | 1 => &[0, 1, 2, 3],
        2 => &[0, 1, 3],
        3 | 4 => &[0, 1, 2, 3, 4],
        5 | 6 => &[0, 1],
        7 => &[2],
        _ => return Err(invalid("invalid LD-MPS QMF tree")),
    })
}
impl Decoder {
    pub fn new(config: &Config, mode: Mode) -> Result<Self> {
        let matrices = aac_ld_mps_assemble::Decoder::new(config)?;
        let qmf_bands = if config.sample_rate < 27713 {
            32
        } else if config.sample_rate >= 55426 {
            128
        } else {
            64
        };
        let decorrelators = seeds(config.tree.code)?
            .iter()
            .map(|&seed| Decorrelator::new(qmf_bands, seed, config.decorrelation, mode))
            .collect::<Result<Vec<_>>>()?;
        let direct = match config.tree.code {
            0 | 1 | 7 => 1,
            2..=4 => 3,
            _ => 6,
        };
        Ok(Self {
            config: config.clone(),
            ges: if config.temporal_shape == 2 {
                Some(super::aac_ld_mps_ges::Decoder::new(
                    config.tree.code,
                    qmf_bands,
                )?)
            } else {
                None
            },
            matrices,
            decorrelators,
            qmf_bands,
            direct,
        })
    }
    pub(crate) fn reconfigure(&mut self, config: &Config, mode: Mode) -> Result<()> {
        if config == &self.config {
            return Ok(());
        }
        let mut next = Self::new(config, mode)?;
        if next.ges.is_some() && self.ges.is_some() {
            next.ges = self.ges.clone();
        }
        next.matrices = self.matrices.clone();
        next.matrices.reconfigure(config)?;
        if config.decorrelation == self.config.decorrelation {
            next.decorrelators = self.decorrelators.clone();
        }
        *self = next;
        Ok(())
    }
    pub fn reset(&mut self) {
        self.matrices.reset();
        if let Some(ges) = &mut self.ges {
            ges.reset();
        }
        for d in &mut self.decorrelators {
            d.reset();
        }
    }
    /// One complete configured spatial frame. All samples are normalized QMF
    /// units, indexed [slot][channel][bin]. Residual boxes are [slot][bin].
    /// A failure retains the reader, parser/matrix and all decorrelator histories.
    pub fn decode(
        &mut self,
        bits: &mut BitReader<'_>,
        downmix: &[Vec<Vec<Complex>>],
        residuals: &[Option<Vec<Vec<Complex>>>],
    ) -> Result<Output> {
        let mut next = self.clone();
        let mut reader = bits.clone();
        let matrices = next.matrices.decode(&mut reader)?;
        let slots = usize::from(next.config.frame_slots);
        let bins = next.qmf_bands;
        let finite = |v: &Complex| v.re.is_finite() && v.im.is_finite();
        if downmix.len() != slots
            || downmix.iter().any(|s| {
                s.len() != usize::from(next.config.tree.input_channels)
                    || s.iter().any(|c| c.len() != bins || !c.iter().all(finite))
            })
            || residuals.len() != matrices.residual_cutoffs.len()
            || residuals.iter().any(|r| {
                r.as_ref().is_some_and(|r| {
                    r.len() != slots || r.iter().any(|s| s.len() != bins || !s.iter().all(finite))
                })
            })
        {
            return Err(invalid("invalid LD-MPS downmix/residual QMF inputs"));
        }
        let mut input = downmix.to_vec();
        if next.config.tree.ttt_boxes != 0 {
            let box_index = next.config.tree.ott_lfe.len();
            let cutoff = matrices.residual_cutoffs[box_index];
            if cutoff != 0 && residuals[box_index].is_none() {
                return Err(invalid("missing decoded LD-MPS TTT residual QMF input"));
            }
            for t in 0..slots {
                let mut channel = vec![Complex::default(); bins];
                for (q, &band) in matrices.prepared.qmf_parameter_bands.iter().enumerate() {
                    if band < cutoff {
                        channel[q] = residuals[box_index].as_ref().unwrap()[t][q];
                    }
                }
                input[t].push(channel);
            }
        }
        let virtuals = aac_ld_mps_matrix_apply::apply(
            &matrices.m1,
            matrices.m1_layout,
            next.config.parameter_bands,
            bins,
            0,
            Some(1),
            &input,
        )?;
        let channels = matrices.router.channels();
        let mut decorrelated = vec![vec![vec![Complex::default(); bins]; channels]; slots];
        for (j, decorrelator) in next.decorrelators.iter_mut().enumerate() {
            let channel = next.direct + j;
            let source = virtuals
                .iter()
                .map(|slot| slot[channel].clone())
                .collect::<Vec<_>>();
            let out = decorrelator.process(&source)?;
            for (t, samples) in out.into_iter().enumerate() {
                decorrelated[t][channel] = samples;
            }
        }
        let routed = matrices
            .router
            .process(&virtuals, &decorrelated, residuals)?;
        let mut dry = aac_ld_mps_matrix_apply::apply(
            &matrices.dry,
            matrices.m2_layout,
            next.config.parameter_bands,
            bins,
            0,
            None,
            &routed.dry,
        )?;
        let wet = aac_ld_mps_matrix_apply::apply(
            &matrices.wet,
            matrices.m2_layout,
            next.config.parameter_bands,
            bins,
            0,
            None,
            &routed.wet,
        )?;
        if let Some(ges) = &mut next.ges {
            let envelopes = match &matrices.prepared.temporal_shape {
                super::aac_ld_mps_prepare::Shape::Ges(e) => Some(e.as_slice()),
                super::aac_ld_mps_prepare::Shape::Disabled => None,
                _ => return Err(invalid("unexpected STP frame in LD-MPS GES context")),
            };
            ges.apply(downmix, &mut dry, &wet, envelopes)?;
        }
        let result = Output {
            dry,
            wet,
            prepared: matrices.prepared,
        };
        *self = next;
        *bits = reader;
        Ok(result)
    }
}

impl Decoder {
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        self.config.visit_retained(f)?;
        if let Some(ges) = &self.ges {
            ges.visit_retained(f)?;
        }
        self.matrices.visit_retained(f)?;
        f.vector(&self.decorrelators)?;
        for d in &self.decorrelators {
            d.visit_retained(f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod inband_state_tests {
    use super::*;
    #[test]
    fn fixed_gain_keeps_decorrelation_and_matrix_history_but_new_design_clears_decorrelation() {
        let config = Config::parse(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-eld-mps-core/tree7.ssc"
        ))
        .unwrap();
        let packet = include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-eld-mps-core/tree7-0.spatial"
        );
        let rows: Vec<_> = (0..config.frame_slots)
            .map(|t| {
                vec![
                    (0..64)
                        .map(|b| Complex {
                            re: ((t as usize * 64 + b) as f64 * 0.13).sin() * 0.01,
                            im: 0.003,
                        })
                        .collect(),
                ]
            })
            .collect();
        let mut decoder = Decoder::new(&config, Mode::Complex).unwrap();
        decoder
            .decode(&mut BitReader::new(packet), &rows, &[None])
            .unwrap();
        let mut uninterrupted = decoder.clone();
        let prior = decoder.clone();
        decoder.reconfigure(&config, Mode::Complex).unwrap();
        assert_eq!(decoder, prior);
        let mut gain = config.clone();
        gain.downmix_gain = (gain.downmix_gain + 1) % 8;
        decoder.reconfigure(&gain, Mode::Complex).unwrap();
        assert_eq!(decoder.decorrelators, prior.decorrelators);
        let a = decoder
            .decode(&mut BitReader::new(packet), &rows, &[None])
            .unwrap();
        let b = uninterrupted
            .decode(&mut BitReader::new(packet), &rows, &[None])
            .unwrap();
        assert_eq!(a.dry, b.dry);
        assert_eq!(a.wet, b.wet);
        let mut changed = gain.clone();
        changed.decorrelation = (changed.decorrelation + 1) % 3;
        let fresh = Decoder::new(&changed, Mode::Complex).unwrap();
        assert_ne!(decoder.decorrelators, fresh.decorrelators);
        decoder.reconfigure(&changed, Mode::Complex).unwrap();
        assert_eq!(decoder.decorrelators, fresh.decorrelators);
        let saved = decoder.clone();
        let mut invalid = changed;
        invalid.sample_rate = 24000;
        assert!(decoder.reconfigure(&invalid, Mode::Complex).is_err());
        assert_eq!(decoder, saved);
    }
}

#[cfg(test)]
mod ges_context_tests {
    use super::*;
    #[test]
    fn ges_histories_survive_same_domain_grid_changes_and_restart_after_shape_changes() {
        let mut config = Config::parse(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-eld-mps-core/tree7.ssc"
        ))
        .unwrap();
        config.temporal_shape = 2;
        let mut decoder = Decoder::new(&config, Mode::Complex).unwrap();
        let fresh = decoder.ges.clone();
        let input = vec![
            vec![vec![
                Complex {
                    re: 0.01,
                    im: 0.003
                };
                64
            ]];
            8
        ];
        let mut dry = vec![
            vec![
                vec![
                    Complex {
                        re: 0.005,
                        im: 0.001
                    };
                    64
                ];
                2
            ];
            8
        ];
        let wet = vec![vec![vec![Complex::default(); 64]; 2]; 8];
        decoder
            .ges
            .as_mut()
            .unwrap()
            .apply(&input, &mut dry, &wet, None)
            .unwrap();
        let warm = decoder.ges.clone();
        assert_ne!(warm, fresh);
        config.parameter_bands = 4;
        config.ott_bands.fill(4);
        decoder.reconfigure(&config, Mode::Complex).unwrap();
        assert_eq!(decoder.ges, warm);
        config.temporal_shape = 0;
        decoder.reconfigure(&config, Mode::Complex).unwrap();
        assert!(decoder.ges.is_none());
        config.temporal_shape = 2;
        decoder.reconfigure(&config, Mode::Complex).unwrap();
        assert_eq!(decoder.ges, fresh);
        decoder
            .ges
            .as_mut()
            .unwrap()
            .apply(&input, &mut dry, &wet, None)
            .unwrap();
        decoder.reset();
        assert_eq!(decoder.ges, fresh);
    }
}
