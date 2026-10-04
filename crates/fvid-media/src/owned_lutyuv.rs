//! Owned sample lookup tables for planar YUV, with explicit colour range.
use crate::{owned_expression::Expression, owned_frame::GeometryFrame};
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug)]
pub struct LutYuv {
    expressions: [Expression; 3],
    cache: std::sync::Arc<std::sync::Mutex<Option<Tables>>>,
}
#[derive(Debug)]
struct Tables {
    key: (usize, usize, u8, bool),
    planes: Vec<Vec<u16>>,
}
impl LutYuv {
    pub fn parse(args: &str) -> Result<Self> {
        let mut expressions = std::array::from_fn(|_| Expression::parse_lut("clipval").unwrap());
        let mut positional = 0;
        for option in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = match option.split_once('=') {
                Some(pair) => pair,
                None => {
                    let names = ["y", "u", "v"];
                    let key = *names.get(positional).ok_or("too many lutyuv components")?;
                    positional += 1;
                    (key, option)
                }
            };
            let index = match key.trim() {
                "y" | "c0" => 0,
                "u" | "c1" => 1,
                "v" | "c2" => 2,
                _ => return Err("unsupported lutyuv component".into()),
            };
            let expression = Expression::parse_lut(value.trim())?;
            expression.evaluate(&[
                ("w", 1.0),
                ("h", 1.0),
                ("val", 128.0),
                ("minval", 16.0),
                ("maxval", 235.0),
                ("negval", 123.0),
                ("clipval", 128.0),
            ])?;
            expressions[index] = expression;
        }
        Ok(Self {
            expressions,
            cache: Default::default(),
        })
    }
    pub fn apply(&self, frame: &mut GeometryFrame, depth: u8, full_range: bool) -> Result<()> {
        let [sx, sy] = frame.subsampling.ok_or("lutyuv requires planar YUV")?;
        if !(8..=16).contains(&depth) || sx == 0 || sy == 0 || frame.width == 0 || frame.height == 0
        {
            return Err("invalid lutyuv geometry or depth".into());
        }
        let y = frame
            .width
            .checked_mul(frame.height)
            .ok_or("lutyuv geometry overflow")?;
        let c = frame
            .width
            .div_ceil(sx)
            .checked_mul(frame.height.div_ceil(sy))
            .ok_or("lutyuv geometry overflow")?;
        let bytes = if depth == 8 { 1 } else { 2 };
        let count = c
            .checked_mul(2)
            .and_then(|n| n.checked_add(y))
            .ok_or("lutyuv geometry overflow")?;
        if count.checked_mul(bytes) != Some(frame.data.len()) {
            return Err("lutyuv frame length mismatch".into());
        }
        let maximum = (1u32 << depth) - 1;
        if bytes == 2
            && frame
                .data
                .chunks_exact(2)
                .any(|b| u16::from_le_bytes([b[0], b[1]]) as u32 > maximum)
        {
            return Err("lutyuv sample exceeds precision".into());
        }
        let key = (frame.width, frame.height, depth, full_range);
        let mut cache = self.cache.lock().map_err(|_| "LUT cache lock poisoned")?;
        if cache.as_ref().is_none_or(|entry| entry.key != key) {
            // Build all planes before publishing the entry or touching samples.

            let mut tables = Vec::with_capacity(3);
            for component in 0..3 {
                let minimum = if full_range { 0 } else { 16u32 << (depth - 8) };
                let upper = if full_range {
                    maximum
                } else {
                    (if component == 0 { 235u32 } else { 240u32 }) << (depth - 8)
                };
                let mut table = Vec::with_capacity(maximum as usize + 1);
                for val in 0..=maximum {
                    let result = self.expressions[component].evaluate(&[
                        ("w", frame.width as f64),
                        ("h", frame.height as f64),
                        ("val", val as f64),
                        ("minval", minimum as f64),
                        ("maxval", upper as f64),
                        (
                            "negval",
                            ((minimum + upper) as f64 - val as f64)
                                .clamp(minimum as f64, upper as f64),
                        ),
                        ("clipval", val.clamp(minimum, upper) as f64),
                    ])?;
                    if !result.is_finite() {
                        return Err("nonfinite lutyuv expression result".into());
                    }
                    table.push(result.clamp(0.0, maximum as f64) as u16);
                }
                tables.push(table);
            }
            *cache = Some(Tables {
                key,
                planes: tables,
            });
        }
        let tables = &cache.as_ref().unwrap().planes;
        let mut offset = 0;
        for (component, length) in [y, c, c].into_iter().enumerate() {
            for index in offset..offset + length {
                if bytes == 1 {
                    frame.data[index] = tables[component][frame.data[index] as usize] as u8;
                } else {
                    let at = index * 2;
                    let val = u16::from_le_bytes([frame.data[at], frame.data[at + 1]]);
                    frame.data[at..at + 2]
                        .copy_from_slice(&tables[component][val as usize].to_le_bytes());
                }
            }
            offset += length;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_precision_and_odd_chroma() {
        for depth in [8, 12, 16] {
            let max = (1u32 << depth) - 1;
            let data: Vec<u8> = (0..17)
                .flat_map(|_| {
                    if depth == 8 {
                        vec![max as u8]
                    } else {
                        (max as u16).to_le_bytes().to_vec()
                    }
                })
                .collect();
            let mut frame = GeometryFrame {
                width: 3,
                height: 3,
                subsampling: Some([2, 2]),
                data,
            };
            LutYuv::parse("")
                .unwrap()
                .apply(&mut frame, depth, false)
                .unwrap();
            let values: Vec<u16> = if depth == 8 {
                frame.data.iter().map(|v| *v as u16).collect()
            } else {
                frame
                    .data
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect()
            };
            assert_eq!(values[..9], [235u16 << (depth - 8); 9]);
            assert_eq!(values[9..], [240u16 << (depth - 8); 8]);
            LutYuv::parse("y=val*0+maxval:u=val*0:v=val*0+maxval")
                .unwrap()
                .apply(&mut frame, depth, true)
                .unwrap();
            let sample = |i| {
                if depth == 8 {
                    frame.data[i] as u16
                } else {
                    u16::from_le_bytes([frame.data[i * 2], frame.data[i * 2 + 1]])
                }
            };
            assert_eq!(
                (sample(0), sample(9), sample(13)),
                (max as u16, 0, max as u16)
            );
        }
    }
    #[test]
    fn failure_does_not_mutate_samples() {
        let mut frame = GeometryFrame {
            width: 1,
            height: 1,
            subsampling: Some([1, 1]),
            data: vec![128; 3],
        };
        let original = frame.data.clone();
        let filter = LutYuv::parse("y=1/(val-1)").unwrap();
        assert!(filter.apply(&mut frame, 8, false).is_err());
        assert_eq!(frame.data, original);
        for args in ["y=unknown", "a=val", "y=clip(val,0)", "y=random(0)"] {
            assert!(LutYuv::parse(args).is_err());
        }
    }
}

#[cfg(test)]
mod export_tests {
    #[test]
    fn synthetic_y4m_and_ffv1_lut_acceptance() {
        let directory = std::env::temp_dir().join(format!("fvid-lutyuv-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "../../tests/fixtures/playback-errors/colorize-grid-{depth}.y4m"
            ));
            let raw = directory.join(format!("raw-{depth}.mkv"));
            crate::owned_lossless::transcode_lossless(
                &source,
                &raw,
                Default::default(),
                &Default::default(),
            )
            .unwrap();
            for (kind, input) in [("y4m", &source), ("ffv1", &raw)] {
                let output = directory.join(format!("{kind}-{depth}.mkv"));
                let transform = fvid_media_info::LosslessTransform {
                    lutyuv: Some("y=minval:u=maxval:v=minval".into()),
                    ..Default::default()
                };
                assert!(crate::owned_lossless::supports(
                    input,
                    &transform,
                    &Default::default()
                ));
                assert_eq!(
                    crate::owned_lossless::transcode_lossless(
                        input,
                        &output,
                        transform,
                        &Default::default()
                    )
                    .unwrap()
                    .video_frames,
                    3
                );
                let mut frames = 0;
                crate::owned_video_decode::decode_ffv1(
                    &output,
                    &Default::default(),
                    Some(&mut |frame| {
                        let samples: Vec<u16> = if depth == 8 {
                            frame.pixels.iter().map(|s| *s as u16).collect()
                        } else {
                            frame
                                .pixels
                                .chunks_exact(2)
                                .map(|s| u16::from_le_bytes([s[0], s[1]]))
                                .collect()
                        };
                        assert_eq!(&samples[..9], &[16u16 << (depth - 8); 9]);
                        assert_eq!(&samples[9..13], &[240u16 << (depth - 8); 4]);
                        assert_eq!(&samples[13..], &[16u16 << (depth - 8); 4]);
                        assert_eq!(frame.pts_ns, frames * 500_000_000);
                        frames += 1;
                        Ok(())
                    }),
                    None,
                )
                .unwrap()
                .unwrap();
                assert_eq!(frames, 3);
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn reuse_and_geometry_range_precision_changes_do_not_leave_stale_tables() {
        let filter = LutYuv::parse("y=w+h:u=maxval:v=clipval").unwrap();
        let mut frame = GeometryFrame {
            width: 1,
            height: 1,
            subsampling: Some([1, 1]),
            data: vec![128; 3],
        };
        filter.apply(&mut frame, 8, false).unwrap();
        assert_eq!(frame.data, [2, 240, 128]);
        let allocation = filter.cache.lock().unwrap().as_ref().unwrap().planes[0].as_ptr();
        frame.data.fill(128);
        filter.clone().apply(&mut frame, 8, false).unwrap();
        assert_eq!(
            filter.cache.lock().unwrap().as_ref().unwrap().planes[0].as_ptr(),
            allocation
        );
        filter.apply(&mut frame, 8, true).unwrap();
        assert_eq!(frame.data, [2, 255, 128]);
        frame.width = 2;
        frame.data = vec![128; 6];
        filter.apply(&mut frame, 8, true).unwrap();
        assert_eq!(frame.data, [3, 3, 255, 255, 128, 128]);
        frame.data = [128u16; 6].into_iter().flat_map(u16::to_le_bytes).collect();
        filter.apply(&mut frame, 12, false).unwrap();
        let samples: Vec<u16> = frame
            .data
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(samples, [3, 3, 3840, 3840, 256, 256]);
    }
}
