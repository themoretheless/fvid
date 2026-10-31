//! Ordinary LD-MPS PCM analysis/QMF chain/synthesis with canonical channel order.
//! Residual QMF must already be decoded. GES is applied by the owned QMF
//! chain; STP returns dry/wet rows ready for a single sum.
use super::{
    Result,
    aac_ld_mps_config::Config,
    aac_ld_mps_decor::Mode,
    aac_ld_mps_prepare_facts as facts, aac_ld_mps_qmf,
    aac_ld_qmf::{Analysis, Synthesis},
    aac_sbr_qmf::Complex,
    bits::BitReader,
    invalid, unsupported,
};
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub sample_rate: u32,
    pub channels: usize,
    /// Interleaved LF/RF/C/LFE/LS/RS[/AL/AR], or L/R for stereo.
    /// AL/AR retain the tree's additional-channel meaning; no speaker mask is guessed.
    pub samples: Vec<f64>,
}
#[derive(Clone, Debug, PartialEq)]
enum Banks {
    B32 {
        analysis: Vec<Analysis<32>>,
        synthesis: Vec<Synthesis<32>>,
    },
    B64 {
        analysis: Vec<Analysis<64>>,
        synthesis: Vec<Synthesis<64>>,
    },
    B128 {
        analysis: Vec<Analysis<128>>,
        synthesis: Vec<Synthesis<128>>,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct Decoder {
    qmf_input: bool,
    mode: Mode,
    qmf: aac_ld_mps_qmf::Decoder,
    banks: Banks,
    config: Config,
    order: Vec<usize>,
    gains: Vec<f64>,
    clip_gain: f64,
}
pub fn native_order(tree: u8) -> Result<&'static [usize]> {
    Ok(match tree {
        0 => &[0, 1, 2, 3, 4, 5],
        1 | 2 => &[0, 2, 4, 5, 1, 3],
        3..=6 => &[0, 3, 6, 7, 2, 5, 1, 4],
        7 => &[0, 1],
        _ => return Err(invalid("invalid LD-MPS PCM tree")),
    })
}
impl Decoder {
    pub fn new(config: &Config, mode: Mode) -> Result<Self> {
        Self::new_domain(config, mode, false)
    }
    /// Consume already-normalized spatial-domain QMF (e.g. from LD-SBR).
    /// The sole PCM synthesis bank remains here; no analysis round trip runs.
    pub fn new_qmf(config: &Config, mode: Mode) -> Result<Self> {
        Self::new_domain(config, mode, true)
    }
    fn new_domain(config: &Config, mode: Mode, qmf_input: bool) -> Result<Self> {
        let qmf = aac_ld_mps_qmf::Decoder::new(config, mode)?;
        let order = native_order(config.tree.code)?.to_vec();
        let inputs = if qmf_input {
            0
        } else {
            usize::from(config.tree.input_channels)
        };
        let outputs = order.len();
        let banks = if config.sample_rate < 27713 {
            Banks::B32 {
                analysis: (0..inputs)
                    .map(|_| Analysis::new())
                    .collect::<Result<_>>()?,
                synthesis: (0..outputs)
                    .map(|_| Synthesis::new())
                    .collect::<Result<_>>()?,
            }
        } else if config.sample_rate >= 55426 {
            Banks::B128 {
                analysis: (0..inputs)
                    .map(|_| Analysis::new())
                    .collect::<Result<_>>()?,
                synthesis: (0..outputs)
                    .map(|_| Synthesis::new())
                    .collect::<Result<_>>()?,
            }
        } else {
            Banks::B64 {
                analysis: (0..inputs)
                    .map(|_| Analysis::new())
                    .collect::<Result<_>>()?,
                synthesis: (0..outputs)
                    .map(|_| Synthesis::new())
                    .collect::<Result<_>>()?,
            }
        };
        let mut gains = vec![1.0; outputs];
        if outputs > 2 {
            gains[3] = facts::LFE[usize::from(config.lfe_gain.unwrap_or(0))];
            for index in [4, 5] {
                gains[index] = facts::SURROUND[usize::from(config.surround_gain.unwrap_or(0))];
            }
            if matches!(config.tree.code, 4 | 6) {
                gains[6] = gains[4];
                gains[7] = gains[4];
            }
        }
        Ok(Self {
            qmf_input,
            mode,
            qmf,
            banks,
            config: config.clone(),
            order,
            gains,
            clip_gain: facts::DOWNMIX[usize::from(config.downmix_gain)],
        })
    }
    pub(crate) fn enable_native_residuals(&mut self) -> Result<()> {
        self.qmf.enable_native_residuals(self.mode)
    }
    /// Apply a same-format in-band SSC without erasing unrelated filter state.
    /// Fixed downmix-gain changes clear synthesis only. The analysis window,
    /// unchanged decorrelators and matrix interpolation histories continue.
    pub fn reconfigure(&mut self, config: &Config) -> Result<()> {
        if config == &self.config {
            return Ok(());
        }
        if config.sample_rate != self.config.sample_rate || config.tree != self.config.tree {
            return Err(unsupported(
                "LD-MPS in-band output format change requires stream reconfiguration",
            ));
        }
        let mut next = Self::new_domain(config, self.mode, self.qmf_input)?;
        next.qmf = self.qmf.clone();
        next.qmf.reconfigure(config, self.mode)?;
        let keep_synthesis = config.downmix_gain == self.config.downmix_gain;
        match (&mut next.banks, &self.banks) {
            (
                Banks::B32 {
                    analysis: a,
                    synthesis: s,
                },
                Banks::B32 {
                    analysis: old_a,
                    synthesis: old_s,
                },
            ) => {
                *a = old_a.clone();
                if keep_synthesis {
                    *s = old_s.clone();
                }
            }
            (
                Banks::B64 {
                    analysis: a,
                    synthesis: s,
                },
                Banks::B64 {
                    analysis: old_a,
                    synthesis: old_s,
                },
            ) => {
                *a = old_a.clone();
                if keep_synthesis {
                    *s = old_s.clone();
                }
            }
            (
                Banks::B128 {
                    analysis: a,
                    synthesis: s,
                },
                Banks::B128 {
                    analysis: old_a,
                    synthesis: old_s,
                },
            ) => {
                *a = old_a.clone();
                if keep_synthesis {
                    *s = old_s.clone();
                }
            }
            _ => return Err(invalid("LD-MPS in-band filter bank changed")),
        }
        *self = next;
        Ok(())
    }
    pub fn reset(&mut self) {
        self.qmf.reset();
        match &mut self.banks {
            Banks::B32 {
                analysis,
                synthesis,
            } => {
                for a in analysis {
                    a.reset();
                }
                for s in synthesis {
                    s.reset();
                }
            }
            Banks::B64 {
                analysis,
                synthesis,
            } => {
                for a in analysis {
                    a.reset();
                }
                for s in synthesis {
                    s.reset();
                }
            }
            Banks::B128 {
                analysis,
                synthesis,
            } => {
                for a in analysis {
                    a.reset();
                }
                for s in synthesis {
                    s.reset();
                }
            }
        }
    }
    /// Exactly one configured frame, planar normalized decoded downmix PCM before
    /// clip protection. Residual inputs are decoded QMF, [box][slot][bin]. They
    /// are not multiplied by the downmix clip gain. All stages/cursor are atomic.
    pub fn decode(
        &mut self,
        bits: &mut BitReader<'_>,
        downmix: &[Vec<f64>],
        residuals: &[Option<Vec<Vec<Complex>>>],
    ) -> Result<Frame> {
        if self.qmf_input {
            return Err(invalid("QMF LD-MPS decoder requires subband input"));
        }
        let mut next = self.clone();
        let mut reader = bits.clone();
        let slots = usize::from(next.config.frame_slots);
        let samples = match &mut next.banks {
            Banks::B32 {
                analysis,
                synthesis,
            } => render(
                &mut next.qmf,
                &mut reader,
                downmix,
                residuals,
                slots,
                analysis,
                synthesis,
                &next.order,
                &next.gains,
                next.clip_gain,
            )?,
            Banks::B64 {
                analysis,
                synthesis,
            } => render(
                &mut next.qmf,
                &mut reader,
                downmix,
                residuals,
                slots,
                analysis,
                synthesis,
                &next.order,
                &next.gains,
                next.clip_gain,
            )?,
            Banks::B128 {
                analysis,
                synthesis,
            } => render(
                &mut next.qmf,
                &mut reader,
                downmix,
                residuals,
                slots,
                analysis,
                synthesis,
                &next.order,
                &next.gains,
                next.clip_gain,
            )?,
        };
        let frame = Frame {
            sample_rate: next.config.sample_rate,
            channels: next.order.len(),
            samples,
        };
        *self = next;
        *bits = reader;
        Ok(frame)
    }
}
impl Decoder {
    /// Subbands are [slot][canonical downmix channel][band]. Clip protection
    /// applies to the decoded SBR signal before spatial matrices; supplied
    /// residuals retain their own normalized scale.
    pub fn decode_qmf(
        &mut self,
        bits: &mut BitReader<'_>,
        downmix: &[Vec<Vec<Complex>>],
        residuals: &[Option<Vec<Vec<Complex>>>],
    ) -> Result<Frame> {
        if !self.qmf_input {
            return Err(invalid("PCM LD-MPS decoder requires PCM input"));
        }
        let slots = usize::from(self.config.frame_slots);
        let bins = if self.config.sample_rate < 27713 {
            32
        } else if self.config.sample_rate >= 55426 {
            128
        } else {
            64
        };
        if downmix.len() != slots
            || downmix.iter().any(|s| {
                s.len() != usize::from(self.config.tree.input_channels)
                    || s.iter().any(|c| {
                        c.len() != bins || c.iter().any(|v| !v.re.is_finite() || !v.im.is_finite())
                    })
            })
        {
            return Err(invalid("invalid LD-MPS shared QMF frame"));
        }
        let mut next = self.clone();
        let mut reader = bits.clone();
        let mut scaled = downmix.to_vec();
        for slot in &mut scaled {
            for c in slot {
                for v in c {
                    v.re *= next.clip_gain;
                    v.im *= next.clip_gain;
                }
            }
        }
        let samples = match &mut next.banks {
            Banks::B32 { synthesis, .. } => render_subbands(
                &mut next.qmf,
                &mut reader,
                &scaled,
                residuals,
                slots,
                synthesis,
                &next.order,
                &next.gains,
            )?,
            Banks::B64 { synthesis, .. } => render_subbands(
                &mut next.qmf,
                &mut reader,
                &scaled,
                residuals,
                slots,
                synthesis,
                &next.order,
                &next.gains,
            )?,
            Banks::B128 { synthesis, .. } => render_subbands(
                &mut next.qmf,
                &mut reader,
                &scaled,
                residuals,
                slots,
                synthesis,
                &next.order,
                &next.gains,
            )?,
        };
        let frame = Frame {
            sample_rate: next.config.sample_rate,
            channels: next.order.len(),
            samples,
        };
        *self = next;
        *bits = reader;
        Ok(frame)
    }
}

fn render<const B: usize>(
    qmf: &mut aac_ld_mps_qmf::Decoder,
    bits: &mut BitReader<'_>,
    pcm: &[Vec<f64>],
    residuals: &[Option<Vec<Vec<Complex>>>],
    slots: usize,
    analysis: &mut [Analysis<B>],
    synthesis: &mut [Synthesis<B>],
    order: &[usize],
    gains: &[f64],
    clip: f64,
) -> Result<Vec<f64>> {
    if pcm.len() != analysis.len()
        || pcm
            .iter()
            .any(|c| c.len() != slots * B || c.iter().any(|v| !v.is_finite()))
    {
        return Err(invalid("invalid LD-MPS downmix PCM frame"));
    }
    let mut downmix = vec![vec![vec![Complex::default(); B]; analysis.len()]; slots];
    for (c, bank) in analysis.iter_mut().enumerate() {
        let scaled = pcm[c].iter().map(|v| v * clip).collect::<Vec<_>>();
        let values = bank.process(&scaled)?;
        for (t, value) in values.into_iter().enumerate() {
            downmix[t][c] = value.to_vec();
        }
    }
    render_subbands(
        qmf, bits, &downmix, residuals, slots, synthesis, order, gains,
    )
}
fn render_subbands<const B: usize>(
    qmf: &mut aac_ld_mps_qmf::Decoder,
    bits: &mut BitReader<'_>,
    downmix: &[Vec<Vec<Complex>>],
    residuals: &[Option<Vec<Vec<Complex>>>],
    slots: usize,
    synthesis: &mut [Synthesis<B>],
    order: &[usize],
    gains: &[f64],
) -> Result<Vec<f64>> {
    let output = qmf.decode(bits, downmix, residuals)?;
    let mut planar = Vec::with_capacity(synthesis.len());
    for (c, bank) in synthesis.iter_mut().enumerate() {
        let combined = (0..slots)
            .map(|t| {
                std::array::from_fn(|q| Complex {
                    re: output.dry[t][c][q].re + output.wet[t][c][q].re,
                    im: output.dry[t][c][q].im + output.wet[t][c][q].im,
                })
            })
            .collect::<Vec<[Complex; B]>>();
        planar.push(bank.process(&combined)?);
    }
    let mut interleaved = Vec::with_capacity(slots * B * order.len());
    for t in 0..slots * B {
        for (c, &source) in order.iter().enumerate() {
            let value = planar[source][t] * gains[c];
            if !value.is_finite() {
                return Err(invalid("overflowing LD-MPS PCM fixed gain"));
            }
            interleaved.push(value);
        }
    }
    Ok(interleaved)
}

impl Decoder {
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        self.qmf.visit_retained(f)?;
        self.config.visit_retained(f)?;
        f.vector(&self.order)?;
        f.vector(&self.gains)?;
        match &self.banks {
            Banks::B32 {
                analysis,
                synthesis,
            } => {
                f.vector(analysis)?;
                f.vector(synthesis)?;
                for a in analysis {
                    a.visit_retained(f)?;
                }
                for s in synthesis {
                    s.visit_retained(f)?;
                }
            }
            Banks::B64 {
                analysis,
                synthesis,
            } => {
                f.vector(analysis)?;
                f.vector(synthesis)?;
                for a in analysis {
                    a.visit_retained(f)?;
                }
                for s in synthesis {
                    s.visit_retained(f)?;
                }
            }
            Banks::B128 {
                analysis,
                synthesis,
            } => {
                f.vector(analysis)?;
                f.vector(synthesis)?;
                for a in analysis {
                    a.visit_retained(f)?;
                }
                for s in synthesis {
                    s.visit_retained(f)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod inband_state_tests {
    use super::*;
    #[test]
    fn gain_change_preserves_analysis_clears_synthesis_and_failed_reconfigure_is_atomic() {
        let config = Config::parse(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-eld-mps-core/tree7.ssc"
        ))
        .unwrap();
        let packet = include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-eld-mps-core/tree7-0.spatial"
        );
        let pcm = vec![(0..512).map(|i| (i as f64 * 0.13).sin() * 0.01).collect()];
        let mut decoder = Decoder::new(&config, Mode::Complex).unwrap();
        decoder
            .decode(&mut BitReader::new(packet), &pcm, &[None])
            .unwrap();
        let prior = decoder.clone();
        decoder.reconfigure(&config).unwrap();
        assert_eq!(decoder, prior);
        let mut gain = config.clone();
        gain.downmix_gain = (gain.downmix_gain + 1) % 8;
        let fresh = Decoder::new(&gain, Mode::Complex).unwrap();
        decoder.reconfigure(&gain).unwrap();
        let (
            Banks::B64 {
                analysis,
                synthesis,
            },
            Banks::B64 {
                analysis: old_a,
                synthesis: old_s,
            },
            Banks::B64 {
                synthesis: fresh_s, ..
            },
        ) = (&decoder.banks, &prior.banks, &fresh.banks)
        else {
            unreachable!()
        };
        assert_eq!(analysis, old_a);
        assert_eq!(synthesis, fresh_s);
        assert_ne!(synthesis, old_s);
        let saved = decoder.clone();
        let mut changed = gain.clone();
        changed.sample_rate += 1;
        assert!(decoder.reconfigure(&changed).is_err());
        assert_eq!(decoder, saved);
        let mut changed = gain;
        changed.surround_gain = None; // stereo has no surround field: unchanged SSC.
        decoder.reconfigure(&changed).unwrap();
        assert_eq!(decoder, saved);
        let mut longer = decoder.config.clone();
        longer.frame_slots *= 2;
        decoder.reconfigure(&longer).unwrap();
        assert_eq!(decoder.banks, saved.banks);
        assert_eq!(decoder.config.frame_slots, longer.frame_slots);
    }
}
