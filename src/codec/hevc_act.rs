//! H.265 8.6.8.2: inverse residual adaptive colour transform.
use crate::{Result, invalid};
pub(crate) fn inverse(
    samples: &mut [Vec<i32>; 3],
    depths: [u8; 2],
    extended: bool,
    bypass: bool,
) -> Result<()> {
    let n = samples[0].len();
    if !(8..=16).contains(&depths[0])
        || !(8..=16).contains(&depths[1])
        || samples.iter().any(|p| p.len() != n)
    {
        return Err(invalid("invalid HEVC ACT residual geometry or depth"));
    }
    let max_depth = depths[0].max(depths[1]);
    let delta = [
        max_depth - depths[0],
        max_depth - depths[1],
        max_depth - depths[1],
    ];
    let ranges = depths.map(|d| 1i64 << if extended { (d + 6).max(15) } else { 15 });
    for i in 0..n {
        let mut r = std::array::from_fn::<_, 3, _>(|c| {
            let range = ranges[usize::from(c != 0)];
            i64::from(samples[c][i]).clamp(-range, range - 1)
        });
        if !bypass {
            r[0] <<= delta[0];
            r[1] <<= delta[1] + 1;
            r[2] <<= delta[2] + 1;
        }
        let t = r[0] - (r[1] >> 1);
        r = [t + r[1], t - (r[2] >> 1), r[2] + t - (r[2] >> 1)];
        for c in 0..3 {
            if !bypass {
                r[c] = (r[c] + if delta[c] > 0 { 1 << (delta[c] - 1) } else { 0 }) >> delta[c];
            }
            samples[c][i] =
                i32::try_from(r[c]).map_err(|_| invalid("HEVC ACT residual overflow"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn bypass_is_the_inverse_of_reversible_ycocg_for_signed_residuals() {
        for g in [-32768i32, -255, -3, -1, 0, 1, 255, 32767] {
            for b in [-32768i32, -7, 0, 9, 32767] {
                for r in [-32768i32, -5, 0, 11, 32767] {
                    let co = r - b;
                    let t = b + (co >> 1);
                    let cg = g - t;
                    let y = t + (cg >> 1);
                    if [y, cg, co].iter().any(|&v| !(-32768..=32767).contains(&v)) {
                        continue;
                    }
                    let mut samples = [vec![y], vec![cg], vec![co]];
                    super::inverse(&mut samples, [8; 2], false, true).unwrap();
                    assert_eq!(samples, [vec![g], vec![b], vec![r]]);
                }
            }
        }
    }
    #[test]
    fn quantized_residuals_keep_chroma_scaling_signed_rounding_and_mixed_depth() {
        let mut samples = [vec![3, -3], vec![2, -2], vec![-1, 1]];
        super::inverse(&mut samples, [8; 2], false, false).unwrap();
        assert_eq!(samples, [vec![5, -5], vec![2, -2], vec![0, 0]]);
        let mut samples = [vec![3, -3], vec![2, -2], vec![-1, 1]];
        super::inverse(&mut samples, [8, 10], false, false).unwrap();
        assert_eq!(samples, [vec![4, -3], vec![11, -11], vec![9, -9]]);
    }
    #[test]
    fn coefficient_clipping_precedes_the_transform_without_sample_clipping() {
        for depth in 8..=16 {
            for extended in [false, true] {
                let range = 1i32 << if extended { (depth + 6).max(15) } else { 15 };
                let mut samples = [vec![i32::MAX], vec![i32::MIN], vec![0]];
                super::inverse(&mut samples, [depth; 2], extended, false).unwrap();
                assert_eq!(
                    samples,
                    [vec![-1], vec![2 * range - 1], vec![2 * range - 1]]
                );
            }
        }
    }
    #[test]
    fn invalid_geometry_and_depth_are_rejected_before_modification() {
        for depths in [[7, 8], [8, 17]] {
            let mut samples = [vec![1], vec![2], vec![3]];
            let before = samples.clone();
            assert!(super::inverse(&mut samples, depths, false, false).is_err());
            assert_eq!(samples, before);
        }
        let mut samples = [vec![1], vec![2, 3], vec![4]];
        let before = samples.clone();
        assert!(super::inverse(&mut samples, [8; 2], false, false).is_err());
        assert_eq!(samples, before);
    }
    #[test]
    fn every_owned_scm_stream_uses_act_and_matches_pixels_after_reset() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for depth in [8u8, 10] {
            for name in ["intra", "inter", "parallel", "offsets", "bypass"] {
                let stem = format!("hevc-scc-act-{name}-rext{depth}");
                let data = std::fs::read(root.join(format!("{stem}.mp4"))).unwrap();
                let expected = std::fs::read(root.join(format!("{stem}.yuv"))).unwrap();
                let mut input = crate::container::mp4::Mp4Reader::open(
                    std::io::Cursor::new(data),
                    Default::default(),
                )
                .unwrap();
                let mut decoder = super::super::hevc_decoder::HevcDecoder::from_configuration(
                    &input.tracks()[0].configuration,
                    16 << 20,
                )
                .unwrap();
                for _ in 0..2 {
                    let mut actual = Vec::new();
                    let mut active = 0;
                    let mut inter_active = 0;
                    for frame in 0..3 {
                        let mut packet = Vec::new();
                        input.read_packet(0, frame, &mut packet).unwrap();
                        let inter = decoder
                            .slice_headers(&packet)
                            .unwrap()
                            .iter()
                            .any(|h| h.slice_type != super::super::hevc_cabac::SliceType::I);
                        let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
                        if inter {
                            inter_active += decoded.picture.act_blocks;
                        }
                        active += decoded.picture.act_blocks;
                        for p in &decoded.picture.planes {
                            for &sample in p.samples() {
                                if depth == 8 {
                                    actual.push(sample as u8);
                                } else {
                                    actual.extend_from_slice(&sample.to_le_bytes());
                                }
                            }
                        }
                    }
                    assert!(active > 0, "{stem} never used ACT");
                    if name != "intra" {
                        assert!(inter_active > 0, "{stem} never used inter ACT");
                    }
                    assert_eq!(actual, expected, "{stem}");
                    decoder.reset();
                }
            }
        }
    }
}

#[cfg(test)]
mod depth_fixtures {
    #[test]
    fn mixed_deep_and_slice_offset_streams_use_intra_and_inter_act() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors");
        for (kind, pairs) in [
            (
                "depth",
                vec![[8u8, 10], [10, 8], [14, 14], [14, 10], [10, 14]],
            ),
            ("slice", vec![[8u8, 8], [10, 10], [8, 10], [10, 8]]),
        ] {
            for depths in pairs {
                for parallel in [false, true] {
                    let stem = format!(
                        "hevc-scc-act-{kind}-y{}-c{}{}",
                        depths[0],
                        depths[1],
                        if parallel { "-parallel" } else { "" }
                    );
                    let data = std::fs::read(root.join(format!("{stem}.mp4"))).unwrap();
                    let oracle = std::fs::read(root.join(format!("{stem}.yuv"))).unwrap();
                    let mut input = crate::container::mp4::Mp4Reader::open(
                        std::io::Cursor::new(data),
                        Default::default(),
                    )
                    .unwrap();
                    let mut decoder = super::super::hevc_decoder::HevcDecoder::from_configuration(
                        &input.tracks()[0].configuration,
                        16 << 20,
                    )
                    .unwrap();
                    for _ in 0..2 {
                        let mut actual = Vec::new();
                        let mut active = [0usize; 2];
                        for frame in 0..3 {
                            let mut packet = Vec::new();
                            input.read_packet(0, frame, &mut packet).unwrap();
                            let inter =
                                decoder.slice_headers(&packet).unwrap().iter().any(|h| {
                                    h.slice_type != super::super::hevc_cabac::SliceType::I
                                });
                            let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
                            active[usize::from(inter)] += decoded.picture.act_blocks;
                            for plane in &decoded.picture.planes {
                                for &sample in plane.samples() {
                                    if depths.iter().all(|&d| d == 8) {
                                        actual.push(sample as u8);
                                    } else {
                                        actual.extend_from_slice(&sample.to_le_bytes());
                                    }
                                }
                            }
                        }
                        assert!(
                            active.iter().all(|&n| n > 0),
                            "{stem}: active intra/inter {active:?}"
                        );
                        assert_eq!(actual, oracle, "{stem}");
                        decoder.reset();
                    }
                }
            }
        }
    }
}
