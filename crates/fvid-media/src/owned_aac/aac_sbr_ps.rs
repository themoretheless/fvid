//! Owned mono SBR payload/core PCM to delayed stereo PCM.
//! One frame is queued to obtain real PS hybrid lookahead from the next frame.
//! The caller maps `frame_index` back to packet time; full native AAC dispatch
//! and analysis/synthesis startup trimming remain the enclosing decoder's job.
use super::{
    Result, aac_ps_decorrelation::FrameControls, aac_ps_dsp, aac_ps_history,
    aac_sbr_dsp::OutputRate, aac_sbr_history, aac_sbr_qmf::Complex, aac_sbr_qmf_dsp,
    bits::BitReader, invalid,
};
use std::collections::VecDeque;
#[derive(Clone, Debug, PartialEq)]
struct Pending {
    parameters: Option<aac_ps_history::Parameters>,
    rows: Vec<[Complex; 64]>,
    controls: FrameControls,
    frame_index: u64,
}
/// Validated syntax retained while the owning core aligns its PCM. Fields are
/// opaque so callers cannot replace frame identity or header-derived controls.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedFrame {
    frame: Option<aac_sbr_history::Frame>,
    parameters: Option<aac_ps_history::Parameters>,
    controls: FrameControls,
    format: (u32, u8, OutputRate),
    frame_index: u64,
}
impl PreparedFrame {
    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }
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
    next_render_index: u64,
    prepared: VecDeque<PreparedFrame>,
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
    fn check_prepare(&self, rate: u32, slots: u8, output_rate: OutputRate) -> Result<()> {
        if self.finished {
            return Err(invalid("SBR PS input after EOF requires reset"));
        }
        if self.prepared.len() >= 2 {
            return Err(invalid("SBR PS prepared frame lookahead bound exceeded"));
        }
        if self
            .format
            .is_some_and(|old| old != (rate, slots, output_rate))
        {
            return Err(invalid("SBR PS format changed without reset"));
        }
        Ok(())
    }
    /// Parse/validate syntax immediately, without advancing QMF or PS synthesis.
    /// Two unrendered frames are allowed for a core with one packet of alignment.
    /// Reader and all syntax histories commit together only after successful parse.
    pub fn prepare(
        &mut self,
        bits: &mut BitReader<'_>,
        end: usize,
        crc: bool,
        rate: u32,
        slots: u8,
        output_rate: OutputRate,
    ) -> Result<PreparedFrame> {
        self.check_prepare(rate, slots, output_rate)?;
        if !matches!(slots, 15 | 16) {
            return Err(invalid("SBR PS slot count must be 15 or 16"));
        }
        let mut trial = self.clone();
        let mut reader = bits.clone();
        let frame = trial.sbr.read(&mut reader, end, crc, rate, slots, 1)?;
        let data = frame.syntax.data.extended_data.as_deref().unwrap_or(&[]);
        let parsed = trial.parameters.read_sbr_extensions(data, slots * 2)?;
        let prepared = PreparedFrame {
            controls: FrameControls {
                previous_ps_present: trial.previous_ps_present,
                qmf_limit: rows_limit(&frame),
            },
            frame: Some(frame),
            parameters: parsed
                .last()
                .filter(|p| p.parameters.initialized)
                .map(|p| p.parameters.clone()),
            format: (rate, slots, output_rate),
            frame_index: trial.next_index,
        };
        trial.previous_ps_present = !parsed.is_empty();
        trial.ps_seen |= !parsed.is_empty();
        trial.next_index = trial
            .next_index
            .checked_add(1)
            .ok_or_else(|| invalid("SBR PS frame index overflow"))?;
        trial.format = Some(prepared.format);
        trial.prepared.push_back(prepared.clone());
        *self = trial;
        *bits = reader;
        Ok(prepared)
    }
    /// An explicitly absent FIL has no syntax to retain and detects no PS.
    pub fn prepare_upsampling(
        &mut self,
        rate: u32,
        slots: u8,
        output_rate: OutputRate,
    ) -> Result<PreparedFrame> {
        self.check_prepare(rate, slots, output_rate)?;
        if !matches!(slots, 15 | 16) {
            return Err(invalid("invalid SBR pure upsampling inputs"));
        }
        let next = self
            .next_index
            .checked_add(1)
            .ok_or_else(|| invalid("SBR PS frame index overflow"))?;
        let prepared = PreparedFrame {
            frame: None,
            parameters: None,
            controls: FrameControls {
                previous_ps_present: self.previous_ps_present,
                qmf_limit: 32,
            },
            format: (rate, slots, output_rate),
            frame_index: self.next_index,
        };
        self.previous_ps_present = false;
        self.next_index = next;
        self.format = Some(prepared.format);
        self.prepared.push_back(prepared.clone());
        Ok(prepared)
    }
    /// Feed an already validated frame after its core PCM is aligned. Retry,
    /// replay and failed PCM preserve DSP state; duplicate/out-of-order frames
    /// cannot advance synthesis. Syntax may already be one frame ahead.
    pub fn process_prepared(
        &mut self,
        prepared: &PreparedFrame,
        pcm: &[f32],
    ) -> Result<Option<Frame>> {
        if self.finished {
            return Err(invalid("SBR PS input after EOF requires reset"));
        }
        if prepared.frame_index != self.next_render_index
            || prepared.frame_index >= self.next_index
            || self.prepared.front() != Some(prepared)
        {
            return Err(invalid("SBR PS prepared frame identity mismatch"));
        }
        if self.format != Some(prepared.format) {
            return Err(invalid("SBR PS prepared frame format mismatch"));
        }
        let (rate, slots, _) = prepared.format;
        let mut trial = self.clone();
        let rows = if let Some(frame) = &prepared.frame {
            let rows = trial.qmf.process(frame, &[pcm], rate, slots)?;
            if rows.format_reset && trial.pending.is_some() {
                return Err(invalid(
                    "SBR PS format reset requires draining and explicit reset",
                ));
            }
            rows.rows
        } else {
            trial.qmf.process_upsampling(&[pcm], rate, slots)?
        };
        let rows = rows
            .into_iter()
            .next()
            .ok_or_else(|| invalid("missing mono SBR QMF rows"))?;
        let output = trial.render(&rows[..aac_ps_dsp::LOOKAHEAD])?;
        trial.pending = Some(Pending {
            parameters: prepared.parameters.clone(),
            rows,
            controls: prepared.controls,
            frame_index: prepared.frame_index,
        });
        trial.prepared.pop_front();
        trial.next_render_index = trial
            .next_render_index
            .checked_add(1)
            .ok_or_else(|| invalid("SBR PS frame index overflow"))?;
        *self = trial;
        Ok(output)
    }
    /// Atomic syntax plus DSP for cores that already provide aligned PCM.
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
        let mut trial = self.clone();
        let mut reader = bits.clone();
        let prepared = trial.prepare(&mut reader, end, crc, rate, slots, output_rate)?;
        let output = trial.process_prepared(&prepared, pcm)?;
        *self = trial;
        *bits = reader;
        Ok(output)
    }
    /// Atomic absent-FIL processing, with the same core QMF/stereo clock.
    pub fn process_upsampling(
        &mut self,
        pcm: &[f32],
        rate: u32,
        slots: u8,
        output_rate: OutputRate,
    ) -> Result<Option<Frame>> {
        let mut trial = self.clone();
        let prepared = trial.prepare_upsampling(rate, slots, output_rate)?;
        let output = trial.process_prepared(&prepared, pcm)?;
        *self = trial;
        Ok(output)
    }
    /// Emit the last frame using zero EOF lookahead once. Repeated EOF returns
    /// None. A failed render leaves the pending frame and every history intact.
    pub fn finish(&mut self) -> Result<Option<Frame>> {
        if self.finished {
            return Ok(None);
        }
        if self.next_index != self.next_render_index {
            return Err(invalid(
                "SBR PS prepared frames require aligned PCM before EOF",
            ));
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
