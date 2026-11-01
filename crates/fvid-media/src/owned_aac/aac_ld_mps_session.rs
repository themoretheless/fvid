//! PCM/spatial-frame association in the caller's timestamp clock.
//! The AAC owner supplies canonical decoded downmix and completed ancillary
//! messages. Residual AAC is reconstructed in the qualified complex64-bin domain.
use super::{
    Result,
    aac_ld_mps_config::Config,
    aac_ld_mps_decor::Mode,
    aac_ld_mps_pcm,
    aac_ld_mps_transport::{Kind, Message},
    bits::BitReader,
    invalid, unsupported,
};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub pts: i64,
    pub duration: u64,
    pub timescale: u32,
}
/// Exact source-chunk slice represented by part of one rendered spatial frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceSpan {
    /// Offset and full size in the submitted source chunk, in sample frames.
    pub offset: usize,
    pub source_samples: usize,
    /// Original unsliced stamp. `stamp` below describes the represented slice
    /// in a media clock, or retains the opaque ID in packet-association mode.
    pub original_stamp: Stamp,
    pub samples: usize,
    pub stamp: Stamp,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub pcm: aac_ld_mps_pcm::Frame,
    pub stamp: Stamp,
    pub sources: Vec<SourceSpan>,
}
#[derive(Clone, Debug, PartialEq)]
enum Signal {
    Pcm(Vec<Vec<f64>>),
    Qmf(Vec<Vec<Vec<super::aac_sbr_qmf::Complex>>>),
}
enum SignalView<'a> {
    Pcm(&'a [Vec<f64>]),
    Qmf(&'a [Vec<Vec<super::aac_sbr_qmf::Complex>>]),
}
#[derive(Clone, Debug, PartialEq)]
struct Chunk {
    signal: Signal,
    samples: usize,
    offset: usize,
    stamp: Stamp,
}
#[derive(Clone, Debug, PartialEq)]
struct Pending {
    replacement: Option<Config>,
    payload: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    initial_config: Config,
    mode: Mode,
    qmf_input: bool,
    ld_sbr_qmf: bool,
    bins: usize,
    pcm: aac_ld_mps_pcm::Decoder,
    inputs: usize,
    frame_samples: usize,
    residual_boxes: usize,
    chunks: VecDeque<Chunk>,
    pending: VecDeque<Pending>,
    queued_samples: usize,
    packet_ids: bool,
    packet_slices: bool,
}
impl Session {
    pub fn new(config: &Config, mode: Mode) -> Result<Self> {
        Self::new_domain(config, mode, false, false)
    }
    pub fn new_qmf(config: &Config, mode: Mode) -> Result<Self> {
        Self::new_domain(config, mode, true, false)
    }
    /// Shared AAC LD-SBR QMF retains its output-rate synthesis convention.
    pub fn new_ld_sbr_qmf(config: &Config, mode: Mode) -> Result<Self> {
        Self::new_domain(config, mode, true, true)
    }
    fn new_domain(config: &Config, mode: Mode, qmf_input: bool, ld_sbr_qmf: bool) -> Result<Self> {
        if config.frame_slots == 0 || config.tree.input_channels == 0 {
            return Err(invalid("zero LD-MPS session geometry"));
        }
        let bins = if config.sample_rate < 27713 {
            32
        } else if config.sample_rate >= 55426 {
            128
        } else {
            64
        };
        let mut pcm = if ld_sbr_qmf {
            aac_ld_mps_pcm::Decoder::new_ld_sbr_qmf(config, mode)?
        } else if qmf_input {
            aac_ld_mps_pcm::Decoder::new_qmf(config, mode)?
        } else {
            aac_ld_mps_pcm::Decoder::new(config, mode)?
        };
        pcm.enable_native_residuals()?;
        Ok(Self {
            initial_config: config.clone(),
            mode,
            qmf_input,
            ld_sbr_qmf,
            bins,
            pcm,
            inputs: usize::from(config.tree.input_channels),
            frame_samples: usize::from(config.frame_slots) * bins,
            residual_boxes: config.tree.ott_lfe.len() + usize::from(config.tree.ttt_boxes),
            chunks: VecDeque::new(),
            pending: VecDeque::new(),
            queued_samples: 0,
            packet_ids: false,
            packet_slices: false,
        })
    }
    pub fn reset(&mut self) {
        self.pcm = Self::new_domain(
            &self.initial_config,
            self.mode,
            self.qmf_input,
            self.ld_sbr_qmf,
        )
        .expect("previously validated LD-MPS reset configuration")
        .pcm;
        self.frame_samples = usize::from(self.initial_config.frame_slots) * self.bins;
        self.chunks.clear();
        self.pending.clear();
        self.queued_samples = 0;
    }
    /// Reader-owned association IDs are opaque, not timestamps. This mode
    /// requires complete source chunks so each output keeps its original ID.
    pub fn set_source_packet_ids(&mut self, enabled: bool) -> Result<()> {
        if self.packet_ids != enabled && (self.queued_samples != 0 || !self.pending.is_empty()) {
            return Err(invalid("LD-MPS source stamp mode change requires reset"));
        }
        self.packet_ids = enabled;
        Ok(())
    }
    /// Native AAC reassembles these spans before exposing a source window.
    /// Ordinary opaque-ID clients retain the strict whole-chunk contract.
    pub(crate) fn enable_packet_slices(&mut self) {
        self.packet_slices = true;
    }
    pub fn queued_samples(&self) -> usize {
        self.queued_samples
    }
    pub fn queued_frames(&self) -> usize {
        self.pending.len()
    }
    /// EOF does not silently discard unmatched PCM or spatial parameters.
    /// The transport assembler's independent finish check is also required.
    pub fn finish(&self) -> Result<()> {
        if self.queued_samples != 0 || !self.pending.is_empty() {
            Err(invalid("unfinished LD-MPS PCM/spatial association"))
        } else {
            Ok(())
        }
    }
    /// Submit one decoded core packet (if any) and completed spatial messages.
    /// Output may be delayed, split a packet, or span several packets. Timestamp
    /// fractions use exact integer endpoints so splitting conserves duration.
    /// Cross-clock, timed/untimed, and discontinuous spans are refused when they
    /// would be combined into one output frame. All errors retain every queue
    /// and DSP history, including PCM submitted by this call.
    pub fn submit(
        &mut self,
        core: Option<(&[Vec<f64>], Stamp)>,
        messages: &[Message],
    ) -> Result<Vec<Frame>> {
        if self.qmf_input {
            return Err(invalid("QMF LD-MPS session requires subband input"));
        }
        self.submit_signal(core.map(|(v, s)| (SignalView::Pcm(v), s)), messages)
    }
    /// Shared normalized QMF is [slot][canonical channel][band]. Source timing
    /// still uses physical output samples; QMF chunks split only on slot edges.
    pub fn submit_qmf(
        &mut self,
        core: Option<(&[Vec<Vec<super::aac_sbr_qmf::Complex>>], Stamp)>,
        messages: &[Message],
    ) -> Result<Vec<Frame>> {
        if !self.qmf_input {
            return Err(invalid("PCM LD-MPS session requires PCM input"));
        }
        self.submit_signal(core.map(|(v, s)| (SignalView::Qmf(v), s)), messages)
    }
    fn submit_signal(
        &mut self,
        core: Option<(SignalView<'_>, Stamp)>,
        messages: &[Message],
    ) -> Result<Vec<Frame>> {
        let mut next = self.clone();
        if let Some((signal, stamp)) = core {
            let count = match &signal {
                SignalView::Pcm(planar) => {
                    let count = planar.first().map_or(0, Vec::len);
                    if planar.len() != next.inputs
                        || count == 0
                        || planar
                            .iter()
                            .any(|c| c.len() != count || c.iter().any(|v| !v.is_finite()))
                    {
                        return Err(invalid("invalid LD-MPS session downmix geometry or sample"));
                    }
                    count
                }
                SignalView::Qmf(rows) => {
                    if rows.is_empty()
                        || rows.iter().any(|s| {
                            s.len() != next.inputs
                                || s.iter().any(|c| {
                                    c.len() != next.bins
                                        || c.iter().any(|v| !v.re.is_finite() || !v.im.is_finite())
                                })
                        })
                    {
                        return Err(invalid("invalid LD-MPS session QMF geometry or sample"));
                    }
                    rows.len()
                        .checked_mul(next.bins)
                        .ok_or_else(|| invalid("LD-MPS QMF sample count overflow"))?
                }
            };
            if stamp.timescale == 0 {
                return Err(invalid("zero LD-MPS source timescale"));
            }
            if !next.packet_ids {
                endpoint(stamp, count, count)?;
            }
            next.queued_samples = next
                .queued_samples
                .checked_add(count)
                .ok_or_else(|| invalid("LD-MPS queued sample count overflow"))?;
            let signal = match signal {
                SignalView::Pcm(v) => Signal::Pcm(v.to_vec()),
                SignalView::Qmf(v) => Signal::Qmf(v.to_vec()),
            };
            next.chunks
                .try_reserve(1)
                .map_err(|_| invalid("LD-MPS PCM queue allocation failed"))?;
            next.chunks.push_back(Chunk {
                signal,
                samples: count,
                offset: 0,
                stamp,
            });
        }
        next.pending
            .try_reserve(messages.len())
            .map_err(|_| invalid("LD-MPS frame queue allocation failed"))?;
        for message in messages {
            let (replacement, payload) = if message.kind == Kind::HeaderAndFrame {
                let mut bits = BitReader::new(&message.payload);
                let (config, delay) = Config::read_in_band(&mut bits)?;
                if delay.is_some_and(|delay| delay != 0) {
                    // ISO/IEC 14496-26:2010/Amd.3:2014,7.26.1.2.1.9:
                    // ldsacTimeAlign shall be zero in the LD-MPS profile.
                    return Err(invalid(
                        "nonzero LD-MPS time alignment is not permitted by the low-delay profile",
                    ));
                }
                if config.sample_rate != next.initial_config.sample_rate
                    || config.tree.code != next.initial_config.tree.code
                {
                    return Err(unsupported(
                        "LD-MPS in-band output format change requires stream reconfiguration",
                    ));
                }
                let _validated =
                    Self::new_domain(&config, next.mode, next.qmf_input, next.ld_sbr_qmf)?;
                let offset = bits.position() / 8;
                if offset == message.payload.len() {
                    return Err(invalid(
                        "missing spatial frame after in-band LD-MPS configuration",
                    ));
                }
                (Some(config), message.payload[offset..].to_vec())
            } else {
                (None, message.payload.clone())
            };
            next.pending.push_back(Pending {
                replacement,
                payload,
            });
        }
        let mut output = Vec::new();
        while let Some(pending) = next.pending.front() {
            // HeaderAndFrame owns the length of this frame, including when
            // its header arrived before enough core packets were available.
            // Do not use the preceding frame's length to test readiness or
            // slice the queued source spans.
            let samples = pending
                .replacement
                .as_ref()
                .map_or(next.frame_samples, |c| {
                    usize::from(c.frame_slots) * next.bins
                });
            if next.queued_samples < samples {
                break;
            }
            next.frame_samples = samples;
            let (signal, stamp, sources) = next.front_signal()?;
            let pending = next.pending.front().unwrap();
            if let Some(candidate) = &pending.replacement {
                next.pcm.reconfigure(candidate)?;
            }
            let mut bits = BitReader::new(&pending.payload);
            let residuals = vec![None; next.residual_boxes];
            let pcm = match signal {
                Signal::Pcm(planar) => next.pcm.decode(&mut bits, &planar, &residuals)?,
                Signal::Qmf(rows) => next.pcm.decode_qmf(&mut bits, &rows, &residuals)?,
            };
            if bits.remaining() > 7 {
                return Err(invalid("trailing bytes after LD-MPS spatial frame"));
            }
            while bits.remaining() > 0 {
                if bits.bit()? {
                    return Err(invalid("nonzero LD-MPS frame alignment bits"));
                }
            }
            output
                .try_reserve(1)
                .map_err(|_| invalid("LD-MPS output queue allocation failed"))?;
            output.push(Frame {
                pcm,
                stamp,
                sources,
            });
            next.pending.pop_front();
            let mut remaining = next.frame_samples;
            while remaining != 0 {
                let chunk = next.chunks.front_mut().unwrap();
                let available = chunk.samples - chunk.offset;
                let take = available.min(remaining);
                chunk.offset += take;
                remaining -= take;
                if take == available {
                    next.chunks.pop_front();
                }
            }
            next.queued_samples -= next.frame_samples;
        }
        *self = next;
        Ok(output)
    }
    fn front_signal(&self) -> Result<(Signal, Stamp, Vec<SourceSpan>)> {
        let mut signal = if self.qmf_input {
            Signal::Qmf(Vec::with_capacity(self.frame_samples / self.bins))
        } else {
            Signal::Pcm(
                (0..self.inputs)
                    .map(|_| Vec::with_capacity(self.frame_samples))
                    .collect(),
            )
        };
        let mut remaining = self.frame_samples;
        let mut stamp: Option<Stamp> = None;
        let mut prior_end = 0;
        let mut sources = Vec::new();
        for chunk in &self.chunks {
            let total = chunk.samples;
            let take = (total - chunk.offset).min(remaining);
            let (start, end) = if self.packet_ids {
                // Association IDs never participate in timestamp arithmetic.
                (chunk.stamp.pts, chunk.stamp.pts)
            } else {
                (
                    endpoint(chunk.stamp, chunk.offset, total)?,
                    endpoint(chunk.stamp, chunk.offset + take, total)?,
                )
            };
            if self.packet_ids && !self.packet_slices && (chunk.offset != 0 || take != total) {
                return Err(invalid("LD-MPS packet association splits a source window"));
            }
            let timed = self.packet_ids || chunk.stamp.duration != 0;
            if let Some(first) = &stamp {
                if first.timescale != chunk.stamp.timescale {
                    return Err(invalid("LD-MPS frame spans different source clocks"));
                }
                // A positive source duration may round to zero for a short
                // prefix; retain source timing mode rather than testing the
                // accumulated output duration to distinguish untimed input.
                let first_timed =
                    self.packet_ids || self.chunks.front().unwrap().stamp.duration != 0;
                if first_timed != timed {
                    return Err(invalid("LD-MPS frame mixes timed and untimed PCM"));
                }
                if timed && !self.packet_ids && start != prior_end {
                    return Err(invalid("LD-MPS frame spans discontinuous timestamps"));
                }
            } else {
                stamp = Some(Stamp {
                    pts: start,
                    duration: 0,
                    timescale: chunk.stamp.timescale,
                });
            }
            sources
                .try_reserve(1)
                .map_err(|_| invalid("LD-MPS source span allocation failed"))?;
            sources.push(SourceSpan {
                offset: chunk.offset,
                source_samples: total,
                original_stamp: chunk.stamp,
                samples: take,
                stamp: Stamp {
                    pts: if self.packet_ids {
                        chunk.stamp.pts
                    } else {
                        start
                    },
                    duration: if self.packet_ids {
                        chunk.stamp.duration
                    } else if timed {
                        u64::try_from(i128::from(end) - i128::from(start))
                            .map_err(|_| invalid("LD-MPS source duration overflow"))?
                    } else {
                        0
                    },
                    timescale: chunk.stamp.timescale,
                },
            });
            prior_end = end;
            let out = stamp.as_mut().unwrap();
            if timed {
                out.duration = if self.packet_ids {
                    let begin =
                        u128::from(chunk.stamp.duration) * chunk.offset as u128 / total as u128;
                    let end = u128::from(chunk.stamp.duration) * (chunk.offset + take) as u128
                        / total as u128;
                    out.duration
                        .checked_add((end - begin) as u64)
                        .ok_or_else(|| invalid("LD-MPS output duration overflow"))?
                } else {
                    u64::try_from(i128::from(end) - i128::from(out.pts))
                        .map_err(|_| invalid("LD-MPS output duration overflow"))?
                };
            }
            match (&mut signal, &chunk.signal) {
                (Signal::Pcm(dst), Signal::Pcm(src)) => {
                    for (dst, src) in dst.iter_mut().zip(src) {
                        dst.extend_from_slice(&src[chunk.offset..chunk.offset + take]);
                    }
                }
                (Signal::Qmf(dst), Signal::Qmf(src)) => {
                    if !chunk.offset.is_multiple_of(self.bins) || !take.is_multiple_of(self.bins) {
                        return Err(invalid("LD-MPS QMF queue split between subband slots"));
                    }
                    dst.extend_from_slice(
                        &src[chunk.offset / self.bins..(chunk.offset + take) / self.bins],
                    );
                }
                _ => return Err(invalid("LD-MPS input domain changed within a queued frame")),
            }
            remaining -= take;
            if remaining == 0 {
                break;
            }
        }
        if remaining != 0 {
            return Err(invalid("LD-MPS PCM queue count mismatch"));
        }
        let mut stamp = stamp.ok_or_else(|| invalid("empty LD-MPS PCM queue"))?;
        if !self.packet_ids && self.chunks.front().unwrap().stamp.duration == 0 {
            stamp.duration = 0;
        }
        Ok((signal, stamp, sources))
    }
}
fn endpoint(stamp: Stamp, offset: usize, samples: usize) -> Result<i64> {
    let delta = u128::from(stamp.duration) * (offset as u128) / (samples as u128);
    i64::try_from(
        i128::from(stamp.pts)
            + i128::try_from(delta).map_err(|_| invalid("LD-MPS timestamp offset overflow"))?,
    )
    .map_err(|_| invalid("LD-MPS timestamp endpoint overflow"))
}

impl Session {
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        self.pcm.visit_retained(f)?;
        self.initial_config.visit_retained(f)?;
        f.deque(&self.chunks)?;
        for chunk in &self.chunks {
            match &chunk.signal {
                Signal::Pcm(planar) => {
                    f.vector(planar)?;
                    for row in planar {
                        f.vector(row)?;
                    }
                }
                Signal::Qmf(slots) => {
                    f.vector(slots)?;
                    for slot in slots {
                        f.vector(slot)?;
                        for row in slot {
                            f.vector(row)?;
                        }
                    }
                }
            }
        }
        f.deque(&self.pending)?;
        for pending in &self.pending {
            f.vector(&pending.payload)?;
            if let Some(candidate) = &pending.replacement {
                candidate.visit_retained(f)?;
            }
        }
        Ok(())
    }
}

impl Session {
    /// Owned retained heap payload including spare capacity, excluding static
    /// QMF modulation caches, stack/allocator headers and caller-owned PCM.
    /// This is not peak working memory. Inspection allocates no heap memory.
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        self.retained_payload_bytes_with_checkpoint(None)
    }
    pub fn retained_payload_bytes_with_checkpoint(&self, saved: Option<&Self>) -> Result<usize> {
        let mut footprint = super::memory::Footprint::new();
        self.visit_retained(&mut footprint)
            .map_err(|e| invalid(&e))?;
        if let Some(saved) = saved.filter(|saved| !std::ptr::eq(*saved, self)) {
            saved
                .visit_retained(&mut footprint)
                .map_err(|e| invalid(&e))?;
        }
        Ok(footprint.total())
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    #[test]
    fn queued_spare_capacity_and_independent_snapshots_are_included() {
        let config = Config::parse(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-eld-mps-core/tree7.ssc"
        ))
        .unwrap();
        let mut session = Session::new(&config, Mode::Complex).unwrap();
        session
            .submit(
                Some((
                    &[vec![0.; 123]],
                    Stamp {
                        pts: 0,
                        duration: 123,
                        timescale: 48000,
                    },
                )),
                &[],
            )
            .unwrap();
        // Keep parameters pending: 123 input samples cannot render a 512-sample frame.
        let message = Message {
            extension: super::super::aac_ld_mps_transport::Extension::LowDelay,
            kind: Kind::Frame,
            payload: vec![1, 2, 3],
        };
        session.submit(None, &[message]).unwrap();
        let before = session.retained_payload_bytes().unwrap();
        // At least the owned 64-bin analysis/synthesis histories and minimum
        // allpass storage must be present, in addition to queues and matrices.
        assert!(before >= (10 * 64 + 2 * 20 * 64) * 8 + 64 * (2 * 4 * 16 + 16 + 2 * 3 * 16));
        let mut extra = 0;
        let prior = session.chunks.capacity();
        session.chunks.reserve(19);
        extra += (session.chunks.capacity() - prior) * std::mem::size_of::<Chunk>();
        let prior = session.pending.capacity();
        session.pending.reserve(17);
        extra += (session.pending.capacity() - prior) * std::mem::size_of::<Pending>();
        let Signal::Pcm(planar) = &mut session.chunks.front_mut().unwrap().signal else {
            unreachable!()
        };
        let prior = planar.capacity();
        planar.reserve(11);
        extra += (planar.capacity() - prior) * std::mem::size_of::<Vec<f64>>();
        let row = &mut planar[0];
        let prior = row.capacity();
        row.reserve(157);
        extra += (row.capacity() - prior) * std::mem::size_of::<f64>();
        let payload = &mut session.pending.front_mut().unwrap().payload;
        let prior = payload.capacity();
        payload.reserve(293);
        extra += payload.capacity() - prior;
        assert_eq!(session.retained_payload_bytes().unwrap(), before + extra);
        assert_eq!(
            session
                .retained_payload_bytes_with_checkpoint(Some(&session))
                .unwrap(),
            before + extra
        );
        let saved = session.clone();
        assert_eq!(
            session
                .retained_payload_bytes_with_checkpoint(Some(&saved))
                .unwrap(),
            session.retained_payload_bytes().unwrap() + saved.retained_payload_bytes().unwrap()
        );
        let saved_size = saved.retained_payload_bytes().unwrap();
        session.reset();
        assert!(session.retained_payload_bytes().unwrap() < before + extra);
        assert_eq!(saved.retained_payload_bytes().unwrap(), saved_size);
    }
}

