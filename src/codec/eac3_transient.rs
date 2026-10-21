//! E.2.7 single-channel PCM correction. Decoder streaming integration is pending.
use crate::{Result, invalid};

/// Apply one event to a contiguous PCM/history window with an absolute sample
/// origin. The caller must retain all source and destination samples. This
/// operation performs no clipping, channel folding, or decoder delay handling.
/// Missing history/future PCM is an error and leaves the buffer unchanged.
pub fn apply_window(pcm: &mut [f32], buffer_start: i64, transient: i64, length: u8) -> Result<()> {
    apply_inner(pcm, buffer_start, transient, length, None)
}

fn correction_start(transient: i64, length: u8) -> Result<i64> {
    let pn = 256 + transient.rem_euclid(256);
    transient
        .checked_sub(pn + i64::from(length) + 256)
        .ok_or_else(|| invalid("E-AC-3 transient destination clock overflow"))
}

fn apply_inner(
    pcm: &mut [f32],
    buffer_start: i64,
    transient: i64,
    length: u8,
    cutoff: Option<i64>,
) -> Result<()> {
    const TC1: usize = 256;
    const TC2: usize = 128;
    // PN starts at the previous coding block, not the transient's own block.
    let pn = TC1 + transient.rem_euclid(TC1 as i64) as usize;
    let total = pn + usize::from(length) + TC1;
    let source_point = transient
        .checked_sub((2 * TC1 + 2 * pn) as i64)
        .ok_or_else(|| invalid("E-AC-3 transient source clock overflow"))?;
    let destination_point = correction_start(transient, length)?;
    let visible = if let Some(end) = cutoff {
        let end = end.min(transient);
        if end <= destination_point {
            return Ok(());
        }
        usize::try_from(end - destination_point)
            .map_err(|_| invalid("E-AC-3 transient visible span overflow"))?
    } else {
        total
    };
    let index = |point: i64| {
        let relative = point
            .checked_sub(buffer_start)
            .ok_or_else(|| invalid("E-AC-3 transient sample clock overflow"))?;
        usize::try_from(relative)
            .map_err(|_| invalid("E-AC-3 transient requires earlier PCM history"))
    };
    let source = index(source_point)?;
    let destination = index(destination_point)?;
    let count = if cutoff.is_some() {
        visible
    } else {
        2 * TC1 + pn
    };
    let source_end = source
        .checked_add(count)
        .filter(|&end| end <= pcm.len())
        .ok_or_else(|| invalid("E-AC-3 transient source PCM unavailable"))?;
    destination
        .checked_add(visible)
        .filter(|&end| end <= pcm.len())
        .ok_or_else(|| invalid("E-AC-3 transient destination PCM unavailable"))?;
    // PN <= 511, so the standard's synthesis window is at most 1023 samples.
    // Snapshot before writing: the source and corrected regions can overlap.
    let mut synthesis = [0.0f32; 1023];
    synthesis[..count].copy_from_slice(&pcm[source..source_end]);
    let fade = |sample: usize, count: usize| {
        (0.5 - 0.5 * (std::f64::consts::PI * sample as f64 / (count - 1) as f64).cos()) as f32
    };
    for sample in 0..visible {
        let at = destination + sample;
        pcm[at] = if sample < TC1 {
            let weight = fade(sample, TC1);
            pcm[at] * (1.0 - weight) + synthesis[sample] * weight
        } else if sample >= total - TC2 {
            let weight = fade(sample - (total - TC2), TC2);
            pcm[at] * weight + synthesis[sample] * (1.0 - weight)
        } else {
            synthesis[sample]
        };
    }
    Ok(())
}

/// Ready PCM in the decoder's raw sample clock, before priming and folding.
#[derive(Debug, PartialEq)]
pub struct PcmChunk {
    pub start: i64,
    pub samples: Vec<f32>,
}

