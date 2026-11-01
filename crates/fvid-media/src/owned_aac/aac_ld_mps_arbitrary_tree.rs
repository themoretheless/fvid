//! Post-shaping arbitrary-tree gain routing. Native PCM currently qualifies
//! full-band splits and dense output permutations; sparse layouts remain explicit refusals.
use super::{aac_ld_mps_matrix_apply::Samples, aac_sbr_qmf::Complex, invalid, unsupported, Result};
#[derive(Clone, Debug, PartialEq)]
struct Leaf {
    source: usize,
    output: usize,
    path: Vec<(usize, bool)>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Decoder {
    bands: usize,
    inputs: usize,
    boxes: usize,
    leaves: Vec<Leaf>,
    previous: Vec<Vec<f64>>,
}
fn dense_positions(positions: &[u8]) -> Result<()> {
    let mut seen = vec![false; positions.len()];
    for &p in positions {
        let Some(v) = seen.get_mut(usize::from(p)) else {
            return Err(unsupported(
                "sparse arbitrary-tree output positions require qualification",
            ));
        };
        if *v {
            return Err(invalid("duplicate arbitrary-tree output position"));
        }
        *v = true;
    }
    Ok(())
}
impl Decoder {
    /// Preorder split/leaf flags per base-tree output. Positions must be a
    /// permutation of dense output indices. Every split spans the full grid.
    pub fn new(nodes: &[Vec<bool>], cutoffs: &[u8], positions: &[u8], bands: u8) -> Result<Self> {
        if !matches!(bands, 4 | 5 | 7 | 9 | 12 | 15 | 23)
            || nodes.is_empty()
            || nodes.len() > 8
            || positions.is_empty()
            || positions.len() > 27
        {
            return Err(invalid("invalid LD-MPS arbitrary-tree geometry"));
        }
        if cutoffs.iter().any(|&b| b != bands) {
            return Err(unsupported(
                "partial-band arbitrary-tree splits require qualification",
            ));
        }
        dense_positions(positions)?;
        let mut leaves = Vec::new();
        let mut boxes = 0;
        for (source, row) in nodes.iter().enumerate() {
            let mut pending = vec![Vec::new()];
            for &split in row {
                let Some(path) = pending.pop() else {
                    return Err(invalid("excess arbitrary-tree preorder node"));
                };
                if split {
                    if boxes >= cutoffs.len() || path.len() >= 26 {
                        return Err(invalid("invalid arbitrary-tree split count/depth"));
                    }
                    let mut right = path.clone();
                    right.push((boxes, false));
                    let mut left = path;
                    left.push((boxes, true));
                    boxes += 1;
                    pending.push(right);
                    pending.push(left);
                } else {
                    let output = *positions.get(leaves.len()).ok_or_else(|| {
                        invalid("arbitrary-tree leaf count exceeds output positions")
                    })?;
                    leaves.push(Leaf {
                        source,
                        output: usize::from(output),
                        path,
                    });
                }
            }
            if !pending.is_empty() {
                return Err(invalid("unfinished arbitrary-tree preorder"));
            }
        }
        if boxes != cutoffs.len() || leaves.len() != positions.len() {
            return Err(invalid("arbitrary-tree box/leaf count mismatch"));
        }
        let previous = leaves
            .iter()
            .map(|_| vec![1.0; usize::from(bands)])
            .collect();
        Ok(Self {
            bands: usize::from(bands),
            inputs: nodes.len(),
            boxes,
            leaves,
            previous,
        })
    }
    /// Change only the output permutation. Histories belong to preorder leaves,
    /// not output indices; validation finishes before any route is changed.
    pub fn set_output_positions(&mut self, positions: &[u8]) -> Result<()> {
        if positions.len() != self.leaves.len() {
            return Err(invalid("arbitrary-tree output permutation count mismatch"));
        }
        dense_positions(positions)?;
        for (leaf, &position) in self.leaves.iter_mut().zip(positions) {
            leaf.output = usize::from(position);
        }
        Ok(())
    }
    pub fn reset(&mut self) {
        for p in &mut self.previous {
            p.fill(1.0);
        }
    }
    /// Input [slot][base output][QMF bin], dequantized CLD [set][box][band].
    /// Endpoint interpolation is independent of spatial matrix smoothing.
    /// Errors retain every gain history; callers may clone full checkpoints.
    pub fn process(
        &mut self,
        input: &Samples,
        endpoints: &[u8],
        cld: &[Vec<Vec<f64>>],
        map: &[u8],
        fixed: &[f64],
    ) -> Result<Samples> {
        let slots = input.len();
        if slots == 0
            || slots > 32
            || map.is_empty()
            || map.len() > 128
            || map.iter().any(|&b| usize::from(b) >= self.bands)
            || endpoints.is_empty()
            || endpoints.len() != cld.len()
            || endpoints.last().copied() != Some((slots - 1) as u8)
            || endpoints.windows(2).any(|w| w[0] >= w[1])
            || endpoints.iter().any(|&p| usize::from(p) >= slots)
            || fixed.len() != self.inputs
            || fixed.iter().any(|v| !v.is_finite())
            || input.iter().any(|s| {
                s.len() != self.inputs
                    || s.iter().any(|c| {
                        c.len() != map.len()
                            || c.iter().any(|z| !z.re.is_finite() || !z.im.is_finite())
                    })
            })
            || cld.iter().any(|s| {
                s.len() != self.boxes
                    || s.iter().any(|b| {
                        b.len() != self.bands || b.iter().any(|v| !v.is_finite() || v.abs() > 150.0)
                    })
            })
        {
            return Err(invalid("invalid arbitrary-tree frame geometry/values"));
        }
        let mut previous = self.previous.clone();
        let mut out = vec![vec![vec![Complex::default(); map.len()]; self.leaves.len()]; slots];
        let mut begin = 0;
        for (&end, parameters) in endpoints.iter().zip(cld) {
            let stop = usize::from(end) + 1;
            for (i, leaf) in self.leaves.iter().enumerate() {
                let mut target = vec![1.0; self.bands];
                for &(node, left) in &leaf.path {
                    for (band, gain) in target.iter_mut().enumerate() {
                        let ratio = 10.0f64.powf(parameters[node][band] / 10.0);
                        *gain *= if left {
                            (ratio / (1.0 + ratio)).sqrt()
                        } else {
                            (1.0 / (1.0 + ratio)).sqrt()
                        };
                    }
                }
                for t in begin..stop {
                    let alpha = (t + 1 - begin) as f64 / (stop - begin) as f64;
                    for (q, &band) in map.iter().enumerate() {
                        let band = usize::from(band);
                        let gain = fixed[leaf.source]
                            * ((1.0 - alpha) * previous[i][band] + alpha * target[band]);
                        let z = input[t][leaf.source][q];
                        let result = Complex {
                            re: z.re * gain,
                            im: z.im * gain,
                        };
                        if !result.re.is_finite() || !result.im.is_finite() {
                            return Err(invalid("nonfinite arbitrary-tree output"));
                        }
                        out[t][leaf.output][q] = result;
                    }
                }
                previous[i] = target;
            }
            begin = stop;
        }
        self.previous = previous;
        Ok(out)
    }
}

/// Native post-shaping owner; the public component above also supports saved
/// independent reference probes without parsing or synthesizing AAC.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stage {
    extension: usize,
    decoder: Decoder,
    fixed: Vec<f64>,
}
impl Stage {
    pub(crate) fn new(config: &super::aac_ld_mps_config::Config) -> Result<Option<Self>> {
        use super::aac_ld_mps_config::ExtensionValue;
        let mut result = None;
        for (extension, e) in config.extensions.iter().enumerate() {
            if let ExtensionValue::ArbitraryTree {
                nodes,
                ott,
                positions,
            } = &e.value
            {
                if result.is_some() {
                    return Err(invalid("multiple arbitrary-tree extensions"));
                }
                if nodes.len() != usize::from(config.tree.output_channels) {
                    return Err(invalid("arbitrary-tree base output count mismatch"));
                }
                let decoder = Decoder::new(
                    nodes,
                    &ott.iter().map(|o| o.bands).collect::<Vec<_>>(),
                    positions,
                    config.parameter_bands,
                )?;
                let mut fixed = vec![1.0; nodes.len()];
                let surround = *super::aac_ld_mps_prepare_facts::SURROUND
                    .get(usize::from(config.surround_gain.unwrap_or(0)))
                    .ok_or_else(|| invalid("invalid arbitrary-tree surround gain"))?;
                let lfe = *super::aac_ld_mps_prepare_facts::LFE
                    .get(usize::from(config.lfe_gain.unwrap_or(0)))
                    .ok_or_else(|| invalid("invalid arbitrary-tree LFE gain"))?;
                match config.tree.code {
                    0 => {
                        fixed[3] = lfe;
                        fixed[4] = surround;
                        fixed[5] = surround;
                    }
                    1 | 2 => {
                        fixed[5] = lfe;
                        fixed[1] = surround;
                        fixed[3] = surround;
                    }
                    3..=6 => {
                        fixed[7] = lfe;
                        for i in [1, 2, 4, 5] {
                            fixed[i] = surround;
                        }
                    }
                    7 => {}
                    _ => return Err(invalid("invalid arbitrary-tree base tree")),
                }
                result = Some(Self {
                    extension,
                    decoder,
                    fixed,
                });
            }
        }
        Ok(result)
    }
    pub(crate) fn channels(&self) -> usize {
        self.decoder.leaves.len()
    }
    pub(crate) fn inherit(&mut self, old: &Self) -> Result<()> {
        if self.extension != old.extension
            || self.decoder.bands != old.decoder.bands
            || self.decoder.leaves.len() != old.decoder.leaves.len()
            || self
                .decoder
                .leaves
                .iter()
                .zip(&old.decoder.leaves)
                .any(|(a, b)| a.source != b.source || a.path != b.path)
        {
            return Err(unsupported(
                "LD-MPS arbitrary-tree layout/grid change requires stream reconfiguration",
            ));
        }
        self.decoder.previous = old.decoder.previous.clone();
        Ok(())
    }
    pub(crate) fn reset(&mut self) {
        self.decoder.reset();
    }
    pub(crate) fn process(
        &mut self,
        input: &Samples,
        p: &super::aac_ld_mps_prepare::Prepared,
    ) -> Result<Samples> {
        use super::aac_ld_mps_frame::ParameterId;
        let mut cld = vec![vec![vec![0.0; self.decoder.bands]; self.decoder.boxes]; p.slots.len()];
        for b in 0..self.decoder.boxes {
            let values = &p
                .parameters
                .iter()
                .find(|v| {
                    v.id == ParameterId::ArbitraryCld {
                        extension: self.extension,
                        box_index: b,
                    }
                })
                .ok_or_else(|| invalid("missing arbitrary-tree CLD parameters"))?
                .values;
            if values.len() != p.slots.len() {
                return Err(invalid("arbitrary-tree CLD endpoint count mismatch"));
            }
            for (set, row) in values.iter().enumerate() {
                cld[set][b] = row.clone();
            }
        }
        self.decoder
            .process(input, &p.slots, &cld, &p.qmf_parameter_bands, &self.fixed)
    }
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        f.vector(&self.fixed)?;
        f.vector(&self.decoder.leaves)?;
        for leaf in &self.decoder.leaves {
            f.vector(&leaf.path)?;
        }
        f.vector(&self.decoder.previous)?;
        for row in &self.decoder.previous {
            f.vector(row)?;
        }
        Ok(())
    }
}
