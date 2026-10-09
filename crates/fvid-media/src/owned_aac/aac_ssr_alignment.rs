//! Bounded packet-boundary PCM alignment for independently switched AAC SSR lanes.
//! Each lane is mono with per-chunk output gains. Frame stamps belong to the
//! caller's clock. Native SSR dispatch uses this queue when lane extents differ.
use super::{Result, invalid};
use std::collections::VecDeque;
const MAX_ROWS: usize = 1472;
const MAX_QUEUED: usize = 2 * MAX_ROWS;
#[derive(Clone, Copy, Debug)]
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
    fn validate(&self, rows: usize, inputs: &[LaneInput<'_>]) -> Result<()> {
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
            if input.samples.iter().any(|sample| {
                input
                    .outputs
                    .iter()
                    .any(|gain| !(*sample * gain.gain).is_finite())
            }) {
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
        self.validate(rows, inputs)?;
        let emitted = self
            .pending
            .front()
            .copied()
            .map(|frame| self.render(frame, Some(inputs)))
            .transpose()?;
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
        Ok(emitted)
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
}
