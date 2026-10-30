//! PCM/spatial-frame association in the caller's timestamp clock.
//! The AAC owner supplies canonical decoded downmix and completed ancillary
//! messages. This does not enable native admission or decode residual AAC.
use super::{
    Result,
    aac_ld_mps_config::{Config, ExtensionValue},
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
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub pcm: aac_ld_mps_pcm::Frame,
    pub stamp: Stamp,
}
#[derive(Clone, Debug, PartialEq)]
struct Chunk {
    planar: Vec<Vec<f64>>,
    offset: usize,
    stamp: Stamp,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pcm: aac_ld_mps_pcm::Decoder,
    inputs: usize,
    frame_samples: usize,
    residual_boxes: usize,
    chunks: VecDeque<Chunk>,
    pending: VecDeque<Vec<u8>>,
    queued_samples: usize,
}
impl Session {
    pub fn new(config: &Config, mode: Mode) -> Result<Self> {
        if config.extensions.iter().any(|e| {
            matches!(&e.value,
            ExtensionValue::Residual {bands, ..} if bands.iter().any(|b| b.unwrap_or(0)!=0))
        }) {
            return Err(unsupported(
                "LD-MPS session residual AAC reconstruction is not yet implemented",
            ));
        }
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
        Ok(Self {
            pcm: aac_ld_mps_pcm::Decoder::new(config, mode)?,
            inputs: usize::from(config.tree.input_channels),
            frame_samples: usize::from(config.frame_slots) * bins,
            residual_boxes: config.tree.ott_lfe.len() + usize::from(config.tree.ttt_boxes),
            chunks: VecDeque::new(),
            pending: VecDeque::new(),
            queued_samples: 0,
        })
    }
    pub fn reset(&mut self) {
        self.pcm.reset();
        self.chunks.clear();
        self.pending.clear();
        self.queued_samples = 0;
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
        let mut next = self.clone();
        if let Some((planar, stamp)) = core {
            let count = planar.first().map_or(0, Vec::len);
            if planar.len() != next.inputs
                || count == 0
                || planar
                    .iter()
                    .any(|c| c.len() != count || c.iter().any(|v| !v.is_finite()))
            {
                return Err(invalid("invalid LD-MPS session downmix geometry or sample"));
            }
            if stamp.timescale == 0 {
                return Err(invalid("zero LD-MPS source timescale"));
            }
            endpoint(stamp, count, count)?;
            next.queued_samples = next
                .queued_samples
                .checked_add(count)
                .ok_or_else(|| invalid("LD-MPS queued sample count overflow"))?;
            next.chunks
                .try_reserve(1)
                .map_err(|_| invalid("LD-MPS PCM queue allocation failed"))?;
            next.chunks.push_back(Chunk {
                planar: planar.to_vec(),
                offset: 0,
                stamp,
            });
        }
        next.pending
            .try_reserve(messages.len())
            .map_err(|_| invalid("LD-MPS frame queue allocation failed"))?;
        for message in messages {
            if message.kind != Kind::Frame {
                return Err(unsupported(
                    "LD-MPS in-band configuration is not yet implemented",
                ));
            }
            next.pending.push_back(message.payload.clone());
        }
        let mut output = Vec::new();
        while next.queued_samples >= next.frame_samples && !next.pending.is_empty() {
            let (planar, stamp) = next.front_pcm()?;
            let payload = next.pending.front().unwrap();
            let mut bits = BitReader::new(payload);
            let residuals = vec![None; next.residual_boxes];
            let pcm = next.pcm.decode(&mut bits, &planar, &residuals)?;
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
            output.push(Frame { pcm, stamp });
            next.pending.pop_front();
            let mut remaining = next.frame_samples;
            while remaining != 0 {
                let chunk = next.chunks.front_mut().unwrap();
                let available = chunk.planar[0].len() - chunk.offset;
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
    fn front_pcm(&self) -> Result<(Vec<Vec<f64>>, Stamp)> {
        let mut planar: Vec<Vec<f64>> = (0..self.inputs)
            .map(|_| Vec::with_capacity(self.frame_samples))
            .collect();
        let mut remaining = self.frame_samples;
        let mut stamp: Option<Stamp> = None;
        let mut prior_end = 0;
        for chunk in &self.chunks {
            let total = chunk.planar[0].len();
            let take = (total - chunk.offset).min(remaining);
            let start = endpoint(chunk.stamp, chunk.offset, total)?;
            let end = endpoint(chunk.stamp, chunk.offset + take, total)?;
            let timed = chunk.stamp.duration != 0;
            if let Some(first) = &stamp {
                if first.timescale != chunk.stamp.timescale {
                    return Err(invalid("LD-MPS frame spans different source clocks"));
                }
                // A positive source duration may round to zero for a short
                // prefix; retain source timing mode rather than testing the
                // accumulated output duration to distinguish untimed input.
                let first_timed = self.chunks.front().unwrap().stamp.duration != 0;
                if first_timed != timed {
                    return Err(invalid("LD-MPS frame mixes timed and untimed PCM"));
                }
                if timed && start != prior_end {
                    return Err(invalid("LD-MPS frame spans discontinuous timestamps"));
                }
            } else {
                stamp = Some(Stamp {
                    pts: start,
                    duration: 0,
                    timescale: chunk.stamp.timescale,
                });
            }
            prior_end = end;
            let out = stamp.as_mut().unwrap();
            if timed {
                out.duration = u64::try_from(i128::from(end) - i128::from(out.pts))
                    .map_err(|_| invalid("LD-MPS output duration overflow"))?;
            }
            for (dst, src) in planar.iter_mut().zip(&chunk.planar) {
                dst.extend_from_slice(&src[chunk.offset..chunk.offset + take]);
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
        if self.chunks.front().unwrap().stamp.duration == 0 {
            stamp.duration = 0;
        }
        Ok((planar, stamp))
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
        f.deque(&self.chunks)?;
        for chunk in &self.chunks {
            f.vector(&chunk.planar)?;
            for row in &chunk.planar {
                f.vector(row)?;
            }
        }
        f.deque(&self.pending)?;
        for payload in &self.pending {
            f.vector(payload)?;
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
        extra += (session.pending.capacity() - prior) * std::mem::size_of::<Vec<u8>>();
        let planar = &mut session.chunks.front_mut().unwrap().planar;
        let prior = planar.capacity();
        planar.reserve(11);
        extra += (planar.capacity() - prior) * std::mem::size_of::<Vec<f64>>();
        let row = &mut planar[0];
        let prior = row.capacity();
        row.reserve(157);
        extra += (row.capacity() - prior) * std::mem::size_of::<f64>();
        let payload = session.pending.front_mut().unwrap();
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
