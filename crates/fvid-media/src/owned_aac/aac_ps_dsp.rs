//! Transactional owned PS QMF-to-stereo-PCM pipeline.
//! Inputs include six future QMF slots to compensate hybrid analysis delay.
//! The caller owns QMF analysis startup, packet buffering and container time.
//! PCM amplitude inherits QMF input units; native AAC/SBR scaling is external.
use super::{
    Result,
    aac_ps_decorrelation::{self as decor, FrameControls},
    aac_ps_history::Parameters,
    aac_ps_hybrid,
    aac_ps_mapping::Bands,
    aac_ps_matrix_controller as matrix, aac_sbr_downsampled_qmf,
    aac_sbr_dsp::OutputRate,
    aac_sbr_qmf::Complex,
    aac_sbr_synthesis_qmf, invalid,
};
pub const LOOKAHEAD: usize = 6;
#[derive(Clone, Debug, PartialEq)]
enum Synthesis {
    Double([aac_sbr_synthesis_qmf::Synthesis; 2]),
    Core([aac_sbr_downsampled_qmf::Synthesis; 2]),
}
impl Synthesis {
    fn new(rate: OutputRate) -> Self {
        match rate {
            OutputRate::Double => Self::Double(std::array::from_fn(|_| Default::default())),
            OutputRate::Core => Self::Core(std::array::from_fn(|_| Default::default())),
        }
    }
    fn rate(&self) -> OutputRate {
        match self {
            Self::Double(_) => OutputRate::Double,
            Self::Core(_) => OutputRate::Core,
        }
    }
    fn process(&mut self, qmf: &[Vec<[Complex; 64]>; 2]) -> Result<[Vec<f64>; 2]> {
        match self {
            Self::Double(banks) => Ok([banks[0].process(&qmf[0])?, banks[1].process(&qmf[1])?]),
            Self::Core(banks) => {
                let low: [Vec<[Complex; 32]>; 2] = std::array::from_fn(|c| {
                    qmf[c]
                        .iter()
                        .map(|r| std::array::from_fn(|k| r[k]))
                        .collect()
                });
                Ok([banks[0].process(&low[0])?, banks[1].process(&low[1])?])
            }
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dsp {
    matrix: matrix::State,
    hybrid: aac_ps_hybrid::State,
    decorrelation: decor::State,
    synthesis: Option<Synthesis>,
    lookahead: Option<[[Complex; 64]; LOOKAHEAD]>,
}
impl Dsp {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn matrix(&self) -> &matrix::State {
        &self.matrix
    }
    pub fn hybrid(&self) -> &aac_ps_hybrid::State {
        &self.hybrid
    }
    pub fn decorrelation(&self) -> &decor::State {
        &self.decorrelation
    }
    pub fn retained_lookahead(&self) -> Option<&[[Complex; 64]; LOOKAHEAD]> {
        self.lookahead.as_ref()
    }
    pub fn output_rate(&self) -> Option<OutputRate> {
        self.synthesis.as_ref().map(Synthesis::rate)
    }
    fn validate_input(&self, slots: u8, qmf: &[[Complex; 64]], rate: OutputRate) -> Result<()> {
        if !matches!(slots, 24 | 30 | 32) {
            return Err(invalid("PS DSP slot count must be 24, 30 or 32"));
        }
        if qmf.len() != usize::from(slots) + LOOKAHEAD {
            return Err(invalid(
                "PS DSP requires a frame plus six QMF lookahead slots",
            ));
        }
        if qmf
            .iter()
            .flatten()
            .any(|c| !c.re.is_finite() || !c.im.is_finite())
        {
            return Err(invalid("PS DSP QMF input must be finite"));
        }
        if self.output_rate().is_some_and(|old| old != rate) {
            return Err(invalid("PS DSP output rate changed without reset"));
        }
        if self
            .lookahead
            .as_ref()
            .is_some_and(|old| old.as_slice() != &qmf[..LOOKAHEAD])
        {
            return Err(invalid(
                "PS DSP QMF lookahead overlap does not match retained input",
            ));
        }
        Ok(())
    }
    /// Normal SBR mono mapped to both output channels while PS is absent or
    /// not independently initialized. Retain raw hybrid history/lookahead for
    /// reentry and BOTH existing synthesis histories for continuous PCM.
    pub fn process_dual_mono(
        &mut self,
        slots: u8,
        qmf: &[[Complex; 64]],
        rate: OutputRate,
    ) -> Result<Frame> {
        self.validate_input(slots, qmf, rate)?;
        let mut trial = self.clone();
        let initial = trial.lookahead.is_none();
        let input = if initial { qmf } else { &qmf[LOOKAHEAD..] };
        let bands = trial.matrix.bands();
        trial.hybrid.process(bands, input)?;
        let rows = qmf[..usize::from(slots)].to_vec();
        let qmf = [rows.clone(), rows];
        let pcm = trial
            .synthesis
            .get_or_insert_with(|| Synthesis::new(rate))
            .process(&qmf)?;
        trial.lookahead = Some(std::array::from_fn(|n| input[input.len() - LOOKAHEAD + n]));
        let frame = Frame {
            bands,
            bands_changed: false,
            slots,
            output_rate: rate,
            qmf,
            pcm,
        };
        *self = trial;
        Ok(frame)
    }
    /// One PS frame of 24/30/32 aligned output slots. QMF input contains the
    /// frame's chronological slots followed by exactly six future slots.
    /// On subsequent calls its first six slots must equal the preceding
    /// lookahead. They are already in raw hybrid history and are not advanced
    /// twice. At EOF the caller may provide six zero future slots.
    ///
    /// All matrix, hybrid, decorrelation, overlap and BOTH synthesis histories
    /// commit together, only after PCM generation for both channels succeeds.
    pub fn process(
        &mut self,
        parameters: &Parameters,
        slots: u8,
        qmf: &[[Complex; 64]],
        controls: FrameControls,
        rate: OutputRate,
    ) -> Result<Frame> {
        self.validate_input(slots, qmf, rate)?;
        let mut trial = self.clone();
        let matrices = trial.matrix.process(parameters, slots)?;
        let bands = matrices.temporal.bands;
        let initial = trial.lookahead.is_none();
        let input = if initial { qmf } else { &qmf[LOOKAHEAD..] };
        let mut mono = trial.hybrid.process(bands, input)?.slots;
        if initial {
            mono.drain(..LOOKAHEAD);
        }
        let diffuse = trial.decorrelation.process_frame(bands, controls, &mono)?;
        let stereo = matrices.mix(&mono, &diffuse.output)?;
        let qmf = [
            aac_ps_hybrid::synthesize(bands, &stereo.channels[0])?,
            aac_ps_hybrid::synthesize(bands, &stereo.channels[1])?,
        ];
        let synthesis = trial.synthesis.get_or_insert_with(|| Synthesis::new(rate));
        let pcm = synthesis.process(&qmf)?;
        trial.lookahead = Some(std::array::from_fn(|n| input[input.len() - LOOKAHEAD + n]));
        let frame = Frame {
            bands,
            bands_changed: matrices.bands_changed,
            slots,
            output_rate: rate,
            qmf,
            pcm,
        };
        *self = trial;
        Ok(frame)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub bands: Bands,
    pub bands_changed: bool,
    pub slots: u8,
    pub output_rate: OutputRate,
    pub qmf: [Vec<[Complex; 64]>; 2],
    /// Separate chronological left/right PCM vectors, 64 or 32 samples/slot.
    pub pcm: [Vec<f64>; 2],
}
