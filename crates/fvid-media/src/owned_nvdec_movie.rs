//! Bound decode-order caching and replay for the owned MP4 movie clock.
use crate::owned_nvdec_avc_decoder::{AvcNvdecDecoder, DecodedAvc};
use crate::owned_nvdec_mp4::{AvcMp4Input, Presentation};
use std::{
    collections::BTreeSet,
    io::{Read, Seek},
};
struct Epoch {
    end: usize,
    needed: BTreeSet<usize>,
}
enum Action<T> {
    Decode(usize),
    Ready(Presentation, Option<T>),
    Rewind,
    End,
}
struct Queue<T> {
    events: Vec<Presentation>,
    epochs: Vec<Epoch>,
    epoch: usize,
    at: usize,
    next_decode: usize,
    cache: Vec<(usize, T)>,
    limit: usize,
}
impl<T> Queue<T> {
    fn new(events: Vec<Presentation>, limit: usize) -> Result<Self, String> {
        if limit == 0 {
            return Err("movie decode cache must have positive capacity".into());
        }
        let mut epochs = Vec::new();
        let mut needed = BTreeSet::new();
        for (index, event) in events.iter().enumerate() {
            if event.start >= event.end {
                return Err("movie presentation has invalid extent".into());
            }
            if index > 0
                && (event.range != events[index - 1].range
                    || event.sample.is_some_and(|sample| needed.contains(&sample)))
            {
                epochs.push(Epoch { end: index, needed });
                needed = BTreeSet::new();
            }
            if let Some(sample) = event.sample {
                needed.insert(sample);
            }
        }
        epochs.push(Epoch {
            end: events.len(),
            needed,
        });
        Ok(Self {
            events,
            epochs,
            epoch: 0,
            at: 0,
            next_decode: 0,
            cache: Vec::new(),
            limit,
        })
    }
    fn next(&mut self) -> Result<Action<T>, String> {
        if self.at == self.events.len() {
            return Ok(Action::End);
        }
        if self.at == self.epochs[self.epoch].end {
            return Ok(Action::Rewind);
        }
        let event = self.events[self.at];
        let Some(sample) = event.sample else {
            self.at += 1;
            return Ok(Action::Ready(event, None));
        };
        if let Some(index) = self.cache.iter().position(|entry| entry.0 == sample) {
            let (_, frame) = self.cache.swap_remove(index);
            self.at += 1;
            self.epochs[self.epoch].needed.remove(&sample);
            return Ok(Action::Ready(event, Some(frame)));
        }
        if sample < self.next_decode {
            return Err("movie queue lost a required decoded picture".into());
        }
        if self.epochs[self.epoch].needed.contains(&self.next_decode)
            && self.cache.len() >= self.limit
        {
            return Err("movie B-picture cache exceeds configured capacity".into());
        }
        Ok(Action::Decode(self.next_decode))
    }
    fn supply(&mut self, index: usize, frame: T) -> Result<(), String> {
        if index != self.next_decode {
            return Err("movie decoder supplied an out-of-order sample".into());
        }
        let next = index.checked_add(1).ok_or("movie decode index overflow")?;
        if self.epochs[self.epoch].needed.contains(&index) {
            if self.cache.len() >= self.limit {
                return Err("movie decode cache is full".into());
            }
            self.cache.try_reserve(1).map_err(|e| e.to_string())?;
            self.cache.push((index, frame));
        }
        self.next_decode = next;
        Ok(())
    }
    fn rewound(&mut self) -> Result<(), String> {
        if self.at != self.epochs[self.epoch].end || self.epoch + 1 >= self.epochs.len() {
            return Err("movie queue has no pending rewind".into());
        }
        self.cache.clear();
        self.epoch += 1;
        self.next_decode = 0;
        Ok(())
    }
}
pub struct PresentedAvc {
    pub event: Presentation,
    /// None denotes an explicit blank span; rendering supplies a black frame.
    pub frame: Option<DecodedAvc>,
}
/// Pulls frames on the movie clock through own demux, POC/DPB and direct NVDEC.
/// Finish all mapped/raw-pointer use before asking for the next presentation:
/// replay may close the previous decoder and invalidate its surfaces/tickets.
pub struct AvcMovieReader<R> {
    source: AvcMp4Input<R>,
    decoder: AvcNvdecDecoder,
    queue: Queue<DecodedAvc>,
    scratch: Vec<u8>,
    ordinal: usize,
    decode_surfaces: u32,
    output_surfaces: u32,
    failed: bool,
}
impl<R: Read + Seek> AvcMovieReader<R> {
    pub fn new(
        mut source: AvcMp4Input<R>,
        ordinal: usize,
        decode_surfaces: u32,
        output_surfaces: u32,
        max_events: usize,
        cache_frames: usize,
    ) -> Result<Self, String> {
        let queue = Queue::new(source.movie_presentations(max_events)?, cache_frames)?;
        source.rewind_packets();
        let decoder = source.create_decoder(ordinal, decode_surfaces, output_surfaces)?;
        Ok(Self {
            source,
            decoder,
            queue,
            scratch: Vec::new(),
            ordinal,
            decode_surfaces,
            output_surfaces,
            failed: false,
        })
    }
    pub fn media_timescale(&self) -> u32 {
        self.source.track().timescale
    }
    pub fn next_presentation(&mut self) -> Result<Option<PresentedAvc>, String> {
        if self.failed {
            return Err("native movie reader failed; reopen it".into());
        }
        let result = self.next_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn next_inner(&mut self) -> Result<Option<PresentedAvc>, String> {
        loop {
            match self.queue.next()? {
                Action::End => return Ok(None),
                Action::Ready(event, frame) => return Ok(Some(PresentedAvc { event, frame })),
                Action::Decode(index) => {
                    let packet = self
                        .source
                        .decode_next(&mut self.decoder, &mut self.scratch)?
                        .ok_or("movie presentation requires a missing source packet")?;
                    if packet.index != index {
                        return Err("movie source cursor does not match decode queue".into());
                    }
                    self.queue.supply(index, packet.frame)?;
                }
                Action::Rewind => {
                    self.decoder.close()?;
                    self.source.rewind_packets();
                    self.decoder = self.source.create_decoder(
                        self.ordinal,
                        self.decode_surfaces,
                        self.output_surfaces,
                    )?;
                    self.queue.rewound()?;
                }
            }
        }
    }
    pub fn map(&mut self, frame: &DecodedAvc) -> Result<fvid_cuda::NvdecSurface, String> {
        self.decoder.map(frame)
    }
    pub fn unmap(&mut self, slot: usize) -> Result<(), String> {
        self.decoder.unmap(slot)
    }
    pub fn close(&mut self) -> Result<(), String> {
        self.decoder.close()?;
        self.failed = true;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn frame(index: usize) -> Presentation {
        Presentation {
            sample: Some(index),
            range: 0,
            start: index as i64,
            end: index as i64 + 1,
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    #[ignore = "requires an NVIDIA CUDA device with NVDEC"]
    fn synthetic_movie_blanks_and_repeated_ranges_present_on_nvidia() {
        const EMPTY: &[u8] =
            include_bytes!("../../../tests/fixtures/playback-errors/edit-empty-spans.mov");
        let source = AvcMp4Input::open(
            std::io::Cursor::new(EMPTY),
            crate::owned_mp4::Limits::default(),
        )
        .unwrap();
        let expected = source.movie_presentations(1000).unwrap();
        let (width, height) = source.coded_dimensions();
        let mut blank = fvid_cuda::Nv12Buffer::new(0, width, height).unwrap();
        let mut reader = AvcMovieReader::new(source, 0, 32, 2, 1000, 8).unwrap();
        let mut count = 0;
        let mut blanks = 0;
        while let Some(presented) = reader.next_presentation().unwrap() {
            let expected = expected[count];
            assert_eq!(
                (
                    presented.event.sample,
                    presented.event.start,
                    presented.event.end
                ),
                (expected.sample, expected.start, expected.end)
            );
            if let Some(frame) = presented.frame {
                let surface = reader.map(&frame).unwrap();
                assert_ne!(surface.pointer, 0);
                reader.unmap(surface.slot).unwrap();
            } else {
                blank.fill_black(false).unwrap();
                blanks += 1;
            }
            count += 1;
        }
        assert_eq!(count, expected.len());
        assert_eq!(blanks, 2);
        reader.close().unwrap();
        assert!(reader.next_presentation().is_err());
    }
    #[test]
    fn b_order_and_repeats_rewind_without_retaining_the_previous_range() {
        let mut queue = Queue::new(
            vec![frame(0), frame(2), frame(1), frame(0), frame(2), frame(1)],
            2,
        )
        .unwrap();
        let mut output = Vec::new();
        let mut rewinds = 0;
        loop {
            match queue.next().unwrap() {
                Action::Decode(index) => queue.supply(index, index).unwrap(),
                Action::Ready(_, value) => output.push(value.unwrap()),
                Action::Rewind => {
                    assert!(queue.cache.is_empty());
                    rewinds += 1;
                    queue.rewound().unwrap();
                }
                Action::End => break,
            }
        }
        assert_eq!(output, vec![0, 2, 1, 0, 2, 1]);
        assert_eq!(rewinds, 1);
    }
    #[test]
    fn preroll_and_blank_spans_do_not_fill_cache_for_a_later_repeat() {
        let blank = Presentation {
            sample: None,
            range: 0,
            start: 0,
            end: 1,
        };
        let mut queue = Queue::new(
            vec![
                blank,
                frame(100),
                Presentation {
                    range: 1,
                    ..frame(0)
                },
            ],
            1,
        )
        .unwrap();
        assert!(matches!(queue.next().unwrap(), Action::Ready(_, None)));
        for index in 0..=100 {
            assert!(matches!(queue.next().unwrap(), Action::Decode(i) if i == index));
            queue.supply(index, index).unwrap();
            assert!(queue.cache.len() <= 1);
        }
        assert!(matches!(queue.next().unwrap(), Action::Ready(_, Some(100))));
        assert!(matches!(queue.next().unwrap(), Action::Rewind));
        queue.rewound().unwrap();
        assert!(matches!(queue.next().unwrap(), Action::Decode(0)));
    }
    #[test]
    fn wrong_order_and_insufficient_b_cache_refuse_before_more_decode() {
        let mut queue = Queue::new(vec![frame(2), frame(1)], 1).unwrap();
        assert!(queue.supply(1, 1).is_err());
        queue.supply(0, 0).unwrap();
        queue.supply(1, 1).unwrap();
        assert!(queue.next().is_err());
    }
}
