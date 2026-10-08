//! Owned mono SBR payload/core PCM to delayed stereo PCM.
//! One frame is queued to obtain real PS hybrid lookahead from the next frame.
//! The caller maps `frame_index` back to packet time; full native AAC dispatch
//! and analysis/synthesis startup trimming remain the enclosing decoder's job.
use super::{
    Result, aac_ps_decorrelation::FrameControls, aac_ps_dsp, aac_ps_history,
    aac_sbr_dsp::OutputRate, aac_sbr_history, aac_sbr_qmf::Complex, aac_sbr_qmf_dsp,
    bits::BitReader, invalid, unsupported,
};
#[derive(Clone, Debug, PartialEq)]
struct Pending {
    parameters: Option<aac_ps_history::Parameters>,
    rows: Vec<[Complex; 64]>,
    controls: FrameControls,
    frame_index: u64,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decoder {
    sbr: aac_sbr_history::Stream,
    parameters: aac_ps_history::Stream,
    qmf: aac_sbr_qmf_dsp::Dsp,
    ps: aac_ps_dsp::Dsp,
    pending: Option<Pending>,
    format: Option<(u32, u8, OutputRate)>,
    next_index: u64,
    previous_ps_present: bool,
    ps_seen: bool,
    finished: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub frame_index: u64,
    pub slots: u8,
    pub output_rate: OutputRate,
    /// Normalized, unclipped, chronological left/right PCM.
    pub pcm: [Vec<f64>; 2],
}
impl Decoder {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn ps_seen(&self) -> bool {
        self.ps_seen
    }
    pub fn pending_frame_index(&self) -> Option<u64> {
        self.pending.as_ref().map(|p| p.frame_index)
    }
    fn render(&mut self, future: &[[Complex; 64]]) -> Result<Option<Frame>> {
        let Some(pending) = self.pending.take() else {
            return Ok(None);
        };
        if future.len() != aac_ps_dsp::LOOKAHEAD {
            return Err(invalid("SBR PS requires six future QMF slots"));
        }
        let (_, slots, output_rate) = self
            .format
            .ok_or_else(|| invalid("missing SBR PS format"))?;
        let mut input = pending.rows;
        input.extend_from_slice(future);
        let mut output = if let Some(parameters) = &pending.parameters {
            self.ps
                .process(parameters, slots * 2, &input, pending.controls, output_rate)?
        } else {
            self.ps.process_dual_mono(slots * 2, &input, output_rate)?
        };
        for value in output.pcm.iter_mut().flatten() {
            *value /= 32768.0;
        }
        Ok(Some(Frame {
            frame_index: pending.frame_index,
            slots: slots * 2,
            output_rate,
            pcm: output.pcm,
        }))
    }
    /// Read one mono SBR payload with at most one PS element. The first call
    /// queues output and returns None; later calls return the preceding frame.
    /// Reader, SBR, native parameters, QMF, PS and queue commit atomically.
    /// Absent or independently uninitialized PS maps normal SBR mono to stereo.
    pub fn read(
        &mut self,
        bits: &mut BitReader<'_>,
        end: usize,
        crc: bool,
        pcm: &[f32],
        rate: u32,
        slots: u8,
        output_rate: OutputRate,
    ) -> Result<Option<Frame>> {
        if self.finished {
            return Err(invalid("SBR PS input after EOF requires reset"));
        }
        if !matches!(slots, 15 | 16) {
            return Err(invalid("SBR PS slot count must be 15 or 16"));
        }
        let format = (rate, slots, output_rate);
        if self.format.is_some_and(|old| old != format) {
            return Err(invalid("SBR PS format changed without reset"));
        }
        let mut trial = self.clone();
        let mut reader = bits.clone();
        let frame = trial.sbr.read(&mut reader, end, crc, rate, slots, 1)?;
        let data = frame.syntax.data.extended_data.as_deref().unwrap_or(&[]);
        let parsed = trial.parameters.read_sbr_extensions(data, slots * 2)?;
        if parsed.len() > 1 {
            return Err(unsupported("SBR PS frame permits at most one PS element"));
        }
        let rows = trial.qmf.process(&frame, &[pcm], rate, slots)?;
        if rows.format_reset && trial.pending.is_some() {
            return Err(invalid(
                "SBR PS format reset requires draining and explicit reset",
            ));
        }
        trial.format = Some(format);
        let rows = rows
            .rows
            .into_iter()
            .next()
            .ok_or_else(|| invalid("missing mono SBR QMF rows"))?;
        let output = trial.render(&rows[..aac_ps_dsp::LOOKAHEAD])?;
        trial.pending = Some(Pending {
            parameters: parsed
                .first()
                .filter(|p| p.parameters.initialized)
                .map(|p| p.parameters.clone()),
            rows,
            controls: FrameControls {
                previous_ps_present: trial.previous_ps_present,
                qmf_limit: rows_limit(&frame),
            },
            frame_index: trial.next_index,
        });
        trial.previous_ps_present = !parsed.is_empty();
        trial.ps_seen |= !parsed.is_empty();
        trial.next_index = trial
            .next_index
            .checked_add(1)
            .ok_or_else(|| invalid("SBR PS frame index overflow"))?;
        *self = trial;
        *bits = reader;
        Ok(output)
    }
    /// Emit the last frame using zero EOF lookahead once. Repeated EOF returns
    /// None. A failed render leaves the pending frame and every history intact.
    pub fn finish(&mut self) -> Result<Option<Frame>> {
        if self.finished {
            return Ok(None);
        }
        let mut trial = self.clone();
        let output = trial.render(&[[Complex::default(); 64]; aac_ps_dsp::LOOKAHEAD])?;
        trial.finished = true;
        *self = trial;
        Ok(output)
    }
}
fn rows_limit(frame: &aac_sbr_history::Frame) -> usize {
    usize::from(*frame.syntax.data.frequency.high.last().unwrap())
}
