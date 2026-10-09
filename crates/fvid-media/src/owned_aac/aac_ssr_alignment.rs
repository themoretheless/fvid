//! Bounded packet-boundary PCM alignment for independently switched AAC SSR lanes.
//! Each lane is mono with per-chunk output gains. Frame stamps belong to the
//! caller's clock. Native SSR dispatch uses this queue when lane extents differ.
use super::{Result, invalid};
use std::collections::VecDeque;
const MAX_ROWS: usize = 1472;
const MAX_QUEUED: usize = 2 * MAX_ROWS;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputGain {
    pub channel: usize,
    pub gain: f32,
}
pub struct LaneInput<'a> {
    pub samples: &'a [f32],
    pub outputs: &'a [OutputGain],
}
#[derive(Clone)]
struct Chunk {
    samples: Vec<f32>,
    used: usize,
    outputs: Vec<OutputGain>,
}
#[derive(Clone, Default)]
struct Lane {
    chunks: VecDeque<Chunk>,
    rows: usize,
}
#[derive(Clone, Copy)]
struct Pending {
    stamp: u64,
    rows: usize,
}
#[derive(Debug, PartialEq)]
pub struct AlignedFrame {
    pub stamp: u64,
    pub samples: Vec<f32>,
}
/// One source interval before output gain/mixing. Chunk boundaries preserve
/// gain changes when an aligned packet crosses two synthesis submissions.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceChunk {
    pub samples: Vec<f32>,
    pub outputs: Vec<OutputGain>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct AlignedSources {
    pub stamp: u64,
    pub rows: usize,
    pub lanes: Vec<Vec<SourceChunk>>,
}
#[derive(Clone)]
pub struct SsrPcmAlignment {
    channels: usize,
    lanes: Vec<Lane>,
    pending: VecDeque<Pending>,
    finished: bool,
}
impl SsrPcmAlignment {
    pub fn new(channels: usize, lanes: usize) -> Result<Self> {
        if !(1..=255).contains(&channels) || !(channels..=channels + 16).contains(&lanes) {
            return Err(invalid("SSR PCM alignment invalid channel or lane count"));
        }
        Ok(Self {
            channels,
            lanes: vec![Lane::default(); lanes],
            pending: VecDeque::with_capacity(2),
            finished: false,
        })
    }
    /// Reorder existing source lanes and add sources at the current packet.
    /// New sources have no contribution in the already pending frame. Their
    /// silent prefix belongs only to that past interval, not their new samples.
    /// All old lanes must survive exactly once; retiring a source with retained
    /// synthesis/PCM history is a different operation, never an implicit drop.
    pub fn extend_lanes(&mut self, order: &[Option<usize>]) -> Result<()> {
        if self.finished {
            return Err(invalid("SSR PCM alignment requires reset after finish"));
        }
        if order.len() < self.lanes.len()
            || order.len() > self.channels + 16
            || order
                .iter()
                .take(self.channels)
                .enumerate()
                .any(|(i, index)| *index != Some(i))
        {
            return Err(invalid("SSR PCM alignment invalid lane extension"));
        }
        let mut visited = vec![false; self.lanes.len()];
        for index in order.iter().flatten() {
            let slot = visited
                .get_mut(*index)
                .ok_or_else(|| invalid("SSR PCM alignment invalid lane extension"))?;
            if *slot {
                return Err(invalid("SSR PCM alignment duplicate source lane"));
            }
            *slot = true;
        }
        if visited.iter().any(|v| !*v) {
            return Err(invalid("SSR PCM alignment cannot discard an existing lane"));
        }
        let prefix = self.pending.iter().try_fold(0usize, |rows, frame| {
            rows.checked_add(frame.rows)
                .ok_or_else(|| invalid("SSR PCM alignment prefix overflow"))
        })?;
        if prefix > MAX_QUEUED {
            return Err(invalid("SSR PCM alignment prefix exceeds lookahead bound"));
        }
        let lanes = order
            .iter()
            .map(|index| match index {
                Some(index) => self.lanes[*index].clone(),
                None => {
                    let mut lane = Lane::default();
                    if prefix > 0 {
                        lane.chunks.push_back(Chunk {
                            samples: vec![0.0; prefix],
                            used: 0,
                            outputs: Vec::new(),
                        });
                        lane.rows = prefix;
                    }
                    lane
                }
            })
            .collect();
        self.lanes = lanes;
        Ok(())
    }
    pub fn reset(&mut self) {
        for lane in &mut self.lanes {
            lane.chunks.clear();
            lane.rows = 0;
        }
        self.pending.clear();
        self.finished = false;
    }
    /// Retained payload, excluding allocator headers and caller buffers.
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        let mut bytes = self.lanes.capacity() * std::mem::size_of::<Lane>()
            + self.pending.capacity() * std::mem::size_of::<Pending>();
        for lane in &self.lanes {
            bytes = bytes
                .checked_add(lane.chunks.capacity() * std::mem::size_of::<Chunk>())
                .ok_or_else(|| invalid("SSR PCM alignment accounting overflow"))?;
            for chunk in &lane.chunks {
                bytes = bytes
                    .checked_add(
                        chunk.samples.capacity() * 4
                            + chunk.outputs.capacity() * std::mem::size_of::<OutputGain>(),
                    )
                    .ok_or_else(|| invalid("SSR PCM alignment accounting overflow"))?;
            }
        }
        Ok(bytes)
    }
    fn validate(&self, rows: usize, inputs: &[LaneInput<'_>], mixed: bool) -> Result<()> {
        if self.finished {
            return Err(invalid("SSR PCM alignment requires reset after finish"));
        }
        if !matches!(rows, 576 | 1024 | 1472) || inputs.len() != self.lanes.len() {
            return Err(invalid("SSR PCM alignment invalid frame or lane geometry"));
        }
        for (lane, input) in self.lanes.iter().zip(inputs) {
            if !matches!(input.samples.len(), 576 | 1024 | 1472)
                || input.outputs.len() > 512
                || input.samples.iter().any(|v| !v.is_finite())
                || input
                    .outputs
                    .iter()
                    .any(|o| o.channel >= self.channels || !o.gain.is_finite())
            {
                return Err(invalid("SSR PCM alignment invalid samples or output gains"));
            }
            if mixed
                && input.samples.iter().any(|sample| {
                    input
                        .outputs
                        .iter()
                        .any(|gain| !(*sample * gain.gain).is_finite())
                })
            {
                return Err(invalid(
                    "SSR PCM alignment scaled samples exceed finite f32",
                ));
            }
            if lane.rows + input.samples.len() > MAX_QUEUED {
                return Err(invalid(
                    "SSR PCM alignment retained rows exceed lookahead bound",
                ));
            }
        }
        Ok(())
    }
    // Peek at current lanes plus uncommitted input. Arithmetic errors commit no
    // queue mutation, including when only the last lane overflows the PCM sum.
    fn render(&self, frame: Pending, inputs: Option<&[LaneInput<'_>]>) -> Result<AlignedFrame> {
        let mut samples = vec![0.0; frame.rows * self.channels];
        for (index, lane) in self.lanes.iter().enumerate() {
            let incoming = inputs.map(|v| &v[index]);
            if lane.rows + incoming.map_or(0, |v| v.samples.len()) < frame.rows {
                return Err(invalid(
                    "SSR PCM alignment requires more than one packet of lookahead",
                ));
            }
            let mut written = 0;
            for chunk in &lane.chunks {
                let count = (chunk.samples.len() - chunk.used).min(frame.rows - written);
                Self::mix(
                    &chunk.samples[chunk.used..chunk.used + count],
                    &chunk.outputs,
                    &mut samples,
                    written,
                    self.channels,
                )?;
                written += count;
                if written == frame.rows {
                    break;
                }
            }
            if written < frame.rows {
                let input = incoming
                    .ok_or_else(|| invalid("SSR PCM alignment incomplete at stream end"))?;
                Self::mix(
                    &input.samples[..frame.rows - written],
                    input.outputs,
                    &mut samples,
                    written,
                    self.channels,
                )?;
            }
        }
        Ok(AlignedFrame {
            stamp: frame.stamp,
            samples,
        })
    }
    fn separate(&self, frame: Pending, inputs: Option<&[LaneInput<'_>]>) -> Result<AlignedSources> {
        let mut lanes = Vec::with_capacity(self.lanes.len());
        for (index, lane) in self.lanes.iter().enumerate() {
            let mut chunks = Vec::new();
            let mut remaining = frame.rows;
            for chunk in &lane.chunks {
                let count = (chunk.samples.len() - chunk.used).min(remaining);
                if count > 0 {
                    chunks.push(SourceChunk {
                        samples: chunk.samples[chunk.used..chunk.used + count].to_vec(),
                        outputs: chunk.outputs.clone(),
                    });
                    remaining -= count;
                }
                if remaining == 0 {
                    break;
                }
            }
            if remaining > 0 {
                let input = inputs
                    .and_then(|v| v.get(index))
                    .ok_or_else(|| invalid("SSR PCM alignment incomplete at stream end"))?;
                let samples = input.samples.get(..remaining).ok_or_else(|| {
                    invalid("SSR PCM alignment requires more than one packet of lookahead")
                })?;
                chunks.push(SourceChunk {
                    samples: samples.to_vec(),
                    outputs: input.outputs.to_vec(),
                });
            }
            lanes.push(chunks);
        }
        Ok(AlignedSources {
            stamp: frame.stamp,
            rows: frame.rows,
            lanes,
        })
    }
    /// Preserve each source independently for per-source extension synthesis.
    /// Gain application is deferred; sources are checked for finite values and
    /// bounded geometry, without requiring an unused core-domain mix to fit.
    pub fn submit_sources(
        &mut self,
        stamp: u64,
        rows: usize,
        inputs: &[LaneInput<'_>],
    ) -> Result<Option<AlignedSources>> {
        self.validate(rows, inputs, false)?;
        let sources = self
            .pending
            .front()
            .copied()
            .map(|frame| self.separate(frame, Some(inputs)))
            .transpose()?;
        self.enqueue(stamp, rows, inputs);
        Ok(sources)
    }
    /// Drain separate sources. Failure leaves pending samples and stamps intact.
    pub fn finish_sources(&mut self) -> Result<Option<AlignedSources>> {
        let Some(frame) = self.pending.front().copied() else {
            self.finished = true;
            return Ok(None);
        };
        if self.lanes.iter().any(|v| v.rows != frame.rows) {
            return Err(invalid("SSR PCM alignment incomplete at stream end"));
        }
        let sources = self.separate(frame, None)?;
        self.consume(frame.rows);
        self.pending.pop_front();
        self.finished = true;
        Ok(Some(sources))
    }
    fn mix(
        source: &[f32],
        gains: &[OutputGain],
        destination: &mut [f32],
        start: usize,
        channels: usize,
    ) -> Result<()> {
        for (i, &sample) in source.iter().enumerate() {
            for output in gains {
                let slot = &mut destination[(start + i) * channels + output.channel];
                *slot += sample * output.gain;
                if !slot.is_finite() {
                    return Err(invalid("SSR PCM alignment mixed samples exceed finite f32"));
                }
            }
        }
        Ok(())
    }
    fn consume(&mut self, rows: usize) {
        for lane in &mut self.lanes {
            let mut remaining = rows;
            while remaining > 0 {
                let chunk = lane.chunks.front_mut().unwrap();
                let count = (chunk.samples.len() - chunk.used).min(remaining);
                chunk.used += count;
                remaining -= count;
                if chunk.used == chunk.samples.len() {
                    lane.chunks.pop_front();
                }
            }
            lane.rows -= rows;
        }
    }
    /// Retains one packet to align both ahead and behind lanes. After the first
    /// submission, emits the preceding complete frame with its original stamp.
    /// Invalid data or excessive drift leaves queues/frame stamps unchanged.
    pub fn submit(
        &mut self,
        stamp: u64,
        rows: usize,
        inputs: &[LaneInput<'_>],
    ) -> Result<Option<AlignedFrame>> {
        self.validate(rows, inputs, true)?;
        let emitted = self
            .pending
            .front()
            .copied()
            .map(|frame| self.render(frame, Some(inputs)))
            .transpose()?;
        self.enqueue(stamp, rows, inputs);
        Ok(emitted)
    }
    fn enqueue(&mut self, stamp: u64, rows: usize, inputs: &[LaneInput<'_>]) {
        for (lane, input) in self.lanes.iter_mut().zip(inputs) {
            lane.chunks.push_back(Chunk {
                samples: input.samples.to_vec(),
                used: 0,
                outputs: input.outputs.to_vec(),
            });
            lane.rows += input.samples.len();
        }
        if let Some(frame) = self.pending.pop_front() {
            self.consume(frame.rows);
        }
        self.pending.push_back(Pending { stamp, rows });
    }
    /// Emits the final complete frame. Unequal stream ends refuse without
    /// inserting silence or discarding queued contributions. Repeated finish is
    /// empty; new submissions require reset.
    pub fn finish(&mut self) -> Result<Option<AlignedFrame>> {
        let Some(frame) = self.pending.front().copied() else {
            self.finished = true;
            return Ok(None);
        };
        if self.lanes.iter().any(|v| v.rows != frame.rows) {
            return Err(invalid("SSR PCM alignment incomplete at stream end"));
        }
        let output = self.render(frame, None)?;
        self.consume(frame.rows);
        self.pending.pop_front();
        self.finished = true;
        Ok(Some(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_source_prefix_is_silent_and_reordering_preserves_old_samples_and_gains() {
        let target = vec![0.0; 1024];
        let old = vec![3.0; 1024];
        let new = vec![10.0; 1024];
        let unity = [OutputGain {
            channel: 0,
            gain: 1.0,
        }];
        let old_gain = [OutputGain {
            channel: 0,
            gain: 2.0,
        }];
        let new_gain = [OutputGain {
            channel: 0,
            gain: 0.5,
        }];
        let mut alignment = SsrPcmAlignment::new(1, 2).unwrap();
        assert!(
            alignment
                .submit(
                    11,
                    1024,
                    &[
                        LaneInput {
                            samples: &target,
                            outputs: &unity
                        },
                        LaneInput {
                            samples: &old,
                            outputs: &old_gain
                        }
                    ]
                )
                .unwrap()
                .is_none()
        );
        let checkpoint = alignment.clone();
        let before = alignment.retained_payload_bytes().unwrap();
        for bad in [
            &[Some(0), Some(0), None][..],
            &[None, Some(0), Some(1)][..],
            &[Some(0), None][..],
        ] {
            assert!(alignment.extend_lanes(bad).is_err());
            assert_eq!(alignment.retained_payload_bytes().unwrap(), before);
        }
        alignment.extend_lanes(&[Some(0), None, Some(1)]).unwrap();
        let inputs = [
            LaneInput {
                samples: &target,
                outputs: &unity,
            },
            LaneInput {
                samples: &new,
                outputs: &new_gain,
            },
            LaneInput {
                samples: &old,
                outputs: &old_gain,
            },
        ];
        let previous = alignment.submit(12, 1024, &inputs).unwrap().unwrap();
        assert_eq!(
            previous,
            AlignedFrame {
                stamp: 11,
                samples: vec![6.0; 1024]
            }
        );
        assert_eq!(
            alignment.finish().unwrap().unwrap(),
            AlignedFrame {
                stamp: 12,
                samples: vec![11.0; 1024]
            }
        );
        assert!(
            alignment
                .extend_lanes(&[Some(0), Some(1), Some(2)])
                .is_err()
        );
        alignment = checkpoint;
        alignment.extend_lanes(&[Some(0), None, Some(1)]).unwrap();
        assert_eq!(
            alignment.submit(12, 1024, &inputs).unwrap().unwrap(),
            previous
        );
    }
    #[test]
    fn new_source_count_is_bounded_and_every_old_lane_must_be_preserved() {
        let mut alignment = SsrPcmAlignment::new(1, 1).unwrap();
        let mut order = vec![None; 17];
        order[0] = Some(0);
        alignment.extend_lanes(&order).unwrap();
        let bytes = alignment.retained_payload_bytes().unwrap();
        let mut too_many: Vec<_> = (0..17).map(Some).collect();
        too_many.push(None);
        assert!(alignment.extend_lanes(&too_many).is_err());
        let mut lost: Vec<_> = (0..17).map(Some).collect();
        lost[16] = None;
        assert_eq!(
            alignment.extend_lanes(&lost).unwrap_err().to_string(),
            "SSR PCM alignment cannot discard an existing lane"
        );
        assert_eq!(alignment.retained_payload_bytes().unwrap(), bytes);
    }
    #[test]
    fn legal_window_transitions_stay_within_one_packet_lookahead() {
        let extents = [1024, 1472, 1024, 576];
        let next = [[0, 1], [2, 3], [2, 3], [0, 1]];
        let gains = [
            OutputGain {
                channel: 0,
                gain: 1.0,
            },
            OutputGain {
                channel: 1,
                gain: 1.0,
            },
        ];
        let mut random = 0x6183d27bu32;
        for initial in 0..16 {
            let mut states = [initial / 4, initial % 4];
            let mut alignment = SsrPcmAlignment::new(2, 2).unwrap();
            for packet in 0..64 {
                let signals = [vec![1.0; extents[states[0]]], vec![2.0; extents[states[1]]]];
                let lanes = [
                    LaneInput {
                        samples: &signals[0],
                        outputs: &gains[..1],
                    },
                    LaneInput {
                        samples: &signals[1],
                        outputs: &gains[1..],
                    },
                ];
                let output = alignment.submit(packet, signals[0].len(), &lanes).unwrap();
                if let Some(frame) = output {
                    assert_eq!(frame.stamp, packet - 1);
                    assert!(frame.samples.chunks_exact(2).all(|r| r == [1.0, 2.0]));
                }
                for state in &mut states {
                    random ^= random << 13;
                    random ^= random >> 17;
                    random ^= random << 5;
                    *state = next[*state][(random & 1) as usize];
                }
            }
        }
    }
    #[test]
    fn late_lane_mix_overflow_preserves_every_queue_and_stamp() {
        let mut alignment = SsrPcmAlignment::new(1, 2).unwrap();
        let gain = [OutputGain {
            channel: 0,
            gain: 1.0,
        }];
        let huge = vec![f32::MAX; 1024];
        let first = [
            LaneInput {
                samples: &huge,
                outputs: &gain,
            },
            LaneInput {
                samples: &huge,
                outputs: &gain,
            },
        ];
        alignment.submit(17, 1024, &first).unwrap();
        let zero = vec![0.0; 1024];
        let next = [
            LaneInput {
                samples: &zero,
                outputs: &gain,
            },
            LaneInput {
                samples: &zero,
                outputs: &gain,
            },
        ];
        assert!(
            alignment
                .submit(29, 1024, &next)
                .unwrap_err()
                .to_string()
                .contains("mixed samples")
        );
        assert_eq!(alignment.pending.front().unwrap().stamp, 17);
        assert!(
            alignment
                .lanes
                .iter()
                .all(|lane| lane.rows == 1024 && lane.chunks[0].used == 0)
        );
    }
    fn inputs<'a>(signals: &'a [Vec<f32>], gains: &'a [Vec<OutputGain>]) -> Vec<LaneInput<'a>> {
        signals
            .iter()
            .zip(gains)
            .map(|(samples, outputs)| LaneInput { samples, outputs })
            .collect()
    }
    #[test]
    fn opposite_window_switches_align_every_sample_and_keep_original_stamps() {
        for layouts in [
            [vec![1024, 1472, 1024, 576, 1024, 1024], vec![1024; 6]],
            [vec![1024; 6], vec![1024, 1472, 1024, 576, 1024, 1024]],
            [
                vec![1024, 1472, 1024, 576, 1024, 1024],
                vec![1024, 1024, 1472, 1024, 576, 1024],
            ],
            [
                vec![1024, 1472, 1024, 576, 1024, 1024],
                vec![1024, 576, 1024, 1472, 1024, 1024],
            ],
        ] {
            let mut alignment = SsrPcmAlignment::new(2, 2).unwrap();
            let mut positions = [0; 2];
            let mut accumulated: [Vec<f32>; 2] = [vec![], vec![]];
            let mut emitted = vec![];
            let gains = vec![
                vec![OutputGain {
                    channel: 0,
                    gain: 1.0,
                }],
                vec![OutputGain {
                    channel: 1,
                    gain: 1.0,
                }],
            ];
            for packet in 0..6 {
                let signals: Vec<_> = (0..2)
                    .map(|lane| {
                        let values: Vec<_> = (positions[lane]
                            ..positions[lane] + layouts[lane][packet])
                            .map(|i| ((i * 7 + lane * 31) % 151) as f32 / 151.0)
                            .collect();
                        positions[lane] += values.len();
                        accumulated[lane].extend_from_slice(&values);
                        values
                    })
                    .collect();
                let out = alignment
                    .submit(
                        900 + packet as u64 * 17,
                        layouts[0][packet],
                        &inputs(&signals, &gains),
                    )
                    .unwrap();
                if packet == 0 {
                    assert!(out.is_none());
                } else {
                    let out = out.unwrap();
                    assert_eq!(out.stamp, 900 + (packet - 1) as u64 * 17);
                    emitted.extend(out.samples);
                }
            }
            let last = alignment.finish().unwrap().unwrap();
            assert_eq!(last.stamp, 985);
            emitted.extend(last.samples);
            let expected: Vec<_> = accumulated[0]
                .iter()
                .zip(&accumulated[1])
                .flat_map(|(&a, &b)| [a, b])
                .collect();
            assert_eq!(emitted, expected);
            assert!(alignment.finish().unwrap().is_none());
        }
    }
    #[test]
    fn coupling_gains_follow_source_chunks_across_target_frame_boundaries() {
        let mut alignment = SsrPcmAlignment::new(2, 3).unwrap();
        let mut output = vec![];
        let mut expected = [vec![], vec![]];
        let extents = [1472, 1024, 576, 1024];
        for (packet, &extent) in extents.iter().enumerate() {
            let gains = vec![
                vec![OutputGain {
                    channel: 0,
                    gain: 1.0,
                }],
                vec![OutputGain {
                    channel: 1,
                    gain: 1.0,
                }],
                vec![
                    OutputGain {
                        channel: 0,
                        gain: 0.5,
                    },
                    OutputGain {
                        channel: 1,
                        gain: if packet % 2 == 0 { 2.0 } else { -1.0 },
                    },
                ],
            ];
            let signals = vec![
                vec![3.0; 1024],
                vec![5.0; 1024],
                vec![(packet + 1) as f32; extent],
            ];
            for _ in 0..extent {
                expected[0].push(3.0 + (packet + 1) as f32 * 0.5);
                expected[1].push(5.0 + (packet + 1) as f32 * gains[2][1].gain);
            }
            if let Some(frame) = alignment
                .submit(packet as u64, 1024, &inputs(&signals, &gains))
                .unwrap()
            {
                output.extend(frame.samples);
            }
        }
        output.extend(alignment.finish().unwrap().unwrap().samples);
        assert_eq!(
            output,
            expected[0]
                .iter()
                .zip(&expected[1])
                .flat_map(|(&a, &b)| [a, b])
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn malformed_input_overflow_checkpoint_and_reset_are_transactional() {
        let mut alignment = SsrPcmAlignment::new(1, 1).unwrap();
        let values = vec![1.0; 1024];
        let gains = [OutputGain {
            channel: 0,
            gain: 1.0,
        }];
        alignment
            .submit(
                123,
                1024,
                &[LaneInput {
                    samples: &values,
                    outputs: &gains,
                }],
            )
            .unwrap();
        let checkpoint = alignment.clone();
        let bad = [OutputGain {
            channel: 0,
            gain: f32::MAX,
        }];
        let huge = vec![f32::MAX; 1024];
        assert!(
            alignment
                .submit(
                    456,
                    1024,
                    &[LaneInput {
                        samples: &huge,
                        outputs: &bad
                    }]
                )
                .is_err()
        );
        assert_eq!(alignment.pending.front().unwrap().stamp, 123);
        assert_eq!(alignment.lanes[0].rows, 1024);
        alignment = checkpoint.clone();
        let actual = alignment
            .submit(
                456,
                1024,
                &[LaneInput {
                    samples: &values,
                    outputs: &gains,
                }],
            )
            .unwrap();
        let mut reference = checkpoint;
        assert_eq!(
            actual,
            reference
                .submit(
                    456,
                    1024,
                    &[LaneInput {
                        samples: &values,
                        outputs: &gains
                    }]
                )
                .unwrap()
        );
        alignment.reset();
        assert!(alignment.finish().unwrap().is_none());
        alignment.reset();
        assert!(
            alignment
                .submit(
                    123,
                    1024,
                    &[LaneInput {
                        samples: &values,
                        outputs: &gains
                    }]
                )
                .unwrap()
                .is_none()
        );
        assert!(SsrPcmAlignment::new(0, 1).is_err());
    }
    #[test]
    fn unfinished_drift_has_a_bounded_refusal_without_losing_pending_frame() {
        let mut alignment = SsrPcmAlignment::new(2, 2).unwrap();
        let gains = vec![
            vec![OutputGain {
                channel: 0,
                gain: 1.0,
            }],
            vec![OutputGain {
                channel: 1,
                gain: 1.0,
            }],
        ];
        let signals = vec![vec![1.0; 1472], vec![2.0; 576]];
        alignment
            .submit(100, 1472, &inputs(&signals, &gains))
            .unwrap();
        assert!(alignment.finish().is_err());
        assert_eq!(alignment.pending.front().unwrap().stamp, 100);
        let next = vec![vec![1.0; 1024], vec![2.0; 1024]];
        let frame = alignment
            .submit(200, 1024, &inputs(&next, &gains))
            .unwrap()
            .unwrap();
        assert_eq!(frame.stamp, 100);
        assert!(frame.samples.chunks_exact(2).all(|r| r == [1.0, 2.0]));
        assert!(alignment.finish().is_err());
    }
    #[test]
    fn separate_sources_preserve_drift_gain_boundaries_checkpoint_and_eof() {
        let mut queue = SsrPcmAlignment::new(1, 2).unwrap();
        let first = vec![vec![2.0; 1472], vec![3.0; 576]];
        let gains = vec![
            vec![OutputGain {
                channel: 0,
                gain: 1.0,
            }],
            vec![OutputGain {
                channel: 0,
                gain: 0.5,
            }],
        ];
        assert!(
            queue
                .submit_sources(10, 1024, &inputs(&first, &gains))
                .unwrap()
                .is_none()
        );
        let saved = queue.clone();
        let next = vec![vec![5.0; 576], vec![7.0; 1472]];
        let next_gains = vec![
            vec![OutputGain {
                channel: 0,
                gain: 0.25,
            }],
            vec![OutputGain {
                channel: 0,
                gain: -1.0,
            }],
        ];
        let frame = queue
            .submit_sources(20, 1024, &inputs(&next, &next_gains))
            .unwrap()
            .unwrap();
        assert_eq!((frame.stamp, frame.rows), (10, 1024));
        assert_eq!(frame.lanes[0].len(), 1);
        assert_eq!(frame.lanes[0][0].samples, vec![2.0; 1024]);
        assert_eq!(frame.lanes[1][0].samples, vec![3.0; 576]);
        assert_eq!(frame.lanes[1][0].outputs, gains[1]);
        assert_eq!(frame.lanes[1][1].samples, vec![7.0; 448]);
        assert_eq!(frame.lanes[1][1].outputs, next_gains[1]);
        let mut replay = saved;
        assert_eq!(
            Some(frame),
            replay
                .submit_sources(20, 1024, &inputs(&next, &next_gains))
                .unwrap()
        );
        let tail = queue.finish_sources().unwrap().unwrap();
        assert_eq!((tail.stamp, tail.rows), (20, 1024));
        assert_eq!(tail.lanes[0][0].samples, vec![2.0; 448]);
        assert_eq!(tail.lanes[0][1].samples, vec![5.0; 576]);
        assert_eq!(tail.lanes[0][1].outputs, next_gains[0]);
        assert_eq!(tail.lanes[1][0].samples, vec![7.0; 1024]);
        assert_eq!(queue.finish_sources().unwrap(), None);
        assert_eq!(replay.finish_sources().unwrap(), Some(tail));
    }
    #[test]
    fn separate_source_failure_preserves_pending_state() {
        let mut queue = SsrPcmAlignment::new(1, 1).unwrap();
        let samples = vec![vec![1.0; 576]];
        let gains = vec![vec![OutputGain {
            channel: 0,
            gain: 1.0,
        }]];
        queue
            .submit_sources(7, 1024, &inputs(&samples, &gains))
            .unwrap();
        let before = queue.retained_payload_bytes().unwrap();
        assert!(queue.finish_sources().is_err());
        assert_eq!(queue.retained_payload_bytes().unwrap(), before);
        assert_eq!(queue.pending.front().unwrap().stamp, 7);
        let short = vec![vec![1.0; 1]];
        assert!(
            queue
                .submit_sources(8, 1024, &inputs(&short, &gains))
                .is_err()
        );
        assert_eq!(queue.retained_payload_bytes().unwrap(), before);
        let next = vec![vec![4.0; 1472]];
        let frame = queue
            .submit_sources(8, 1024, &inputs(&next, &gains))
            .unwrap()
            .unwrap();
        assert_eq!(frame.lanes[0][0].samples.len(), 576);
        assert_eq!(frame.lanes[0][1].samples.len(), 448);
        assert_eq!(
            queue.finish_sources().unwrap().unwrap().lanes[0][0].samples,
            vec![4.0; 1024]
        );
    }
    #[test]
    fn separate_sources_defer_core_mix_overflow_to_the_extension_consumer() {
        let signals = vec![vec![f32::MAX; 1024], vec![f32::MAX; 1024]];
        let gains = vec![
            vec![OutputGain {
                channel: 0,
                gain: 2.0
            }];
            2
        ];
        let mut mixed = SsrPcmAlignment::new(1, 2).unwrap();
        assert!(mixed.submit(0, 1024, &inputs(&signals, &gains)).is_err());
        let mut separate = SsrPcmAlignment::new(1, 2).unwrap();
        separate
            .submit_sources(0, 1024, &inputs(&signals, &gains))
            .unwrap();
        let frame = separate.finish_sources().unwrap().unwrap();
        assert_eq!(frame.lanes[0][0].samples, signals[0]);
        assert_eq!(frame.lanes[1][0].outputs, gains[1]);
    }
}