#[cfg(test)]
mod inband_memory_tests {
    use super::*;
    #[test]
    fn queued_configuration_nested_spare_capacity_and_checkpoints_are_counted() {
        let raw =
            include_bytes!("../../../../tests/fixtures/playback-errors/aac-eld-mps-core/tree7.ssc");
        let config = Config::parse(raw).unwrap();
        let fresh = Session::new(&config, Mode::Complex).unwrap();
        let mut session = fresh.clone();
        // A bounded unknown extension keeps a separately owned payload in SSC.
        let mut body = raw.to_vec();
        body.extend([0xf1, 0x5a]);
        let mut payload = vec![body.len() as u8];
        payload.extend(body);
        payload.extend(include_bytes!(
            "../../../../tests/fixtures/playback-errors/aac-eld-mps-core/tree7-0.spatial"
        ));
        let message = Message {
            extension: super::super::aac_ld_mps_transport::Extension::LowDelay,
            kind: Kind::HeaderAndFrame,
            payload,
        };
        assert!(session.submit(None, &[message]).unwrap().is_empty());
        let before = session.retained_payload_bytes().unwrap();
        let mut extra = 0;
        let pending = session.pending.front_mut().unwrap();
        let prior = pending.payload.capacity();
        pending.payload.reserve(93);
        extra += pending.payload.capacity() - prior;
        let cfg = pending.replacement.as_mut().unwrap();
        let prior = cfg.tree.ott_lfe.capacity();
        cfg.tree.ott_lfe.reserve(29);
        extra += cfg.tree.ott_lfe.capacity() - prior;
        let prior = cfg.ttt.capacity();
        cfg.ttt.reserve(11);
        extra += (cfg.ttt.capacity() - prior)
            * std::mem::size_of::<super::super::aac_ld_mps_config::Ttt>();
        let prior = cfg.extensions.capacity();
        cfg.extensions.reserve(17);
        extra += (cfg.extensions.capacity() - prior)
            * std::mem::size_of::<super::super::aac_ld_mps_config::Extension>();
        let bytes = &mut cfg.extensions.last_mut().unwrap().payload;
        let prior = bytes.capacity();
        bytes.reserve(37);
        extra += bytes.capacity() - prior;
        assert_eq!(session.retained_payload_bytes().unwrap(), before + extra);
        assert_eq!(
            session
                .retained_payload_bytes_with_checkpoint(Some(&session))
                .unwrap(),
            before + extra
        );
        let saved = session.clone();
        assert_eq!(
            session
                .retained_payload_bytes_with_checkpoint(Some(&saved))
                .unwrap(),
            before + extra + saved.retained_payload_bytes().unwrap()
        );
        session.reset();
        assert_eq!(session, fresh);
        assert!(!saved.pending.is_empty());
    }
}
