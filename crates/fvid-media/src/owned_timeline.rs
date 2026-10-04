//! Stateless generic filter enable expressions evaluated at each input frame.
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    expression: Option<crate::owned_expression::Expression>,
}
impl Timeline {
    pub fn grayworld(args: &str) -> Result<Self, String> {
        let text = args.trim();
        if matches!(text, "" | "0") {
            return Ok(Self::default());
        }
        let text = text
            .strip_prefix("enable=")
            .ok_or("unknown grayworld option")?
            .trim();
        let text = if text.starts_with('\'') {
            text.strip_prefix('\'')
                .and_then(|v| v.strip_suffix('\''))
                .ok_or("unclosed timeline quote")?
        } else if text.starts_with('"') {
            text.strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .ok_or("unclosed timeline quote")?
        } else {
            text
        };
        let expression = crate::owned_expression::Expression::parse(text)?;
        expression.evaluate(&[
            ("n", 0.0),
            ("t", f64::NAN),
            ("w", 1.0),
            ("h", 1.0),
            ("pos", f64::NAN),
        ])?;
        Ok(Self {
            expression: Some(expression),
        })
    }
    pub fn enabled(
        &self,
        n: u64,
        t: Option<f64>,
        width: usize,
        height: usize,
    ) -> Result<bool, String> {
        match &self.expression {
            None => Ok(true),
            Some(expression) => Ok(expression
                .evaluate(&[
                    ("n", n as f64),
                    ("t", t.unwrap_or(f64::NAN)),
                    ("w", width as f64),
                    ("h", height as f64),
                    ("pos", f64::NAN),
                ])?
                .abs()
                >= 0.5),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_time_dimensions_nan_and_threshold() {
        let filter =
            Timeline::grayworld("enable='between(n,1,2)*gte(t,0.5)*eq(w,5)*eq(h,3)'").unwrap();
        assert!(!filter.enabled(0, Some(0.0), 5, 3).unwrap());
        assert!(filter.enabled(1, Some(0.5), 5, 3).unwrap());
        assert!(!filter.enabled(3, Some(1.5), 5, 3).unwrap());
        for (args, expected) in [
            ("enable=0.49", false),
            ("enable=-0.5", true),
            ("enable=NAN", false),
            ("enable=isnan(t)*isnan(pos)", true),
        ] {
            assert_eq!(
                Timeline::grayworld(args)
                    .unwrap()
                    .enabled(0, None, 1, 1)
                    .unwrap(),
                expected
            );
        }
        assert!(Timeline::grayworld("enable=unknown").is_err());
        assert!(Timeline::grayworld("enable='1").is_err());
    }
}

#[cfg(test)]
mod reference_acceptance {
    #[test]
    fn float_video_enable_cases_match_saved_reference() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        let input = std::fs::read(root.join("grayworld-grid.rgba_f32")).unwrap();
        for (case, args) in [
            "enable=lt(n,1)",
            "enable=gte(t,1)",
            "enable=eq(w,8)*eq(h,8)*between(n,1,2)",
            "enable=0.49",
            "enable=NAN",
            "enable=-0.5",
        ]
        .into_iter()
        .enumerate()
        {
            let expected =
                std::fs::read(root.join(format!("grayworld-timeline-reference-{case}.rgba_f32")))
                    .unwrap();
            let timeline = super::Timeline::grayworld(args).unwrap();
            let mut actual = Vec::new();
            for (n, frame) in input.chunks_exact(8 * 8 * 4 * 4).enumerate() {
                let mut rgb: Vec<f32> = frame
                    .chunks_exact(4)
                    .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                    .collect();
                let before = rgb.clone();
                if timeline
                    .enabled(n as u64, Some(n as f64 / 2.0), 8, 8)
                    .unwrap()
                {
                    crate::owned_grayworld::GrayWorld
                        .apply_rgb_f32(&mut rgb, 8, 8, 4)
                        .unwrap();
                } else {
                    assert_eq!(rgb, before);
                }
                actual.extend(rgb);
            }
            let expected: Vec<f32> = expected
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            assert_eq!(actual.len(), expected.len());
            for (a, b) in actual.iter().zip(&expected) {
                assert!(
                    (a - b).abs() <= 8.0 * f32::EPSILON * b.abs().max(1.0),
                    "case={case} actual={a} expected={b}"
                );
            }
        }
    }
    #[test]
    fn ffv1_scheduler_passes_frame_number_and_pts_to_filter() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors/grayworld-gamut-8.y4m");
        let output = std::env::temp_dir().join(format!(
            "fvid-grayworld-timeline-lib-{}.mkv",
            std::process::id()
        ));
        crate::owned_lossless::transcode_lossless(
            &source,
            &output,
            Default::default(),
            &Default::default(),
        )
        .unwrap();
        let collect = |args: Option<&str>| {
            let request = fvid_media_info::DecodeTransform {
                grayworld: args.map(str::to_owned),
                ..Default::default()
            };
            let mut frames = Vec::new();
            crate::owned_video_decode::decode_ffv1(
                &output,
                &request,
                Some(&mut |frame| {
                    frames.push(frame.pixels.to_vec());
                    Ok(())
                }),
                None,
            )
            .unwrap()
            .unwrap();
            frames
        };
        let original = collect(None);
        for (args, selected) in [("enable=lt(n,1)", 0), ("enable=gte(t,1)", 2)] {
            let frames = collect(Some(args));
            assert_eq!(frames.len(), 3);
            for i in 0..3 {
                if i == selected {
                    assert_ne!(frames[i], original[i]);
                } else {
                    assert_eq!(frames[i], original[i]);
                }
            }
        }
        std::fs::remove_file(output).unwrap();
    }
}
