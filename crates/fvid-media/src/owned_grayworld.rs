//! Owned white balance in logarithmic LMS opponent coordinates.
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug, Default)]
pub struct GrayWorld;
impl GrayWorld {
    pub fn parse(args: &str) -> Result<Self> {
        match args.trim(){""|"0"=>Ok(Self),_=>Err("grayworld takes empty or 0 options; timeline expressions require timeline processing".into())}
    }
    /// Packed linear-light float RGB/RGBA. Alpha and output headroom are preserved.
    pub fn apply_rgb_f32(
        &self,
        data: &mut [f32],
        width: usize,
        height: usize,
        channels: usize,
    ) -> Result<()> {
        let pixels = width
            .checked_mul(height)
            .ok_or("grayworld geometry overflow")?;
        if width == 0
            || height == 0
            || !matches!(channels, 3 | 4)
            || pixels.checked_mul(channels) != Some(data.len())
            || data.iter().any(|v| !v.is_finite())
        {
            return Err("invalid grayworld float RGB frame".into());
        }
        let length = pixels
            .checked_mul(3)
            .ok_or("grayworld workspace overflow")?;
        let mut lab = Vec::new();
        lab.try_reserve_exact(length)
            .map_err(|_| "grayworld workspace allocation failed")?;
        let mut total = [0.0f32; 2];
        for row in data.chunks_exact(width * channels) {
            let mut row_sum = [0.0f32; 2];
            for pixel in row.chunks_exact(channels) {
                let lms = matrix(
                    [
                        [0.3811, 0.5783, 0.0402],
                        [0.1967, 0.7244, 0.0782],
                        [0.0241, 0.1288, 0.8444],
                    ],
                    [pixel[0], pixel[1], pixel[2]],
                )
                .map(|v| if v > 0.0 { v.ln() } else { -1024.0 });
                let coordinates = matrix(
                    [
                        [0.5774, 0.5774, 0.5774],
                        [0.40825, 0.40825, -0.816458],
                        [0.707, -0.707, 0.0],
                    ],
                    lms,
                );
                row_sum[0] += coordinates[1];
                row_sum[1] += coordinates[2];
                lab.extend(coordinates);
            }
            total[0] += row_sum[0];
            total[1] += row_sum[1];
        }
        let mean = total.map(|v| v / pixels as f32);
        for coordinate in lab.chunks_exact_mut(3) {
            let balanced = [
                coordinate[0],
                coordinate[1] - mean[0],
                coordinate[2] - mean[1],
            ];
            let linear = matrix(
                [
                    [0.57735, 0.40825, 0.707],
                    [0.57735, 0.40825, -0.707],
                    [0.57735, -0.8165, 0.0],
                ],
                balanced,
            )
            .map(f32::exp);
            coordinate.copy_from_slice(&matrix(
                [
                    [4.4679, -3.5873, 0.1193],
                    [-1.2186, 2.3809, -0.1624],
                    [0.0497, -0.2439, 1.2045],
                ],
                linear,
            ));
        }
        if lab.iter().any(|v| !v.is_finite()) {
            return Err("grayworld produced nonfinite RGB".into());
        }
        for (pixel, rgb) in data.chunks_exact_mut(channels).zip(lab.chunks_exact(3)) {
            pixel[..3].copy_from_slice(rgb);
        }
        Ok(())
    }
    pub fn apply_rgb(
        &self,
        data: &mut [u8],
        width: usize,
        height: usize,
        depth: u8,
        channels: usize,
    ) -> Result<()> {
        if !(8..=16).contains(&depth) || !matches!(channels, 3 | 4) {
            return Err("invalid grayworld RGB format".into());
        }
        let count = width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(channels))
            .ok_or("grayworld geometry overflow")?;
        let bytes = if depth == 8 { 1 } else { 2 };
        let maximum = ((1u32 << depth) - 1) as f32;
        if count.checked_mul(bytes) != Some(data.len()) {
            return Err("grayworld RGB length mismatch".into());
        }
        if bytes == 2
            && data
                .chunks_exact(2)
                .any(|v| u16::from_le_bytes([v[0], v[1]]) as f32 > maximum)
        {
            return Err("grayworld sample exceeds precision".into());
        }
        let mut rgb = Vec::new();
        rgb.try_reserve_exact(count)
            .map_err(|_| "grayworld RGB allocation failed")?;
        for sample in data.chunks_exact(bytes) {
            rgb.push(if bytes == 1 {
                sample[0] as f32 / maximum
            } else {
                u16::from_le_bytes([sample[0], sample[1]]) as f32 / maximum
            });
        }
        self.apply_rgb_f32(&mut rgb, width, height, channels)?;
        for (out, pixel) in data
            .chunks_exact_mut(bytes * channels)
            .zip(rgb.chunks_exact(channels))
        {
            for channel in 0..3 {
                let value = (pixel[channel] * maximum)
                    .round_ties_even()
                    .clamp(0.0, maximum) as u16;
                if bytes == 1 {
                    out[channel] = value as u8;
                } else {
                    out[2 * channel..2 * channel + 2].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        Ok(())
    }
    pub fn apply_yuv(
        &self,
        frame: &mut crate::owned_frame::GeometryFrame,
        depth: u8,
        full: bool,
        matrix: crate::owned_yuv_rgb::Matrix,
    ) -> Result<()> {
        let width = frame.width;
        let height = frame.height;
        crate::owned_yuv_rgb::filter_rgb_f32_frame(frame, depth, full, matrix, |rgb| {
            // Integer YUV enters this logarithmic filter through bounded RGB.
            for value in rgb.iter_mut() {
                *value = value.clamp(0.0, 1.0);
            }
            self.apply_rgb_f32(rgb, width, height, 3)
        })
    }
}
fn matrix(rows: [[f32; 3]; 3], input: [f32; 3]) -> [f32; 3] {
    rows.map(|row| row[2].mul_add(input[2], row[1].mul_add(input[1], row[0] * input[0])))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn black_alpha_and_invalid_input_are_atomic() {
        let mut rgba = [0.0, 0.0, 0.0, 0.7];
        GrayWorld.apply_rgb_f32(&mut rgba, 1, 1, 4).unwrap();
        assert_eq!(rgba, [0.0, 0.0, 0.0, 0.7]);
        let mut bad = [1.0, f32::INFINITY, 0.0, 0.5];
        let before = bad;
        assert!(GrayWorld.apply_rgb_f32(&mut bad, 1, 1, 4).is_err());
        assert_eq!(bad, before);
        let mut bad = [1, 0, 2, 0, 255, 255];
        let before = bad;
        assert!(GrayWorld.apply_rgb(&mut bad, 1, 1, 12, 3).is_err());
        assert_eq!(bad, before);
        assert!(GrayWorld::parse("0").is_ok());
        assert!(GrayWorld::parse("unknown=1").is_err());
    }
}

#[cfg(test)]
mod reference_acceptance {
    #[test]
    fn synthetic_float_video_matches_saved_reference_with_bounded_rounding() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        let input = std::fs::read(root.join("grayworld-grid.rgba_f32")).unwrap();
        let expected = std::fs::read(root.join("grayworld-reference.rgba_f32")).unwrap();
        assert_eq!(input.len(), expected.len());
        for (source, reference) in input
            .chunks_exact(8 * 8 * 4 * 4)
            .zip(expected.chunks_exact(8 * 8 * 4 * 4))
        {
            let mut actual: Vec<f32> = source
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            let original = actual.clone();
            super::GrayWorld
                .apply_rgb_f32(&mut actual, 8, 8, 4)
                .unwrap();
            let expected: Vec<f32> = reference
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            for (i, (a, b)) in actual.iter().zip(&expected).enumerate() {
                assert!(
                    (a - b).abs() <= 8.0 * f32::EPSILON * b.abs().max(1.0),
                    "component={i} actual={a} expected={b}"
                );
                if i % 4 == 3 {
                    assert_eq!(*a, original[i]);
                    assert_eq!(*b, original[i]);
                }
            }
        }
    }
    #[test]
    fn native_yuv_filter_visits_all_frames_without_legacy() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for depth in [8, 12, 16] {
            let input = std::fs::read(root.join(format!("grayworld-gamut-{depth}.y4m"))).unwrap();
            let request = fvid_media_info::DecodeTransform {
                grayworld: Some("0".into()),
                ..Default::default()
            };
            assert!(crate::owned_y4m_decode::supported_request(&request));
            let mut frames = 0;
            crate::owned_y4m_decode::visit_reader_transformed(
                std::io::Cursor::new(input),
                &request,
                |_, _, _, _| {
                    frames += 1;
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(frames, 3);
        }
    }
}

#[cfg(test)]
mod timeline_qualification {
    fn fixture() -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/playback-errors/grayworld-gamut-8.y4m"),
        )
        .unwrap()
    }
    #[test]
    fn timeline_is_refused_without_claiming_native_acceptance() {
        let request = fvid_media_info::DecodeTransform {
            grayworld: Some("enable=lt(n,1)".into()),
            ..Default::default()
        };
        assert!(!crate::owned_y4m_decode::supported_request(&request));
        let error = crate::owned_y4m_decode::visit_reader_transformed(
            std::io::Cursor::new(fixture()),
            &request,
            |_, _, _, _| Ok(()),
        )
        .unwrap_err();
        assert_eq!(
            error,
            "owned Y4M decoder does not yet implement requested transform options"
        );
    }
    #[test]
    #[ignore = "acceptance pending owned generic filter timeline evaluation"]
    fn timeline_enable_changes_only_selected_frames() {
        let source = fixture();
        let collect = |request: &fvid_media_info::DecodeTransform| {
            let mut frames = Vec::new();
            crate::owned_y4m_decode::visit_reader_transformed(
                std::io::Cursor::new(&source),
                request,
                |_, frame, _, _| {
                    frames.push(frame.to_vec());
                    Ok(())
                },
            )
            .unwrap();
            frames
        };
        let original = collect(&Default::default());
        let request = fvid_media_info::DecodeTransform {
            grayworld: Some("enable=lt(n,1)".into()),
            ..Default::default()
        };
        let filtered = collect(&request);
        assert_eq!(filtered.len(), 3);
        assert_ne!(filtered[0], original[0]);
        assert_eq!(&filtered[1..], &original[1..]);
    }
}
