//! Bounded VP9 reference-picture and probability-context lifetime management.
use super::{
    vp9::{Header, HeaderState},
    vp9_picture::{self, Picture},
    vp9_probs::{CompressedHeader, Probabilities},
};
use crate::{Result, invalid};
use std::sync::Arc;
pub struct Decoder {
    headers: HeaderState,
    contexts: [Probabilities; 4],
    references: [Option<Arc<Picture>>; 8],
    previous: Option<Arc<Picture>>,
    previous_shown: bool,
    previous_key: bool,
    budget: usize,
    failed: bool,
}
pub struct Decoded {
    pub header: Header,
    pub picture: Arc<Picture>,
}
impl Decoder {
    pub fn new(budget: usize) -> Self {
        Self {
            headers: Default::default(),
            contexts: std::array::from_fn(|_| Default::default()),
            references: std::array::from_fn(|_| None),
            previous: None,
            previous_shown: false,
            previous_key: false,
            budget,
            failed: false,
        }
    }
    pub fn reset(&mut self) {
        *self = Self::new(self.budget);
    }
    pub fn decode(&mut self, frame: &[u8]) -> Result<Decoded> {
        if self.failed {
            return Err(invalid("VP9 decoder requires reset after a decode error"));
        }
        let result = self.decode_inner(frame);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn decode_inner(&mut self, frame: &[u8]) -> Result<Decoded> {
        let mut headers = self.headers.clone();
        let header = headers.parse(frame)?;
        if let Some(slot) = header.show_existing {
            let picture = self.references[usize::from(slot)]
                .clone()
                .ok_or_else(|| invalid("missing decoded VP9 show-existing reference"))?;
            return Ok(Decoded { header, picture });
        }
        let independent = header.is_intra() || header.error_resilient;
        let mut contexts = self.contexts.clone();
        if independent {
            if header.keyframe || header.error_resilient || header.reset_context == 3 {
                contexts = std::array::from_fn(|_| Default::default());
            } else if header.reset_context == 2 {
                contexts[usize::from(header.signalled_context)] = Default::default();
            }
        }
        let base = if independent {
            Default::default()
        } else {
            contexts[usize::from(header.context_index())].clone()
        };
        let mut compressed = CompressedHeader::parse(frame, &header, &base)?;
        let refs =
            std::array::from_fn(|i| self.references[usize::from(header.references[i])].as_deref());
        let previous = if self.previous_shown {
            self.previous.as_deref()
        } else {
            None
        };
        // At most eight slots, one previous picture and one new picture can be live.
        let (picture, counts) = vp9_picture::decode_frame_counted(
            frame,
            &header,
            &compressed,
            refs,
            previous,
            self.budget / 10,
        )?;
        let picture = Arc::new(picture);
        if !header.parallel && !header.error_resilient {
            compressed.probabilities.adapt(
                &base,
                &counts,
                header.is_intra(),
                self.previous_key,
                header.high_precision_mv,
            );
        }
        if header.refresh_context {
            contexts[usize::from(header.context_index())] = compressed.probabilities;
        }
        self.headers = headers;
        self.contexts = contexts;
        for i in 0..8 {
            if header.refresh_references & (1 << i) != 0 {
                self.references[i] = Some(picture.clone());
            }
        }
        self.previous = Some(picture.clone());
        self.previous_shown = header.show_frame;
        self.previous_key = header.keyframe;
        Ok(Decoded { header, picture })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_and_inter_frames_match_independent_pixel_oracle() {
        let ivf = include_bytes!("../../tests/fixtures/vp9/header.ivf");
        let expected = include_bytes!("../../tests/fixtures/vp9/header-all.yuv");
        let mut at = 32;
        let mut count = 0;
        let mut decoder = Decoder::new(16 << 20);
        while at < ivf.len() {
            let size = u32::from_le_bytes(ivf[at..at + 4].try_into().unwrap()) as usize;
            at += 12;
            let out = decoder.decode(&ivf[at..at + size]).unwrap();
            at += size;
            let pixels: Vec<u8> = out
                .picture
                .planes
                .iter()
                .flat_map(|p| p.samples.iter().map(|&v| v as u8))
                .collect();
            assert_eq!(
                pixels,
                &expected[count * 1536..(count + 1) * 1536],
                "frame {count}"
            );
            count += 1;
        }
        assert_eq!(count, 3);
    }
}

#[cfg(test)]
mod motion_tests {
    use super::*;
    use crate::container::webm::{Limits, WebmReader};
    use std::io::Cursor;
    #[test]
    fn moving_picture_reference_refresh_and_filters_match_every_pixel() {
        let mut reader = WebmReader::open(
            Cursor::new(include_bytes!("../../tests/fixtures/vp9/motion.webm")),
            Limits::default(),
        )
        .unwrap();
        let oracle = include_bytes!("../../tests/fixtures/vp9/motion.yuv");
        let mut decoder = Decoder::new(16 << 20);
        let size = 128 * 96 * 3 / 2;
        for i in 0..reader.packets.len() {
            let frame = reader.read_packet(i).unwrap();
            let out = decoder.decode(&frame).unwrap();
            let pixels: Vec<u8> = out
                .picture
                .planes
                .iter()
                .flat_map(|p| p.samples.iter().map(|&v| v as u8))
                .collect();
            assert_eq!(pixels, &oracle[i * size..(i + 1) * size], "frame {i}");
        }
        assert_eq!(reader.packets.len(), 10);
        assert!(decoder.decode(&[0]).is_err());
        assert!(decoder.decode(&reader.read_packet(0).unwrap()).is_err());
        decoder.reset();
        assert!(decoder.decode(&reader.read_packet(0).unwrap()).is_ok());
    }
}

#[cfg(test)]
mod depth_tests {
    use super::*;
    use crate::container::webm::{Limits, WebmReader};
    use std::io::Cursor;
    #[test]
    fn ten_and_twelve_bit_motion_crops_and_lossless_match_oracles() {
        let cases: [(&[u8], &[u8], [usize; 2], u8); 2] = [
            (
                include_bytes!("../../tests/fixtures/vp9/odd10.webm"),
                include_bytes!("../../tests/fixtures/vp9/odd10.yuv"),
                [70, 50],
                10,
            ),
            (
                include_bytes!("../../tests/fixtures/vp9/lossless12.webm"),
                include_bytes!("../../tests/fixtures/vp9/lossless12.yuv"),
                [48, 32],
                12,
            ),
        ];
        for (input, expected, [w, h], depth) in cases {
            let mut demux = WebmReader::open(Cursor::new(input), Limits::default()).unwrap();
            let mut decoder = Decoder::new(16 << 20);
            let mut output = Vec::new();
            for i in 0..demux.packets.len() {
                let frame = decoder.decode(&demux.read_packet(i).unwrap()).unwrap();
                assert_eq!(frame.picture.depth, depth);
                for (plane, p) in frame.picture.planes.iter().enumerate() {
                    let sub = if plane == 0 { 1 } else { 2 };
                    for y in 0..h.div_ceil(sub) {
                        for x in 0..w.div_ceil(sub) {
                            output.extend_from_slice(&p.samples[y * p.width + x].to_le_bytes());
                        }
                    }
                }
            }
            assert_eq!(output, expected, "{depth}-bit");
            assert_eq!(demux.packets.len(), 3);
        }
    }
}

#[cfg(test)]
mod malformed_tests {
    use super::*;
    #[test]
    fn bounded_truncations_and_mutations_never_publish_a_partial_picture_or_panic() {
        let ivf = include_bytes!("../../tests/fixtures/vp9/header.ivf");
        let len = u32::from_le_bytes(ivf[32..36].try_into().unwrap()) as usize;
        let frame = &ivf[44..44 + len];
        for end in 0..frame.len() {
            let mut decoder = Decoder::new(16 << 20);
            if let Ok(out) = decoder.decode(&frame[..end]) {
                assert_eq!(out.picture.size, [32, 32]);
                assert_eq!(out.picture.planes[0].samples.len(), 1024);
            }
        }
        for i in (0..frame.len()).step_by(5) {
            let mut mutated = frame.to_vec();
            mutated[i] ^= 0xa5;
            let mut decoder = Decoder::new(16 << 20);
            if let Ok(out) = decoder.decode(&mutated) {
                for p in &out.picture.planes {
                    assert_eq!(p.samples.len(), p.width * p.height);
                    assert!(p.samples.iter().all(|&v| v < (1 << out.picture.depth)));
                }
            }
        }
    }
}

#[cfg(test)]
mod adaptation_tests {
    use super::*;
    use crate::container::webm::{Limits, WebmReader};
    use std::io::Cursor;

    #[test]
    fn adaptive_frames_tiles_and_keyframe_resets_match_every_sample() {
        let cases: [(&[u8], &[u8], usize, bool); 3] = [
            (
                include_bytes!("../../tests/fixtures/vp9/adaptive.webm"),
                include_bytes!("../../tests/fixtures/vp9/adaptive.yuv"),
                24,
                false,
            ),
            (
                include_bytes!("../../tests/fixtures/vp9/adaptive10.webm"),
                include_bytes!("../../tests/fixtures/vp9/adaptive10.yuv"),
                12,
                false,
            ),
            (
                include_bytes!("../../tests/fixtures/vp9/adaptive-tiles.webm"),
                include_bytes!("../../tests/fixtures/vp9/adaptive-tiles.yuv"),
                6,
                true,
            ),
        ];
        for (data, oracle, frames, tiled) in cases {
            let mut reader = WebmReader::open(Cursor::new(data), Limits::default()).unwrap();
            let mut decoder = Decoder::new(16 << 20);
            for _ in 0..2 {
                decoder.reset();
                let mut pixels = Vec::new();
                let mut keys = 0;
                let mut refreshed = false;
                for i in 0..reader.packets.len() {
                    let packet = reader.read_packet(i).unwrap();
                    let out = decoder
                        .decode(&packet)
                        .unwrap_or_else(|e| panic!("fixture {frames} frames, packet {i}: {e}"));
                    assert!(!out.header.parallel && !out.header.error_resilient);
                    assert_eq!(out.header.tile_log2[0] > 0, tiled);
                    keys += usize::from(out.header.keyframe);
                    refreshed |= out.header.refresh_context;
                    for (component, plane) in out.picture.planes.iter().enumerate() {
                        let sub = if component == 0 { 1 } else { 2 };
                        let width = out.picture.size[0].div_ceil(sub) as usize;
                        let height = out.picture.size[1].div_ceil(sub) as usize;
                        for row in plane.samples.chunks(plane.width).take(height) {
                            for &v in &row[..width] {
                                if out.picture.depth == 8 {
                                    pixels.push(v as u8);
                                } else {
                                    pixels.extend_from_slice(&v.to_le_bytes());
                                }
                            }
                        }
                    }
                }
                assert_eq!(reader.packets.len(), frames);
                assert!(keys >= 2 && refreshed);
                assert_eq!(pixels.len(), oracle.len());
                assert!(
                    pixels == oracle,
                    "adaptive {frames}-frame fixture differs from oracle"
                );
            }
        }
    }
}
