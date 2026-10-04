//! Owned luma-dependent chroma correction with frame-wide white balance analysis.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug, Default)]
enum Analyze {
    #[default]
    Manual,
    Average,
    MinMax,
    Median,
}
#[derive(Clone, Copy, Debug)]
pub struct ColorCorrect {
    spots: [f32; 4],
    saturation: f32,
    analyze: Analyze,
}
impl ColorCorrect {
    pub fn parse(args: &str) -> Result<Self> {
        let names = ["rl", "bl", "rh", "bh", "saturation", "analyze"];
        let mut filter = Self {
            spots: [0.0; 4],
            saturation: 1.0,
            analyze: Analyze::Manual,
        };
        let mut position = 0;
        for part in args.split(':').filter(|_| !args.is_empty()) {
            let (name, value) = if let Some(pair) = part.split_once('=') {
                pair
            } else {
                let name = *names.get(position).ok_or("too many colorcorrect options")?;
                position += 1;
                (name, part)
            };
            let index = names
                .iter()
                .position(|n| *n == name.trim())
                .ok_or("unknown colorcorrect option")?;
            if index == 5 {
                filter.analyze = match value.trim() {
                    "0" | "manual" => Analyze::Manual,
                    "1" | "average" => Analyze::Average,
                    "2" | "minmax" => Analyze::MinMax,
                    "3" | "median" => Analyze::Median,
                    _ => return Err("invalid colorcorrect analysis".into()),
                };
            } else {
                let value = crate::owned_expression::constant(value.trim())? as f32;
                let bound = if index == 4 { 3.0 } else { 1.0 };
                if !value.is_finite() || !(-bound..=bound).contains(&value) {
                    return Err("invalid colorcorrect parameter".into());
                }
                if index == 4 {
                    filter.saturation = value;
                } else {
                    filter.spots[index] = value;
                }
            }
        }
        Ok(filter)
    }
    pub fn apply(&self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        let [sx, sy] = frame
            .subsampling
            .ok_or("colorcorrect requires planar YUV")?;
        if !(8..=16).contains(&depth) || sx == 0 || sy == 0 || frame.width == 0 || frame.height == 0
        {
            return Err("invalid colorcorrect geometry or depth".into());
        }
        let y = frame
            .width
            .checked_mul(frame.height)
            .ok_or("colorcorrect geometry overflow")?;
        let cw = frame.width.div_ceil(sx);
        let ch = frame.height.div_ceil(sy);
        let c = cw.checked_mul(ch).ok_or("colorcorrect geometry overflow")?;
        let bytes = if depth == 8 { 1 } else { 2 };
        let expected = c
            .checked_mul(2)
            .and_then(|n| n.checked_add(y))
            .and_then(|n| n.checked_mul(bytes))
            .ok_or("colorcorrect geometry overflow")?;
        if expected != frame.data.len() {
            return Err("colorcorrect frame length mismatch".into());
        }
        let maximum = (1u32 << depth) - 1;
        if bytes == 2
            && frame
                .data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as u32 > maximum)
        {
            return Err("colorcorrect sample exceeds precision".into());
        }
        let imax = 1.0 / maximum as f32;
        let read = |index: usize| -> u16 {
            if bytes == 1 {
                frame.data[index] as u16
            } else {
                u16::from_le_bytes([frame.data[2 * index], frame.data[2 * index + 1]])
            }
        };
        let mut spots = self.spots;
        if !matches!(self.analyze, Analyze::Manual) {
            for (plane, low, high) in [(0, 1, 3), (1, 0, 2)] {
                let start = y + plane * c;
                let (lo, hi) = match self.analyze {
                    Analyze::Average => {
                        let sum = (0..c).map(|i| read(start + i) as u128).sum::<u128>();
                        let v = imax * sum as f32 / c as f32 - 0.5;
                        (v, v)
                    }
                    Analyze::MinMax => {
                        let mut lo = maximum as u16;
                        let mut hi = 0;
                        for i in 0..c {
                            let v = read(start + i);
                            lo = lo.min(v);
                            hi = hi.max(v);
                        }
                        (
                            (lo as f32).mul_add(imax, -0.5),
                            (hi as f32).mul_add(imax, -0.5),
                        )
                    }
                    Analyze::Median => {
                        let mut histogram = Vec::<usize>::new();
                        histogram
                            .try_reserve_exact(maximum as usize + 1)
                            .map_err(|e| e.to_string())?;
                        histogram.resize(maximum as usize + 1, 0);
                        for i in 0..c {
                            histogram[read(start + i) as usize] += 1;
                        }
                        let mut count = 0;
                        let mut median = maximum as usize;
                        for (value, n) in histogram.into_iter().enumerate() {
                            count += n;
                            if count >= c / 2 {
                                median = value;
                                break;
                            }
                        }
                        let v = (median as f32).mul_add(imax, -0.5);
                        (v, v)
                    }
                    Analyze::Manual => unreachable!(),
                };
                spots[low] = -lo;
                spots[high] = -hi;
            }
        }
        let [rl, bl, rh, bh] = spots;
        for row in 0..ch {
            for column in 0..cw {
                let i = row * cw + column;
                let yi = (row * sy) * frame.width + column * sx;
                let luma = if bytes == 1 {
                    frame.data[yi] as f32
                } else {
                    u16::from_le_bytes([frame.data[2 * yi], frame.data[2 * yi + 1]]) as f32
                } * imax;
                for (index, low, high) in [(y + i, bl, bh), (y + c + i, rl, rh)] {
                    let raw = if bytes == 1 {
                        frame.data[index] as f32
                    } else {
                        u16::from_le_bytes([frame.data[2 * index], frame.data[2 * index + 1]])
                            as f32
                    };
                    let centered = raw.mul_add(imax, -0.5);
                    let adjusted = self.saturation * (luma.mul_add(high - low, centered) + low);
                    let value =
                        ((adjusted + 0.5) * maximum as f32).clamp(0.0, maximum as f32) as u16;
                    if bytes == 1 {
                        frame.data[index] = value as u8;
                    } else {
                        frame.data[2 * index..2 * index + 2].copy_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn odd_chroma_cells_and_precision_validation_cover_all_integer_depths() {
        for depth in 8..=16 {
            let maximum = (1u32 << depth) - 1;
            for subsampling in [[1, 1], [2, 1], [1, 2], [2, 2], [4, 1]] {
                let c = 3usize.div_ceil(subsampling[0]) * 5usize.div_ceil(subsampling[1]);
                let samples: Vec<u16> = (0..15 + 2 * c)
                    .map(|i| ((i as u32 * maximum) / ((15 + 2 * c) as u32)) as u16)
                    .collect();
                let data = if depth == 8 {
                    samples.iter().map(|v| *v as u8).collect()
                } else {
                    samples.iter().flat_map(|v| v.to_le_bytes()).collect()
                };
                let mut frame = GeometryFrame {
                    width: 3,
                    height: 5,
                    subsampling: Some(subsampling),
                    data,
                };
                let bytes = if depth == 8 { 1 } else { 2 };
                let luma = frame.data[..15 * bytes].to_vec();
                ColorCorrect::parse("analyze=median:saturation=0")
                    .unwrap()
                    .apply(&mut frame, depth)
                    .unwrap();
                assert_eq!(&frame.data[..15 * bytes], &luma);
                if depth == 8 {
                    assert!(frame.data[15..].iter().all(|v| *v == 127));
                } else {
                    assert!(
                        frame.data[30..]
                            .chunks_exact(2)
                            .all(|v| u16::from_le_bytes([v[0], v[1]]) as u32 == maximum / 2)
                    );
                }
            }
        }
        let mut invalid = GeometryFrame {
            width: 1,
            height: 1,
            subsampling: Some([1, 1]),
            data: vec![0, 0, 0, 0, 255, 255],
        };
        let before = invalid.data.clone();
        assert!(
            ColorCorrect::parse("analyze=median")
                .unwrap()
                .apply(&mut invalid, 12)
                .is_err()
        );
        assert_eq!(invalid.data, before);
        invalid.width = usize::MAX;
        invalid.height = 2;
        assert!(
            ColorCorrect::parse("")
                .unwrap()
                .apply(&mut invalid, 16)
                .is_err()
        );
        assert_eq!(invalid.data, before);
    }
    #[test]
    fn neutral_saturation_preserves_luma_and_validation_is_atomic() {
        let mut frame = GeometryFrame {
            width: 3,
            height: 3,
            subsampling: Some([2, 2]),
            data: (0..17).map(|n| n * 13).collect(),
        };
        let luma = frame.data[..9].to_vec();
        ColorCorrect::parse("saturation=0")
            .unwrap()
            .apply(&mut frame, 8)
            .unwrap();
        assert_eq!(&frame.data[..9], &luma);
        assert!(frame.data[9..].iter().all(|v| *v == 127));
        frame.data.push(0);
        let before = frame.data.clone();
        assert!(
            ColorCorrect::parse("")
                .unwrap()
                .apply(&mut frame, 8)
                .is_err()
        );
        assert_eq!(frame.data, before);
        for args in ["rl=2", "saturation=4", "analyze=unknown", "bl=nan"] {
            assert!(ColorCorrect::parse(args).is_err());
        }
    }
}

#[cfg(test)]
mod reference_acceptance {
    #[test]
    fn synthetic_video_all_analysis_modes_match_saved_single_thread_reference() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for depth in [8, 12, 16] {
            let input = std::fs::read(root.join(format!("colorcorrect-grid-{depth}.y4m"))).unwrap();
            for (case, args) in [
                "",
                "rl=0.2:bl=-0.3:rh=-0.1:bh=0.4",
                "saturation=-2",
                "saturation=0",
                "analyze=average",
                "analyze=minmax:saturation=0.7",
                "analyze=median",
                "rl=1:bl=-1:rh=-1:bh=1:saturation=3",
            ]
            .into_iter()
            .enumerate()
            {
                let request = fvid_media_info::DecodeTransform {
                    colorcorrect: Some(args.into()),
                    ..Default::default()
                };
                assert!(crate::owned_y4m_decode::supported_request(&request));
                let mut actual = Vec::new();
                let mut frames = 0;
                crate::owned_y4m_decode::visit_reader_transformed(
                    std::io::Cursor::new(&input),
                    &request,
                    |_, frame, _, _| {
                        frames += 1;
                        actual.extend_from_slice(frame);
                        Ok(())
                    },
                )
                .unwrap();
                assert_eq!(frames, 3);
                let expected =
                    std::fs::read(root.join(format!("colorcorrect-reference-{depth}-{case}.raw")))
                        .unwrap();
                assert_eq!(actual, expected, "depth={depth} args={args}");
            }
        }
    }
    #[test]
    fn zero_adjustment_chroma_rounding_regression() {
        let input = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/playback-errors/colorcorrect-grid-8.y4m"),
        )
        .unwrap();
        let request = fvid_media_info::DecodeTransform {
            colorcorrect: Some(String::new()),
            ..Default::default()
        };
        let mut frames = Vec::new();
        crate::owned_y4m_decode::visit_reader_transformed(
            std::io::Cursor::new(input),
            &request,
            |_, frame, _, _| {
                frames.push(frame.to_vec());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(frames[0][26], 46);
        assert_eq!(frames[0][38], 47);
    }
}
