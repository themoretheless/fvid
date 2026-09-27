//! Native WebM/Matroska VP9 and AV1 playback with bounded decode-ahead and source timestamps.
use crate::{
    codec::{
        vp9,
        vp9_decoder::{Decoded, Decoder},
    },
    color::hdr::{ColourDescription, HdrMetadata},
    container::webm::{Limits, WebmReader},
    invalid,
    playback_native::{AvcColour, Planar8},
    Result,
};
use std::{
    io::{Read, Seek},
    time::Duration,
};
enum Picture {
    Vp9(Decoded),
    Av1(crate::codec::av1_decoder::Decoded),
}
enum VideoDecoder {
    Vp9(Decoder),
    Av1(crate::codec::av1_decoder::Decoder),
}
struct Frame {
    decoded: Picture,
    pts: i64,
}
/// A decoded visible frame plus its display colour metadata, ready for either
/// CPU RGB conversion or packing into 8-bit planes for the GPU.
struct CurrentFrame {
    decoded: Picture,
    size: [u32; 2],
    depth: u8,
    full_range: bool,
    color_space: u8,
    monochrome: bool,
}
pub struct WebmVideoReader<R> {
    demux: WebmReader<R>,
    decoder: VideoDecoder,
    track: u64,
    index: usize,
    pending: Option<Frame>,
    rgb: Vec<u8>,
    dimensions: [usize; 2],
    pixel_aspect: (u32, u32),
    /// The stated crop borders as pixel insets into the coded frame, `[0; 4]`
    /// when the track states none it can keep.
    insets: [u32; 4],
    /// The signal the track's own `Colour` element states, zeroes where it wrote
    /// none.
    colour: ColourDescription,
    /// The light the track's `MasteringMetadata` and `MaxCLL`/`MaxFALL` name.
    hdr: HdrMetadata,
    /// Which of the two codecs the track's CodecID picked, named for a reader
    /// rather than for a match arm.
    codec: &'static str,
    start: u128,
    end: u128,
    base: Option<i64>,
    last_duration: u64,
    rgb_budget: usize,
    failed: bool,
    frames: u64,
}
impl<R: Read + Seek> WebmVideoReader<R> {
    pub fn open(reader: R, budget: usize) -> Result<Self> {
        let demux = WebmReader::open(reader, Limits::default())?;
        let track = demux
            .tracks
            .iter()
            .find(|t| t.kind == 1 && matches!(t.codec.as_str(), "V_VP9" | "V_AV1"))
            .ok_or_else(|| invalid("WebM/Matroska has no supported VP9 or AV1 video track"))?;
        let av1 = track.codec == "V_AV1";
        let pixel_aspect = track.pixel_aspect();
        // The validated crop borders are each smaller than the coded size they
        // divide; one that cannot be a pixel offset on this machine is read as
        // a file stating a crop it cannot keep.
        let insets = {
            let mut sides = [0u32; 4];
            let mut keeps = true;
            for (side, value) in sides.iter_mut().zip(track.crop) {
                *side = u32::try_from(value).unwrap_or(0);
                keeps &= u32::try_from(value).is_ok();
            }
            if keeps {
                sides
            } else {
                [0; 4]
            }
        };
        // What the muxer wrote about the signal, kept beside the track it
        // describes. An AV1 stream states the same triple again in its own
        // sequence header, and a track with no `Colour` element is graded by
        // that instead, so the two are read apart.
        let (colour, hdr) = (track.colour, track.hdr);
        let track = track.number;
        let rgb_budget = budget / 4;
        Ok(Self {
            demux,
            decoder: if av1 {
                VideoDecoder::Av1(crate::codec::av1_decoder::Decoder::new(
                    (budget - rgb_budget) / 12 * 10,
                ))
            } else {
                VideoDecoder::Vp9(Decoder::new((budget - rgb_budget) / 12 * 10))
            },
            track,
            index: 0,
            pending: None,
            rgb: Vec::new(),
            dimensions: [0; 2],
            pixel_aspect,
            insets,
            colour,
            hdr,
            codec: if av1 { "AV1" } else { "VP9" },
            start: 0,
            end: 0,
            base: None,
            last_duration: 33_333_333,
            rgb_budget,
            failed: false,
            frames: 0,
        })
    }
    pub fn dimensions(&self) -> [usize; 2] {
        self.dimensions
    }
    pub fn pixel_aspect(&self) -> (u32, u32) {
        self.pixel_aspect
    }
    /// The picture's own crop borders as pixel insets, `[0; 4]` when the track
    /// states none it can keep.
    pub fn insets(&self) -> [u32; 4] {
        self.insets
    }
    pub fn codec(&self) -> &'static str {
        self.codec
    }
    /// The signal the track's `Colour` element states, zeroes where the muxer
    /// wrote none or wrote only some of the three codes.
    pub fn colour(&self) -> ColourDescription {
        self.colour
    }
    /// The mastering display and light level the track names, empty where it
    /// names none. A tone map needs these and no AV1 or VP9 track header beyond
    /// the `Colour` element carries them.
    pub fn hdr(&self) -> HdrMetadata {
        self.hdr
    }
    /// The signal the coding states for itself: an AV1 sequence header's colour
    /// description, which is readable once a header has been decoded. VP9
    /// carries a colour matrix and a range flag in every frame header rather
    /// than an H.273 triple, so it is left to the container.
    pub fn bitstream_colour(&self) -> ColourDescription {
        match &self.decoder {
            VideoDecoder::Av1(d) => d
                .color()
                .map(crate::codec::av1_sequence::Color::signal)
                .unwrap_or_default(),
            VideoDecoder::Vp9(_) => ColourDescription::default(),
        }
    }
    /// The cap this reader sizes its RGB picture by, which is the cap a caller
    /// converting the planes it hands over has to size by too.
    pub fn rgb_budget(&self) -> usize {
        self.rgb_budget
    }
    pub fn rgb(&self) -> &[u8] {
        &self.rgb
    }
    pub fn frame_period(&self) -> Duration {
        Duration::from_nanos((self.end - self.start) as u64)
    }
    pub fn frame_interval(&self) -> Option<(u128, u128, u32)> {
        if self.frames == 0 {
            None
        } else {
            Some((self.start, self.end, 1_000_000_000))
        }
    }
    /// Timeline length relative to the first displayed video frame. When a
    /// streaming file omits Duration, estimate its tail from indexed packet PTS.
    pub fn duration(&self) -> Option<Duration> {
        let mut timestamps = self
            .demux
            .packets
            .iter()
            .filter(|p| p.track == self.track)
            .map(|p| p.pts_ns);
        let first = timestamps.next()?;
        let origin = self.base.unwrap_or(first);
        let end = if let Some(declared) = self.demux.duration_ns {
            i128::from(declared)
        } else {
            let mut last = first;
            let mut interval = 33_333_333i128;
            for pts in timestamps {
                if pts > last {
                    interval = i128::from(pts) - i128::from(last);
                    last = pts;
                }
            }
            i128::from(last) + interval
        };
        let nanos = u64::try_from(end - i128::from(origin)).ok()?;
        (nanos > 0).then(|| Duration::from_nanos(nanos))
    }
    pub fn rewind(&mut self) {
        match &mut self.decoder {
            VideoDecoder::Vp9(d) => d.reset(),
            VideoDecoder::Av1(d) => d.reset(),
        };
        self.index = 0;
        self.pending = None;
        self.base = None;
        self.start = 0;
        self.end = 0;
        self.frames = 0;
        self.failed = false;
        self.last_duration = 33_333_333;
    }
    /// Where the timeline starts for the reader: the first frame once one has
    /// been displayed, and the first packet of the track before that, which is
    /// the same fallback `duration` uses.
    fn origin(&self) -> i64 {
        self.base.unwrap_or_else(|| {
            self.demux
                .packets
                .iter()
                .find(|p| p.track == self.track)
                .map_or(0, |p| p.pts_ns)
        })
    }
    /// Restart decoding at the keyframe that opens the stretch of timeline
    /// containing `target_ns`, counted where `frame_interval` counts, and report
    /// the position landed on. Blocks are indexed in decode order, which a
    /// reordered track need not state in, so the scan takes the latest keyframe
    /// rather than the last one it meets; a target in front of every keyframe
    /// restarts the track at its first block.
    pub fn seek_to_sync(&mut self, target_ns: i64) -> i64 {
        let origin = self.origin();
        let target = target_ns.saturating_add(origin);
        let mut chosen: Option<(usize, i64)> = None;
        for (index, packet) in self.demux.packets.iter().enumerate() {
            if packet.track != self.track || !packet.keyframe || packet.pts_ns > target {
                continue;
            }
            if chosen.is_none_or(|(_, at)| packet.pts_ns >= at) {
                chosen = Some((index, packet.pts_ns));
            }
        }
        let chosen = chosen.or_else(|| {
            let index = self
                .demux
                .packets
                .iter()
                .position(|p| p.track == self.track)?;
            Some((index, self.demux.packets[index].pts_ns))
        });
        let Some((index, pts)) = chosen else {
            return 0;
        };
        match &mut self.decoder {
            VideoDecoder::Vp9(d) => d.reset(),
            VideoDecoder::Av1(d) => d.reset(),
        };
        // The picture held over from before belongs to the stretch being left
        // behind, and a decoder that had just failed is the one rebuilt here.
        self.pending = None;
        self.failed = false;
        self.index = index;
        // Pinning the origin matters only before the first frame: without it the
        // keyframe jumped to would become the start of the timeline and the
        // position would read back as zero.
        self.base = Some(origin);
        let landed = i64::try_from(i128::from(pts) - i128::from(origin))
            .unwrap_or(0)
            .max(0);
        self.start = u128::try_from(landed).unwrap_or_default();
        self.end = self.start;
        landed
    }
    /// Seek and land on the frame covering `target_ns`, refreshing `rgb()` the
    /// way continuous playback does. The pre-roll costs what decoding it costs.
    pub fn seek_to_frame(&mut self, target_ns: i64) -> Result<()> {
        self.seek_to_sync(target_ns);
        let target = u128::try_from(target_ns.max(0)).unwrap_or(u128::MAX);
        let result = (|| {
            while self.read_inner()? {
                if self.end > target {
                    return Ok(());
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn next_decoded(&mut self) -> Result<Option<Frame>> {
        while self.index < self.demux.packets.len() {
            let index = self.index;
            self.index += 1;
            if self.demux.packets[index].track != self.track {
                continue;
            }
            let pts = self.demux.packets[index].pts_ns;
            let packet = self.demux.read_packet(index)?;
            let mut visible = None;
            match &mut self.decoder {
                VideoDecoder::Vp9(d) => {
                    for frame in vp9::frames(&packet)? {
                        let decoded = d.decode(frame)?;
                        if decoded.header.show_frame {
                            if visible.is_some() {
                                return Err(invalid(
                                    "multiple visible frames in one WebM packet need distinct timestamps",
                                ));
                            }
                            visible = Some(Frame {
                                decoded: Picture::Vp9(decoded),
                                pts,
                            });
                        }
                    }
                }
                VideoDecoder::Av1(d) => {
                    for decoded in d.decode_packet(&packet)? {
                        if decoded.show {
                            if visible.is_some() {
                                return Err(invalid(
                                    "multiple visible frames in one WebM packet need distinct timestamps",
                                ));
                            }
                            visible = Some(Frame {
                                decoded: Picture::Av1(decoded),
                                pts,
                            });
                        }
                    }
                }
            }
            if visible.is_some() {
                return Ok(visible);
            }
        }
        Ok(None)
    }
    pub fn read_frame(&mut self) -> Result<bool> {
        if self.failed {
            return Err(invalid("WebM playback requires rewind after an error"));
        }
        let result = self.read_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn read_inner(&mut self) -> Result<bool> {
        let Some(current) = self.advance()? else {
            return Ok(false);
        };
        self.convert_to_rgb(&current)
    }
    /// Advances to the next visible frame and updates timing/geometry state.
    /// The decoded picture is returned for the caller to convert.
    fn advance(&mut self) -> Result<Option<CurrentFrame>> {
        let current = match self.pending.take() {
            Some(frame) => Some(frame),
            None => self.next_decoded()?,
        };
        let Some(current) = current else {
            return Ok(None);
        };
        let next = self.next_decoded()?;
        let duration = if let Some(next) = &next {
            u64::try_from(
                next.pts
                    .checked_sub(current.pts)
                    .filter(|&v| v > 0)
                    .ok_or_else(|| invalid("non-increasing WebM presentation timestamps"))?,
            )
            .map_err(|_| invalid("WebM duration overflow"))?
        } else {
            self.last_duration
        };
        let base = *self.base.get_or_insert(current.pts);
        let start = u128::try_from(i128::from(current.pts) - i128::from(base))
            .map_err(|_| invalid("WebM presentation timestamp precedes origin"))?;
        let (size, depth, full_range, color_space, monochrome) = match &current.decoded {
            Picture::Vp9(d) => (
                d.picture.size,
                d.picture.depth,
                d.header.picture.format.full_range,
                d.header.picture.format.color_space,
                false,
            ),
            Picture::Av1(d) => {
                let color_space = match d.color.matrix {
                    1 => 2,
                    2 | 5 | 6 => 1,
                    7 => 4,
                    9 => 5,
                    _ => return Err(invalid("unsupported AV1 RGB matrix coefficients")),
                };
                (
                    d.picture.size,
                    d.picture.depth,
                    d.color.full_range,
                    color_space,
                    d.color.monochrome,
                )
            }
        };
        let w = size[0] as usize;
        let h = size[1] as usize;
        self.pending = next;
        self.dimensions = [w, h];
        self.start = start;
        self.end = start + u128::from(duration);
        self.last_duration = duration;
        self.frames += 1;
        Ok(Some(CurrentFrame {
            decoded: current.decoded,
            size,
            depth,
            full_range,
            color_space,
            monochrome,
        }))
    }
    fn convert_to_rgb(&mut self, current: &CurrentFrame) -> Result<bool> {
        let (kr, kb) = match current.color_space {
            0 | 1 | 3 => (0.299, 0.114),
            2 => (0.2126, 0.0722),
            4 => (0.212, 0.087),
            5 => (0.2627, 0.0593),
            _ => return Err(invalid("unsupported native RGB colour configuration")),
        };
        let w = current.size[0] as usize;
        let h = current.size[1] as usize;
        let len = w
            .checked_mul(h)
            .and_then(|v| v.checked_mul(3))
            .filter(|&v| v <= self.rgb_budget)
            .ok_or_else(|| invalid("WebM RGB frame exceeds budget"))?;
        if self.rgb.len() != len {
            self.rgb = crate::buffer(len)?;
        }
        let scale = f64::from(1u32 << (current.depth - 8));
        let (offset, yr, cr) = if current.full_range {
            (
                0.0,
                f64::from((1u32 << current.depth) - 1),
                f64::from((1u32 << current.depth) - 1),
            )
        } else {
            (16.0 * scale, 219.0 * scale, 224.0 * scale)
        };
        let planes = match &current.decoded {
            Picture::Vp9(d) => d
                .picture
                .planes
                .each_ref()
                .map(|p| (p.samples.as_slice(), p.width)),
            Picture::Av1(d) => d
                .picture
                .planes
                .each_ref()
                .map(|p| (p.samples.as_slice(), p.width)),
        };
        let [y, u, v] = planes;
        let y_stride = y.1;
        let u_stride = u.1;
        let v_stride = v.1;
        let green = 1.0 - kr - kb;
        for py in 0..h {
            let luma_row = py * y_stride;
            let cb_row = (py / 2) * u_stride;
            let cr_row = (py / 2) * v_stride;
            let line = &mut self.rgb[py * w * 3..][..w * 3];
            for (px, pixel) in line.chunks_exact_mut(3).enumerate() {
                let luma = (f64::from(y.0[luma_row + px]) - offset) / yr;
                let cb = (f64::from(u.0[cb_row + px / 2]) - 128.0 * scale) / cr;
                let cv = (f64::from(v.0[cr_row + px / 2]) - 128.0 * scale) / cr;
                let (cb, cv) = if current.monochrome {
                    (0.0, 0.0)
                } else {
                    (cb, cv)
                };
                let r = luma + 2.0 * (1.0 - kr) * cv;
                let b = luma + 2.0 * (1.0 - kb) * cb;
                let g = (luma - kr * r - kb * b) / green;
                for (out, sample) in pixel.iter_mut().zip([r, g, b]) {
                    *out = (sample * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Ok(true)
    }
    /// Decode the next visible frame and return it as packed 8-bit planes for
    /// GPU conversion (no CPU RGB pass). `rgb()` is not updated by this call.
    pub fn read_frame_planes(&mut self) -> Result<Option<Planar8>> {
        if self.failed {
            return Err(invalid("WebM playback requires rewind after an error"));
        }
        let result = self.read_planes_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn read_planes_inner(&mut self) -> Result<Option<Planar8>> {
        let Some(current) = self.advance()? else {
            return Ok(None);
        };
        let (kr, kb) = match current.color_space {
            0 | 1 | 3 => (0.299, 0.114),
            2 => (0.2126, 0.0722),
            4 => (0.212, 0.087),
            5 => (0.2627, 0.0593),
            _ => return Err(invalid("unsupported native RGB colour configuration")),
        };
        let w = current.size[0] as usize;
        let h = current.size[1] as usize;
        let shift = current.depth.saturating_sub(8);
        let chroma_width = w.div_ceil(2);
        let chroma_height = h.div_ceil(2);
        let chroma_bytes = chroma_width
            .checked_mul(chroma_height)
            .ok_or_else(|| invalid("WebM chroma size overflow"))?;
        let planes_bytes = w
            .checked_mul(h)
            .and_then(|v| v.checked_add(chroma_bytes))
            .and_then(|v| v.checked_add(chroma_bytes))
            .ok_or_else(|| invalid("WebM plane size overflow"))?;
        if planes_bytes > self.rgb_budget {
            return Err(invalid("WebM video planes exceed budget"));
        }
        let narrow = |plane: &[u16], stride: usize, width: usize, height: usize| -> Vec<u8> {
            let mut out = Vec::with_capacity(width * height);
            for row in 0..height {
                out.extend(
                    plane[row * stride..][..width]
                        .iter()
                        .map(|&v| (v >> shift) as u8),
                );
            }
            out
        };
        let planes = match &current.decoded {
            Picture::Vp9(d) => d
                .picture
                .planes
                .each_ref()
                .map(|p| (p.samples.as_slice(), p.width)),
            Picture::Av1(d) => d
                .picture
                .planes
                .each_ref()
                .map(|p| (p.samples.as_slice(), p.width)),
        };
        let [y, u, v] = planes;
        let y_plane = narrow(y.0, y.1, w, h);
        let (cb, cr) = if current.monochrome {
            let fill = if current.full_range { 0u8 } else { 128u8 };
            (
                vec![fill; chroma_width * chroma_height],
                vec![fill; chroma_width * chroma_height],
            )
        } else {
            (
                narrow(u.0, u.1, chroma_width, chroma_height),
                narrow(v.0, v.1, chroma_width, chroma_height),
            )
        };
        Ok(Some(Planar8 {
            width: w,
            height: h,
            chroma_width,
            chroma_height,
            y: y_plane,
            cb,
            cr,
            colour: AvcColour {
                kr,
                kb,
                full: current.full_range,
            },
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    #[test]
    fn av1_video_uses_native_reader_and_rewinds() {
        let data = include_bytes!("../tests/fixtures/av1/random-access.webm");
        let mut reader =
            crate::playback_native::NativeReader::without_memory_limit(Cursor::new(data)).unwrap();
        assert!(reader.duration().is_some());
        let mut first = Vec::new();
        let mut count = 0;
        while reader.read_frame().unwrap() {
            assert_eq!(reader.dimensions(), [192, 128]);
            assert_eq!(reader.rgb().len(), 192 * 128 * 3);
            if count == 0 {
                first = reader.rgb().to_vec();
            }
            count += 1;
        }
        assert_eq!(count, 24);
        assert!(!reader.read_frame().unwrap());
        reader.rewind().unwrap();
        assert!(reader.read_frame().unwrap());
        assert_eq!(reader.rgb(), first);
    }
    #[test]
    fn duration_and_progress_use_video_origin_and_index_fallback() {
        use super::*;
        let input = include_bytes!("../tests/fixtures/vp9/motion.webm");
        let mut reader = WebmVideoReader::open(Cursor::new(input), 16 << 20).unwrap();
        assert_eq!(reader.duration(), Some(Duration::from_secs(1)));
        assert!(reader.read_frame().unwrap());
        assert_eq!(
            reader.frame_interval(),
            Some((0, 100_000_000, 1_000_000_000))
        );
        // Files recorded by streaming writers may omit Segment Duration.
        reader.demux.duration_ns = None;
        assert_eq!(reader.duration(), Some(Duration::from_secs(1)));
        reader.rewind();
        for packet in &mut reader.demux.packets {
            packet.pts_ns += 7_000_000;
        }
        assert!(reader.read_frame().unwrap());
        assert_eq!(reader.duration(), Some(Duration::from_secs(1)));
        reader.demux.duration_ns = Some(1_007_000_000);
        assert_eq!(reader.duration(), Some(Duration::from_secs(1)));
        reader.rewind();
        assert_eq!(reader.duration(), Some(Duration::from_secs(1)));
    }
    #[test]
    fn native_detection_timing_rgb_eof_and_rewind() {
        let input = include_bytes!("../tests/fixtures/vp9/motion.webm");
        let mut reader =
            crate::playback_native::NativeReader::new(Cursor::new(input), 16 << 20).unwrap();
        assert!(reader.duration().is_some());
        let mut first = Vec::new();
        let mut count = 0;
        while reader.read_frame().unwrap() {
            assert_eq!(reader.dimensions(), [128, 96]);
            assert_eq!(reader.rgb().len(), 128 * 96 * 3);
            assert_eq!(
                reader.frame_interval(),
                Some((
                    count * 100_000_000,
                    (count + 1) * 100_000_000,
                    1_000_000_000
                ))
            );
            if count == 0 {
                first = reader.rgb().to_vec();
            }
            count += 1;
        }
        assert_eq!(count, 10);
        assert!(!reader.read_frame().unwrap());
        reader.rewind().unwrap();
        assert!(reader.read_frame().unwrap());
        assert_eq!(reader.rgb(), first);
        assert_eq!(
            reader.frame_interval(),
            Some((0, 100_000_000, 1_000_000_000))
        );
    }

    /// Every frame of a fixture in order: the start of its interval and the RGB
    /// continuous playback shows for it.
    fn shown_frames(input: &'static [u8]) -> Vec<(u128, Vec<u8>)> {
        let mut reader =
            crate::playback_native::NativeReader::without_memory_limit(Cursor::new(input)).unwrap();
        let mut frames = Vec::new();
        while reader.read_frame().unwrap() {
            let (start, _, _) = reader.frame_interval().unwrap();
            frames.push((start, reader.rgb().to_vec()));
        }
        frames
    }

    /// Seeking restarts the decoder at the keyframe before the target and runs
    /// it forward, so the picture that lands on the target has to be the one
    /// continuous playback shows there: a decoder left holding references from
    /// the stretch behind would show a different one. The fixture is 10 frames
    /// at 100 ms across three clusters, so a target inside the second cluster is
    /// reached by decoding from its own keyframe rather than from the start.
    #[test]
    fn a_seek_lands_on_the_frame_continuous_playback_shows() {
        use super::Duration;
        let input = include_bytes!("../tests/fixtures/vp9/motion.webm");
        let forward = shown_frames(input);
        assert_eq!(forward.len(), 10);
        let mut reader =
            crate::playback_native::NativeReader::new(Cursor::new(input), 16 << 20).unwrap();
        assert!(reader.seekable());
        for target in [0u64, 50, 400, 600, 850] {
            reader.seek(Duration::from_millis(target)).unwrap();
            let at = forward
                .iter()
                .position(|(start, _)| *start > u128::from(target) * 1_000_000)
                .unwrap_or(forward.len())
                - 1;
            let (start, _, _) = reader.frame_interval().unwrap();
            assert_eq!(
                (start, reader.rgb()),
                (forward[at].0, forward[at].1.as_slice()),
                "seek to {target} ms"
            );
        }
        // The cursor resumes just after the frame landed on, not where the
        // keyframe behind it started.
        assert!(reader.read_frame().unwrap());
        assert_eq!(
            reader.frame_interval(),
            Some((900_000_000, 1_000_000_000, 1_000_000_000))
        );
        // Past the last frame there is nothing to land on but the last frame.
        reader.seek(Duration::from_secs(9)).unwrap();
        assert_eq!(
            reader.frame_interval(),
            Some((900_000_000, 1_000_000_000, 1_000_000_000))
        );
        assert!(!reader.read_frame().unwrap());
    }

    /// A seek is also how `--start-time` opens a file, before a single frame has
    /// been shown. The keyframe jumped to must not become the start of the
    /// timeline: position still counts from the first frame of the track.
    #[test]
    fn seeking_before_the_first_frame_keeps_the_track_origin() {
        use super::Duration;
        let input = include_bytes!("../tests/fixtures/vp9/motion.webm");
        let mut reader =
            crate::playback_native::NativeReader::new(Cursor::new(input), 16 << 20).unwrap();
        reader.seek(Duration::from_millis(600)).unwrap();
        assert_eq!(
            reader.frame_interval(),
            Some((600_000_000, 700_000_000, 1_000_000_000))
        );
        assert_eq!(reader.duration(), Some(Duration::from_secs(1)));
    }

    /// The fixture is a 16×16 VP9 picture with `PixelCropLeft/Top/Right/Bottom`
    /// of two pixels each, so ffmpeg's own decode of it reports the frame side
    /// data `Frame Cropping: 2/2/2/2`:
    ///
    /// ```text
    /// ffmpeg -f lavfi -i testsrc2=size=16x16:rate=4:duration=1 \
    ///   -c:v libvpx-vp9 -crf 63 -b:v 0 -pix_fmt yuv420p -an \
    ///   -write_crc32 0 tests/fixtures/display/crops.mkv
    /// ```
    ///
    /// No ffmpeg version writes the crop elements themselves, so the file is
    /// that one with four `54 cc/bb/dd/aa 84 00000002` elements inserted at the
    /// end of the `Video` element and the sizes of `Video`, `TrackEntry`,
    /// `Tracks` and `Segment` grown to hold them; `-write_crc32 0` is what
    /// leaves those sizes widen-able in place. The reader's side of the
    /// contract: the crop reaches the player as insets, the coded frame keeps
    /// its full size, and two-pixel-square drawing of a 16×16 coded frame
    /// states square pixels rather than a stretched ones.
    #[test]
    fn a_cropped_webm_reaches_the_player_with_its_borders_and_full_frames() {
        let input = include_bytes!("../tests/fixtures/display/crops.mkv");
        let mut reader =
            crate::playback_native::NativeReader::without_memory_limit(Cursor::new(input)).unwrap();
        assert_eq!(reader.insets(), [2, 2, 2, 2]);
        assert_eq!(reader.pixel_aspect(), (1, 1));
        assert_eq!(reader.video_codec(), "VP9");
        let mut count = 0;
        while reader.read_frame().unwrap() {
            // Decoding is unaffected: the frame arrives coded, borders and all.
            assert_eq!(reader.dimensions(), [16, 16]);
            assert_eq!(reader.rgb().len(), 16 * 16 * 3);
            count += 1;
        }
        assert_eq!(count, 4);
    }

    /// One cluster for the whole file, so every seek decodes the entire
    /// pre-roll. The point is that a rebuilt AV1 decoder still arrives at the
    /// same pictures, wherever in the file it was asked to start.
    #[test]
    fn seeking_a_single_cluster_av1_file_replays_the_same_pictures() {
        use super::Duration;
        let input = include_bytes!("../tests/fixtures/av1/random-access.webm");
        let forward = shown_frames(input);
        assert_eq!(forward.len(), 24);
        let mut reader =
            crate::playback_native::NativeReader::without_memory_limit(Cursor::new(input)).unwrap();
        assert!(reader.seekable());
        for at in [forward.len() / 2, forward.len() - 1] {
            let target = u64::try_from(forward[at].0).unwrap() + 1;
            reader.seek(Duration::from_nanos(target)).unwrap();
            assert_eq!(
                (reader.frame_interval().unwrap().0, reader.rgb()),
                (forward[at].0, forward[at].1.as_slice()),
                "seek to frame {at}"
            );
        }
    }
}

/// Compatibility name retained for callers of the original VP9-only adapter.
pub type WebmVp9Reader<R> = WebmVideoReader<R>;
