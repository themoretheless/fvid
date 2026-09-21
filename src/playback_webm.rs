//! Native WebM/Matroska VP9 and AV1 playback with bounded decode-ahead and source timestamps.
use crate::{
    Result,
    codec::{
        vp9,
        vp9_decoder::{Decoded, Decoder},
    },
    container::webm::{Limits, WebmReader},
    invalid,
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
        let current = match self.pending.take() {
            Some(frame) => Some(frame),
            None => self.next_decoded()?,
        };
        let Some(current) = current else {
            return Ok(false);
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
        let (size, depth, planes, full_range, color_space, monochrome) = match &current.decoded {
            Picture::Vp9(d) => (
                d.picture.size,
                d.picture.depth,
                d.picture
                    .planes
                    .each_ref()
                    .map(|p| (p.samples.as_slice(), p.width)),
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
                    d.picture
                        .planes
                        .each_ref()
                        .map(|p| (p.samples.as_slice(), p.width)),
                    d.color.full_range,
                    color_space,
                    d.color.monochrome,
                )
            }
        };
        let (kr, kb) = match color_space {
            0 | 1 | 3 => (0.299, 0.114),
            2 => (0.2126, 0.0722),
            4 => (0.212, 0.087),
            5 => (0.2627, 0.0593),
            _ => return Err(invalid("unsupported native RGB colour configuration")),
        };
        let w = size[0] as usize;
        let h = size[1] as usize;
        let len = w
            .checked_mul(h)
            .and_then(|v| v.checked_mul(3))
            .filter(|&v| v <= self.rgb_budget)
            .ok_or_else(|| invalid("WebM RGB frame exceeds budget"))?;
        if self.rgb.len() != len {
            self.rgb = crate::buffer(len)?;
        }
        let scale = f64::from(1u32 << (depth - 8));
        let (offset, yr, cr) = if full_range {
            (
                0.0,
                f64::from((1u32 << depth) - 1),
                f64::from((1u32 << depth) - 1),
            )
        } else {
            (16.0 * scale, 219.0 * scale, 224.0 * scale)
        };
        let [y, u, v] = planes;
        for (i, pixel) in self.rgb.chunks_exact_mut(3).enumerate() {
            let px = i % w;
            let py = i / w;
            let luma = (f64::from(y.0[py * y.1 + px]) - offset) / yr;
            let cb = (f64::from(u.0[(py / 2) * u.1 + px / 2]) - 128.0 * scale) / cr;
            let cv = (f64::from(v.0[(py / 2) * v.1 + px / 2]) - 128.0 * scale) / cr;
            let (cb, cv) = if monochrome { (0.0, 0.0) } else { (cb, cv) };
            let r = luma + 2.0 * (1.0 - kr) * cv;
            let b = luma + 2.0 * (1.0 - kb) * cb;
            let g = (luma - kr * r - kb * b) / (1.0 - kr - kb);
            for (out, sample) in pixel.iter_mut().zip([r, g, b]) {
                *out = (sample * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        self.pending = next;
        self.dimensions = [w, h];
        self.start = start;
        self.end = start + u128::from(duration);
        self.last_duration = duration;
        self.frames += 1;
        Ok(true)
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
    fn native_detection_timing_rgb_eof_and_rewind() {
        let input = include_bytes!("../tests/fixtures/vp9/motion.webm");
        let mut reader =
            crate::playback_native::NativeReader::new(Cursor::new(input), 16 << 20).unwrap();
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