/// Single-channel frame streaming. This retains zero prehistory for a fresh
/// decoder and withholds three blocks so a later frame can correct its tail.
/// A decoder adapter must align channels and preserve original source windows.
#[derive(Debug)]
pub struct Stream {
    pcm: Vec<f32>,
    buffer_start: i64,
    received: i64,
    emitted: i64,
    events: Vec<(i64, u8)>,
    finished: bool,
}
impl Default for Stream {
    fn default() -> Self {
        let mut pcm = Vec::with_capacity(1536 + 768 + 1536);
        pcm.resize(1536, 0.0);
        Self {
            pcm,
            buffer_start: -1536,
            received: 0,
            emitted: 0,
            events: Vec::new(),
            finished: false,
        }
    }
}
impl Stream {
    /// Append one coded frame (1/2/3/6 blocks). Event locations are relative to
    /// this frame, in samples, as retained by the E-AC-3 header parser.
    pub fn push_frame(
        &mut self,
        samples: &[f32],
        window: Option<super::TransientWindow>,
    ) -> Result<PcmChunk> {
        if self.finished {
            return Err(invalid("E-AC-3 transient stream is already finished"));
        }
        if ![256, 512, 768, 1536].contains(&samples.len()) {
            return Err(invalid(
                "E-AC-3 transient frame must contain 1/2/3/6 blocks",
            ));
        }
        let next = self
            .received
            .checked_add(samples.len() as i64)
            .ok_or_else(|| invalid("E-AC-3 transient stream clock overflow"))?;
        let event = if let Some(w) = window {
            if w.location > 4092 || !w.location.is_multiple_of(4) {
                return Err(invalid(
                    "E-AC-3 transient location exceeds coded resolution/range",
                ));
            }
            Some((
                self.received
                    .checked_add(i64::from(w.location))
                    .ok_or_else(|| invalid("E-AC-3 transient event clock overflow"))?,
                w.length,
            ))
        } else {
            None
        };
        if let Some(event) = event {
            self.events.push(event);
            self.events.sort_by_key(|event| event.0);
        }
        self.pcm.extend_from_slice(samples);
        self.received = next;
        while self
            .events
            .first()
            .is_some_and(|event| event.0 <= self.received)
        {
            let (at, length) = self.events[0];
            apply_window(&mut self.pcm, self.buffer_start, at, length)?;
            self.events.remove(0);
        }
        let mut ready = (self.received - 768).max(0);
        for &(at, length) in &self.events {
            ready = ready.min(correction_start(at, length)?.max(0));
        }
        // Channel folding operates on complete coding blocks.
        ready = ready / 256 * 256;
        self.emit(ready)
    }
    /// Finish at the actual decoded endpoint. A future event corrects only its
    /// available prefix; no silence or extra output duration is invented.
    pub fn finish(&mut self) -> Result<PcmChunk> {
        if self.finished {
            return Ok(PcmChunk {
                start: self.emitted,
                samples: Vec::new(),
            });
        }
        for &(at, length) in &self.events {
            apply_inner(
                &mut self.pcm,
                self.buffer_start,
                at,
                length,
                Some(self.received),
            )?;
        }
        self.events.clear();
        self.finished = true;
        self.emit(self.received)
    }
    fn emit(&mut self, end: i64) -> Result<PcmChunk> {
        if end < self.emitted || end > self.received {
            return Err(invalid("E-AC-3 transient would revise emitted PCM"));
        }
        let first = usize::try_from(self.emitted - self.buffer_start)
            .map_err(|_| invalid("invalid transient output origin"))?;
        let last = usize::try_from(end - self.buffer_start)
            .map_err(|_| invalid("invalid transient output endpoint"))?;
        let samples = self
            .pcm
            .get(first..last)
            .ok_or_else(|| invalid("transient output PCM unavailable"))?
            .to_vec();
        let start = self.emitted;
        self.emitted = end;
        let keep = (end - 1536).max(-1536);
        let drop = usize::try_from(keep - self.buffer_start)
            .map_err(|_| invalid("invalid transient history origin"))?;
        self.pcm.drain(..drop);
        self.buffer_start = keep;
        Ok(PcmChunk { start, samples })
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn retained_samples(&self) -> usize {
        self.pcm.len()
    }
}

/// A reconstructed coding block before correction and output mixing.
#[allow(dead_code)]
pub(crate) struct NativeBlock {
    pub pcm: [[f32; 256]; 7],
    pub windows: [Option<super::TransientWindow>; 5],
    pub mixing: super::super::ac3_decoder::Folding,
    pub gains: [f32; 7],
    pub clip: bool,
}
impl NativeBlock {
    pub(crate) fn silence(channels: usize) -> Self {
        Self {
            pcm: [[0.0; 256]; 7],
            windows: [None; 5],
            mixing: super::super::ac3_decoder::Folding::silence(channels),
            gains: [1.0; 7],
            clip: true,
        }
    }
}

/// Native-plane adapter: channel events may release different amounts of PCM.
/// Finalize only complete aligned blocks, using their original mixing state.
/// `T` carries the caller's source-window identity unchanged through the delay.
#[allow(dead_code)]
pub(crate) struct NativeBlocks<T> {
    streams: [Stream; 7],
    ready: [std::collections::VecDeque<f32>; 7],
    contexts: std::collections::VecDeque<(super::super::ac3_decoder::Folding, [f32; 7], bool, T)>,
}
impl<T> Default for NativeBlocks<T> {
    fn default() -> Self {
        Self {
            streams: std::array::from_fn(|_| Stream::default()),
            ready: std::array::from_fn(|_| Default::default()),
            contexts: Default::default(),
        }
    }
}
#[allow(dead_code)]
impl<T> NativeBlocks<T> {
    /// Supply zero PCM for inactive/coupling planes. Only the five native
    /// full-bandwidth channels can carry transient metadata; never the LFE.
    pub(crate) fn push(
        &mut self,
        pcm: &[[f32; 256]; 7],
        windows: [Option<super::TransientWindow>; 5],
        mixing: super::super::ac3_decoder::Folding,
        gains: [f32; 7],
        clip: bool,
        source: T,
    ) -> Result<Vec<(T, Vec<f32>)>> {
        // Reject malformed metadata before mutating any channel.
        if windows
            .iter()
            .flatten()
            .any(|w| w.location > 4092 || !w.location.is_multiple_of(4))
        {
            return Err(invalid(
                "E-AC-3 transient location exceeds coded resolution/range",
            ));
        }
        for plane in 0..7 {
            let chunk = self.streams[plane]
                .push_frame(&pcm[plane], windows.get(plane).copied().flatten())?;
            self.ready[plane].extend(chunk.samples);
        }
        self.contexts.push_back((mixing, gains, clip, source));
        Ok(self.collect())
    }
    pub(crate) fn finish(&mut self) -> Result<Vec<(T, Vec<f32>)>> {
        for plane in 0..7 {
            self.ready[plane].extend(self.streams[plane].finish()?.samples);
        }
        let output = self.collect();
        if !self.contexts.is_empty() || self.ready.iter().any(|p| !p.is_empty()) {
            return Err(invalid("E-AC-3 transient channel endpoint mismatch"));
        }
        Ok(output)
    }
    fn collect(&mut self) -> Vec<(T, Vec<f32>)> {
        let mut output = Vec::new();
        while !self.contexts.is_empty() && self.ready.iter().all(|p| p.len() >= 256) {
            let pcm = std::array::from_fn(|plane| {
                std::array::from_fn(|_| self.ready[plane].pop_front().unwrap())
            });
            let (mixing, gains, clip, source) = self.contexts.pop_front().unwrap();
            let mut samples = Vec::new();
            mixing.write_pcm(&pcm, &gains, &mut samples, clip);
            output.push((source, samples));
        }
        output
    }
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod native_block_tests {
    use super::*;
    use crate::codec::ac3_decoder::{Folding, Header};

    #[test]
    fn native_transient_blocks_preserve_channel_clock_gain_source_and_drain() {
        let header = Header::annex_e(0, 1024, 0, false, 0.5, 0.25);
        let mut adapter = NativeBlocks::default();
        for replay in 0..2 {
            let mut mono = [Stream::default(), Stream::default()];
            let mut expected = [Vec::new(), Vec::new()];
            let mut actual = Vec::new();
            for block in 0..16 {
                let pcm = std::array::from_fn(|p| {
                    std::array::from_fn(|n| ((block * 256 + n + p * 71) % 173) as f32 / 128.0 - 0.6)
                });
                let mut windows = [None; 5];
                if block == 0 {
                    windows[0] = Some(super::super::TransientWindow {
                        location: 2048,
                        length: 255,
                    });
                }
                if block == 2 {
                    windows[1] = Some(super::super::TransientWindow {
                        location: 0,
                        length: 128,
                    });
                }
                for p in 0..2 {
                    expected[p].extend(mono[p].push_frame(&pcm[p], windows[p]).unwrap().samples);
                }
                let gains = [0.25 + block as f32 * 0.125; 7];
                actual.extend(
                    adapter
                        .push(
                            &pcm,
                            windows,
                            Folding::coded(header),
                            gains,
                            true,
                            (replay, block),
                        )
                        .unwrap(),
                );
            }
            for p in 0..2 {
                expected[p].extend(mono[p].finish().unwrap().samples);
            }
            actual.extend(adapter.finish().unwrap());
            assert!(adapter.finish().unwrap().is_empty());
            assert_eq!(actual.len(), 16);
            for (block, (source, samples)) in actual.into_iter().enumerate() {
                assert_eq!(source, (replay, block));
                assert_eq!(samples.len(), 512);
                for n in 0..256 {
                    for p in 0..2 {
                        let wanted = (expected[p][block * 256 + n] * (0.25 + block as f32 * 0.125))
                            .clamp(-1.0, 1.0);
                        assert_eq!(
                            samples[n * 2 + p],
                            wanted,
                            "block {block}, sample {n}, plane {p}"
                        );
                    }
                }
            }
            adapter.reset();
        }
    }
}

/// Preserve access-unit timing while the native streams retain lookahead PCM.
#[derive(Default)]
pub(crate) struct NativePackets {
    native: NativeBlocks<u64>,
    serial: u64,
    pending: std::collections::VecDeque<PendingPacket>,
    finished: bool,
}
struct PendingPacket {
    serial: u64,
    pts: i64,
    duration: u64,
    remaining: usize,
    samples: Vec<f32>,
}
impl NativePackets {
    pub(crate) fn push(&mut self, blocks: Vec<NativeBlock>, pts: i64, duration: u64) -> Result<()> {
        if self.finished {
            return Err(invalid("E-AC-3 packet after EOF"));
        }
        if blocks.is_empty() {
            return Ok(());
        }
        self.serial = self
            .serial
            .checked_add(1)
            .ok_or_else(|| invalid("E-AC-3 packet counter overflow"))?;
        let serial = self.serial;
        self.pending.push_back(PendingPacket {
            serial,
            pts,
            duration,
            remaining: blocks.len(),
            samples: Vec::new(),
        });
        for b in blocks {
            let ready = self
                .native
                .push(&b.pcm, b.windows, b.mixing, b.gains, b.clip, serial)?;
            self.accept(ready)?;
        }
        Ok(())
    }
    fn accept(&mut self, ready: Vec<(u64, Vec<f32>)>) -> Result<()> {
        for (serial, samples) in ready {
            let packet = self
                .pending
                .iter_mut()
                .find(|p| p.serial == serial)
                .ok_or_else(|| invalid("E-AC-3 output lost its source packet"))?;
            packet.remaining = packet
                .remaining
                .checked_sub(1)
                .ok_or_else(|| invalid("E-AC-3 duplicate output block"))?;
            packet.samples.extend(samples);
        }
        Ok(())
    }
    pub(crate) fn finish(&mut self) -> Result<()> {
        if !self.finished {
            let ready = self.native.finish()?;
            self.accept(ready)?;
            self.finished = true;
            if self.pending.iter().any(|p| p.remaining != 0) {
                return Err(invalid("incomplete E-AC-3 packet at EOF"));
            }
        }
        Ok(())
    }
    pub(crate) fn pop(
        &mut self,
        rate: u32,
        channels: usize,
        lead: &mut usize,
    ) -> Option<crate::audio::DecodedAudio> {
        if self.pending.front().is_none_or(|p| p.remaining != 0) {
            return None;
        }
        let p = self.pending.pop_front().unwrap();
        let skip = (*lead).min(p.samples.len() / channels);
        *lead -= skip;
        let data = p.samples[skip * channels..]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        Some(crate::audio::DecodedAudio {
            packet: crate::audio::AudioPacket {
                data,
                pts: p.pts.max(0) as u64,
                timebase_num: 1,
                timebase_den: rate,
            },
            source_pts: p.pts,
            source_duration: p.duration,
        })
    }
}
