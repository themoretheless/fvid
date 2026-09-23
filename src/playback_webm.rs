//! Native WebM/Matroska VP9 and AV1 playback with bounded decode-ahead and source timestamps.
use crate::{
    Result,
    codec::{
        vp9,
        vp9_decoder::{Decoded, Decoder},
    },
    container::webm::{Limits, WebmReader},
    invalid,
    playback_native::{AvcColour, Planar8},
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
}

/// Compatibility name retained for callers of the original VP9-only adapter.
pub type WebmVp9Reader<R> = WebmVideoReader<R>;
