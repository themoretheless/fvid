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
#[derive(Clone, Debug)]
pub struct MovieVideoMetadata {
    pub options: crate::owned_matroska::TrackOptions,
    pub file: crate::owned_matroska::FileMetadata,
    pub name: String,
    pub language: String,
}
fn transformed_metadata(
    mut metadata: MovieVideoMetadata,
    width: u32,
    height: u32,
    t: fvid_cuda::Nv12Transform,
    full_range: bool,
) -> Result<MovieVideoMetadata, String> {
    validate_transform(width, height, t)?;
    if let Some(video) = metadata.options.video.as_mut() {
        if video
            .colour
            .is_some_and(|colour| colour.full_range != full_range)
        {
            return Err(
                "movie black range differs from source pixels; range conversion is required".into(),
            );
        }
        if video.colour.is_none() {
            video.colour = Some(crate::owned_matroska::ColourDescription {
                primaries: 2,
                transfer: 2,
                matrix: 2,
                full_range,
            });
        }
        let [left, right, top, bottom] = video.crop;
        let visible_right = width.checked_sub(right).ok_or("invalid source crop")?;
        let visible_bottom = height.checked_sub(bottom).ok_or("invalid source crop")?;
        let out_right = t.crop_x + t.out_width;
        let out_bottom = t.crop_y + t.out_height;
        if left.max(t.crop_x) >= visible_right.min(out_right)
            || top.max(t.crop_y) >= visible_bottom.min(out_bottom)
        {
            return Err("movie transform removes the visible image".into());
        }
        video.crop = [
            left.saturating_sub(t.crop_x),
            out_right.saturating_sub(visible_right),
            top.saturating_sub(t.crop_y),
            out_bottom.saturating_sub(visible_bottom),
        ];
        if t.hflip {
            video.crop.swap(0, 1);
        }
        if t.vflip {
            video.crop.swap(2, 3);
        }
    }
    Ok(metadata)
}
/// Owns a reusable GPU output surface and renders each movie event without libav.
/// Complete external uses of `buffer()` before advancing or closing this renderer.
pub struct AvcMovieRenderer<R: Read + Seek> {
    reader: std::mem::ManuallyDrop<AvcMovieReader<R>>,
    output: std::mem::ManuallyDrop<fvid_cuda::Nv12Buffer>,
    filter: std::mem::ManuallyDrop<fvid_cuda::Nv12Processor>,
    transform: fvid_cuda::Nv12Transform,
    full_range: bool,
    metadata: MovieVideoMetadata,
    failed: bool,
}
impl<R: Read + Seek> AvcMovieRenderer<R> {
    pub fn new(
        reader: AvcMovieReader<R>,
        transform: fvid_cuda::Nv12Transform,
        full_range: bool,
    ) -> Result<Self, String> {
        let (width, height) = reader.source.coded_dimensions();
        let metadata = transformed_metadata(
            reader.source.video_metadata(),
            width,
            height,
            transform,
            full_range,
        )?;
        let output =
            fvid_cuda::Nv12Buffer::new(reader.ordinal, transform.out_width, transform.out_height)?;
        let mut filter = fvid_cuda::Nv12Processor::new(reader.ordinal)?;
        filter.follow_stream(output.stream_handle()?);
        Ok(Self {
            reader: std::mem::ManuallyDrop::new(reader),
            output: std::mem::ManuallyDrop::new(output),
            filter: std::mem::ManuallyDrop::new(filter),
            transform,
            full_range,
            metadata,
            failed: false,
        })
    }
    pub fn metadata(&self) -> &MovieVideoMetadata {
        &self.metadata
    }
    pub(crate) fn ordinal(&self) -> usize {
        self.reader.ordinal
    }
    pub fn buffer(&self) -> &fvid_cuda::Nv12Buffer {
        &self.output
    }
    pub fn media_timescale(&self) -> u32 {
        self.reader.media_timescale()
    }
    pub fn render_next(&mut self) -> Result<Option<Presentation>, String> {
        if self.failed {
            return Err("native movie renderer failed; reopen it".into());
        }
        let result = self.render_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn render_inner(&mut self) -> Result<Option<Presentation>, String> {
        let Some(presented) = self.reader.next_presentation()? else {
            return Ok(None);
        };
        if let Some(frame) = presented.frame {
            let surface = self.reader.map(&frame)?;
            let (width, height) = self.reader.source.coded_dimensions();
            let uv = surface
                .pointer
                .checked_add(u64::from(surface.pitch) * u64::from(height))
                .ok_or("NVDEC UV pointer overflow")?;
            let source = fvid_cuda::Nv12View {
                y: surface.pointer,
                uv,
                pitch_y: surface.pitch,
                pitch_uv: surface.pitch,
                width,
                height,
            };
            let operation = self
                .filter
                .apply(source, self.output.view()?, self.transform);
            // Wait even when launch fails: an earlier copy may already be queued.
            // Keep the decoder mapping alive if completion cannot be established.
            self.filter.synchronize()?;
            self.output.synchronize()?;
            self.reader.unmap(surface.slot)?;
            operation?;
        } else {
            self.output.fill_black(self.full_range)?;
        }
        Ok(Some(presented.event))
    }
    pub fn close(&mut self) -> Result<(), String> {
        self.failed = true;
        self.filter.synchronize()?;
        self.output.synchronize()?;
        self.reader.close()
    }
}
impl<R: Read + Seek> Drop for AvcMovieRenderer<R> {
    fn drop(&mut self) {
        // A failed wait must not release allocations still used by the filter.
        // Retain all owners rather than risking device use-after-free.
        if self.filter.synchronize().is_err()
            || self.output.synchronize().is_err()
            || self.reader.close().is_err()
        {
            return;
        }
        // SAFETY: All GPU work completed and the decoder closed. Each field is
        // manually dropped exactly once, and no external use is permitted here.
        unsafe {
            std::mem::ManuallyDrop::drop(&mut self.filter);
            std::mem::ManuallyDrop::drop(&mut self.reader);
            std::mem::ManuallyDrop::drop(&mut self.output);
        }
    }
}

fn validate_transform(width: u32, height: u32, t: fvid_cuda::Nv12Transform) -> Result<(), String> {
    if t.out_width == 0
        || t.out_height == 0
        || (t.crop_x | t.crop_y | t.out_width | t.out_height) & 1 != 0
        || t.crop_x
            .checked_add(t.out_width)
            .is_none_or(|end| end > width)
        || t.crop_y
            .checked_add(t.out_height)
            .is_none_or(|end| end > height)
    {
        return Err("native movie crop must be even and inside the coded image".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn render_metadata_preserves_display_and_maps_crop_through_flips() {
        use crate::owned_matroska::{
            ColourDescription, ContentLight, HdrMetadata, TrackOptions, VideoMetadata,
        };
        let metadata = MovieVideoMetadata {
            file: Default::default(),
            name: "synthetic".into(),
            language: "eng".into(),
            options: TrackOptions {
                rotation: 90,
                video: Some(VideoMetadata {
                    crop: [2, 4, 6, 8],
                    pixel_aspect: (2, 1),
                    colour: Some(ColourDescription {
                        primaries: 9,
                        transfer: 16,
                        matrix: 9,
                        full_range: false,
                    }),
                    hdr: HdrMetadata {
                        light: ContentLight {
                            max_cll: 1000.0,
                            max_fall: 400.0,
                        },
                        ..Default::default()
                    },
                }),
                ..Default::default()
            },
        };
        let t = fvid_cuda::Nv12Transform {
            crop_x: 2,
            crop_y: 6,
            out_width: 60,
            out_height: 40,
            hflip: true,
            vflip: true,
        };
        let transformed = transformed_metadata(metadata.clone(), 64, 48, t, false).unwrap();
        let video = transformed.options.video.unwrap();
        assert_eq!(video.crop, [2, 0, 6, 0]);
        assert_eq!(video.pixel_aspect, (2, 1));
        assert_eq!(video.colour, metadata.options.video.unwrap().colour);
        assert_eq!(video.hdr, metadata.options.video.unwrap().hdr);
        assert_eq!(
            (
                transformed.options.rotation,
                transformed.name.as_str(),
                transformed.language.as_str()
            ),
            (90, "synthetic", "eng")
        );
        assert!(transformed_metadata(metadata.clone(), 64, 48, t, true).is_err());
        assert!(
            transformed_metadata(
                metadata,
                64,
                48,
                fvid_cuda::Nv12Transform {
                    out_width: 2,
                    out_height: 2,
                    ..Default::default()
                },
                false
            )
            .is_err()
        );
    }
    #[test]
    fn render_geometry_is_checked_before_device_allocation() {
        use fvid_cuda::Nv12Transform;
        let valid = Nv12Transform {
            out_width: 32,
            out_height: 24,
            ..Default::default()
        };
        assert!(validate_transform(64, 48, valid).is_ok());
        for invalid in [
            Nv12Transform::default(),
            Nv12Transform { crop_x: 1, ..valid },
            Nv12Transform {
                crop_x: u32::MAX - 1,
                ..valid
            },
            Nv12Transform {
                out_height: 50,
                ..valid
            },
        ] {
            assert!(validate_transform(64, 48, invalid).is_err());
        }
    }
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
        let reader = AvcMovieReader::new(source, 0, 32, 2, 1000, 8).unwrap();
        let mut renderer = AvcMovieRenderer::new(
            reader,
            fvid_cuda::Nv12Transform {
                out_width: width,
                out_height: height,
                hflip: true,
                ..Default::default()
            },
            false,
        )
        .unwrap();
        let mut count = 0;
        let mut blanks = 0;
        while let Some(event) = renderer.render_next().unwrap() {
            let expected = expected[count];
            assert_eq!(
                (event.sample, event.start, event.end),
                (expected.sample, expected.start, expected.end)
            );
            assert_ne!(renderer.buffer().view().unwrap().y, 0);
            if event.sample.is_none() {
                blanks += 1;
            }
            count += 1;
        }
        assert_eq!(count, expected.len());
        assert_eq!(blanks, 2);
        renderer.close().unwrap();
        assert!(renderer.render_next().is_err());
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
