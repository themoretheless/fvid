//! Complete PS QMF-domain hybrid analysis/synthesis, 71/91 routed bands.
//! Protocol topology: 3GPP SP-040428 figures 8.3/8.5; GOST 53556.8 tables 36–38.
//! Physical QMF history survives configuration changes; phase/decorrelator
//! resets are separate owners. No PCM/QMF startup trimming is done here.
use super::{
    Result,
    aac_ps_hybrid_filter::{Analysis, DELAY, Prototype},
    aac_ps_mapping::Bands,
    aac_sbr_qmf::Complex,
    invalid,
};

pub const QMF_CHANNELS: usize = 64;
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    bands: Bands,
    history: [[Complex; QMF_CHANNELS]; 13],
    filters: [Analysis; 5],
}
impl Default for State {
    fn default() -> Self {
        Self::new(Bands::Twenty)
    }
}
impl State {
    pub fn new(bands: Bands) -> Self {
        Self {
            bands,
            history: [[Complex::default(); QMF_CHANNELS]; 13],
            filters: [
                Prototype::TwentyEight,
                Prototype::TwentyTwo,
                Prototype::ThirtyFourTwelve,
                Prototype::ThirtyFourEight,
                Prototype::ThirtyFourFour,
            ]
            .map(Analysis::new),
        }
    }
    pub fn bands(&self) -> Bands {
        self.bands
    }
    pub fn reset(&mut self, bands: Bands) {
        self.history.fill([Complex::default(); QMF_CHANNELS]);
        self.bands = bands;
    }
    fn split(&self, filter: usize, qmf: usize) -> Result<Vec<Complex>> {
        self.filters[filter].filter_history(&std::array::from_fn(|tap| self.history[tap][qmf]))
    }
    /// Advance chronological 64-channel QMF slots. Empty calls have no state
    /// effect, including requested grid changes. A successful nonempty call
    /// uses its requested configuration on the shared physical input history.
    pub fn process(&mut self, bands: Bands, input: &[[Complex; QMF_CHANNELS]]) -> Result<Frame> {
        if input.is_empty() {
            return Ok(Frame {
                bands: self.bands,
                bands_changed: false,
                slots: Vec::new(),
            });
        }
        if input.iter().flatten().any(|&c| !finite(c)) {
            return Err(invalid("PS hybrid QMF input must be finite"));
        }
        let mut trial = self.clone();
        let changed = trial.bands != bands;
        let mut output = Vec::with_capacity(input.len());
        for slot in input {
            trial.history.copy_within(0..12, 1);
            trial.history[0] = *slot;
            let mut hybrid = Vec::with_capacity(bands.bindings().len());
            let first_unsplit = match bands {
                Bands::Twenty => {
                    let q0 = trial.split(0, 0)?;
                    // Figure 8.3: negative low bins first; opposite outer bins
                    // fold into q2/q3. QMF 1's real two-way split is reversed.
                    hybrid.extend([
                        q0[6],
                        q0[7],
                        q0[0],
                        q0[1],
                        add(q0[2], q0[5]),
                        add(q0[3], q0[4]),
                    ]);
                    let q1 = trial.split(1, 1)?;
                    hybrid.extend([q1[1], q1[0]]);
                    hybrid.extend(trial.split(1, 2)?);
                    3
                }
                Bands::ThirtyFour => {
                    hybrid.extend(trial.split(2, 0)?);
                    hybrid.extend(trial.split(3, 1)?);
                    for qmf in 2..5 {
                        hybrid.extend(trial.split(4, qmf)?);
                    }
                    5
                }
            };
            hybrid.extend_from_slice(&trial.history[DELAY][first_unsplit..]);
            if hybrid.iter().any(|&c| !finite(c)) {
                return Err(invalid("PS hybrid folded output is not representable"));
            }
            output.push(hybrid);
        }
        trial.bands = bands;
        *self = trial;
        Ok(Frame {
            bands,
            bands_changed: changed,
            slots: output,
        })
    }
}
fn finite(c: Complex) -> bool {
    c.re.is_finite() && c.im.is_finite()
}
fn add(a: Complex, b: Complex) -> Complex {
    Complex {
        re: a.re + b.re,
        im: a.im + b.im,
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub bands: Bands,
    pub bands_changed: bool,
    /// Chronological slots in the same routed order as Bands::bindings().
    pub slots: Vec<Vec<Complex>>,
}
impl Frame {
    pub fn synthesize(&self) -> Result<Vec<[Complex; QMF_CHANNELS]>> {
        synthesize(self.bands, &self.slots)
    }
}
/// Undo the split by summing each QMF channel's routed subbands (6.4.7).
/// Conjugation belongs to the mixing coefficients, never analysis/synthesis.
pub fn synthesize(bands: Bands, input: &[Vec<Complex>]) -> Result<Vec<[Complex; QMF_CHANNELS]>> {
    let bindings = bands.bindings();
    input
        .iter()
        .map(|slot| {
            if slot.len() != bindings.len() {
                return Err(invalid(
                    "PS hybrid synthesis width differs from configuration",
                ));
            }
            let mut qmf = [Complex::default(); QMF_CHANNELS];
            for (&sample, binding) in slot.iter().zip(bindings) {
                if !finite(sample) {
                    return Err(invalid("PS hybrid synthesis input must be finite"));
                }
                let channel = usize::from(binding.qmf);
                qmf[channel] = add(qmf[channel], sample);
            }
            if qmf.iter().any(|&c| !finite(c)) {
                return Err(invalid("PS hybrid synthesis output is not representable"));
            }
            Ok(qmf)
        })
        .collect()
}
