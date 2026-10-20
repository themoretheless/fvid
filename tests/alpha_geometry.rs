//! Source-depth in-memory geometry acceptance; fixtures are authored, not private media.
use fvid::{
    native_geometry::{Transpose, VideoGeometry},
    playback_native::{NativeReader, PackedPlanar, RawFrame},
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn cases() -> Vec<serde_json::Value> {
    ["ffv1-alpha.json", "ffv1-v3-alpha.json"]
        .into_iter()
        .flat_map(|file| {
            let m: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root().join(file)).unwrap()).unwrap();
            m["cases"].as_array().unwrap().clone()
        })
        .collect()
}
fn words(data: &[u8], depth: u8) -> Vec<u16> {
    if depth == 8 {
        data.iter().map(|&v| u16::from(v)).collect()
    } else {
        data.chunks_exact(2)
            .map(|s| u16::from_le_bytes([s[0], s[1]]))
            .collect()
    }
}
type Rows = Vec<Vec<u16>>;
fn rows(data: &[u8], width: usize, depth: u8) -> Rows {
    words(data, depth)
        .chunks_exact(width)
        .map(|r| r.to_vec())
        .collect()
}
fn flip(mut r: Rows, h: bool, v: bool) -> Rows {
    if h {
        for row in &mut r {
            row.reverse();
        }
    }
    if v {
        r.reverse();
    }
    r
}
fn transpose(r: Rows, mode: Transpose) -> Rows {
    let (w, h) = (r[0].len(), r.len());
    let mut out = vec![vec![0; h]; w];
    for (y, row) in r.iter().enumerate() {
        for (x, &sample) in row.iter().enumerate() {
            let (xx, yy) = match mode {
                Transpose::Clock => (h - 1 - y, x),
                Transpose::CClock => (y, w - 1 - x),
                Transpose::ClockFlip => (h - 1 - y, w - 1 - x),
                Transpose::CClockFlip => (y, x),
            };
            out[yy][xx] = sample;
        }
    }
    out
}
fn pad(r: Rows, maximum: u16) -> Rows {
    let mut out = vec![vec![maximum; r[0].len() + 4]; r.len() + 4];
    for (y, row) in r.iter().enumerate() {
        out[y + 2][2..2 + row.len()].copy_from_slice(row);
    }
    out
}
fn double(r: Rows) -> Rows {
    r.into_iter()
        .flat_map(|r| {
            let row = r.into_iter().flat_map(|v| [v, v]).collect::<Vec<_>>();
            [row.clone(), row]
        })
        .collect()
}
fn color_only(p: &PackedPlanar) -> RawFrame {
    RawFrame::Planar(Arc::new(
        PackedPlanar::new(
            fvid::native_geometry::GeometryFrame {
                width: p.frame.width,
                height: p.frame.height,
                subsampling: p.frame.subsampling,
                data: p.frame.data.clone(),
            },
            p.depth,
            p.colour,
        )
        .unwrap(),
    ))
}
fn assert_result(p: &PackedPlanar, geometry: VideoGeometry, expected: &Rows, media: bool) {
    let original_color = p.frame.data.clone();
    let original_alpha = p.alpha.clone();
    let out = if media {
        geometry.apply_packed_media(p)
    } else {
        geometry.apply_packed(p)
    }
    .unwrap();
    assert_eq!(
        (out.frame.width, out.frame.height),
        (expected[0].len(), expected.len())
    );
    assert_eq!(out.depth, p.depth);
    assert_eq!(
        (out.colour.kr, out.colour.kb, out.colour.full),
        (p.colour.kr, p.colour.kb, p.colour.full)
    );
    assert_eq!(
        words(out.alpha.as_ref().unwrap(), out.depth),
        expected.iter().flatten().copied().collect::<Vec<_>>()
    );
    let raw = color_only(p);
    let color = if media {
        geometry.apply_media(&raw, p.frame.width, p.frame.height)
    } else {
        geometry.apply(&raw, p.frame.width, p.frame.height)
    }
    .unwrap();
    let RawFrame::Planar(plain) = &raw else {
        unreachable!()
    };
    let plain = if media {
        geometry.apply_packed_media(plain)
    } else {
        geometry.apply_packed(plain)
    }
    .unwrap();
    assert!(plain.alpha.is_none());
    assert_eq!(plain.frame.data, color.data);
    assert_eq!(out.frame.data, color.data);
    assert_eq!(out.frame.subsampling, color.subsampling);
    assert_eq!(p.frame.data, original_color);
    assert_eq!(p.alpha, original_alpha);
}
#[test]
fn geometry_preserves_every_authored_alpha_sample_and_color_contract() {
    for case in cases() {
        let mut reader = NativeReader::software(
            Cursor::new(std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            32 << 20,
        )
        .unwrap();
        for frame in case["frames"].as_array().unwrap() {
            let RawFrame::Planar(p) = reader.read_frame_raw().unwrap().unwrap() else {
                panic!("source depth")
            };
            let source = rows(
                &std::fs::read(root().join(frame["alpha"].as_str().unwrap())).unwrap(),
                p.frame.width,
                p.depth,
            );
            let maximum = ((1u32 << p.depth) - 1) as u16;
            for media in [false, true] {
                assert_result(&p, Default::default(), &source, media);
                for (h, v) in [(true, false), (false, true), (true, true)] {
                    assert_result(
                        &p,
                        VideoGeometry {
                            horizontal_flip: h,
                            vertical_flip: v,
                            ..Default::default()
                        },
                        &flip(source.clone(), h, v),
                        media,
                    );
                }
                for mode in [
                    Transpose::Clock,
                    Transpose::CClock,
                    Transpose::ClockFlip,
                    Transpose::CClockFlip,
                ] {
                    assert_result(
                        &p,
                        VideoGeometry {
                            transpose: Some(mode),
                            ..Default::default()
                        },
                        &transpose(source.clone(), mode),
                        media,
                    );
                }
                let (w, h) = (p.frame.width, p.frame.height);
                assert_result(
                    &p,
                    VideoGeometry {
                        pad: Some([w + 4, h + 4, 2, 2]),
                        ..Default::default()
                    },
                    &pad(source.clone(), maximum),
                    media,
                );
                assert_result(
                    &p,
                    VideoGeometry {
                        scale: Some([w * 2, h * 2]),
                        ..Default::default()
                    },
                    &double(source.clone()),
                    media,
                );
                let down: Rows = source
                    .iter()
                    .map(|r| r.iter().skip(1).step_by(2).copied().collect())
                    .collect();
                assert_result(
                    &p,
                    VideoGeometry {
                        scale: Some([w / 2, h]),
                        ..Default::default()
                    },
                    &down,
                    media,
                );
                let cropped: Rows = source.iter().map(|r| r[2..].to_vec()).collect();
                let expected = double(pad(
                    transpose(flip(cropped, true, true), Transpose::Clock),
                    maximum,
                ));
                let (cw, ch) = (h + 4, w - 2 + 4);
                assert_result(
                    &p,
                    VideoGeometry {
                        crop: Some([2, 0, w - 2, h]),
                        horizontal_flip: true,
                        vertical_flip: true,
                        transpose: Some(Transpose::Clock),
                        pad: Some([cw, ch, 2, 2]),
                        scale: Some([cw * 2, ch * 2]),
                        ..Default::default()
                    },
                    &expected,
                    media,
                );
            }
        }
    }
}
#[test]
fn display_rotation_and_stored_crop_precede_user_geometry_for_alpha_too() {
    for case in cases().into_iter().filter(|c| c["coder"] == 1) {
        let mut reader = NativeReader::software(
            Cursor::new(std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            32 << 20,
        )
        .unwrap();
        let RawFrame::Planar(p) = reader.read_frame_raw().unwrap().unwrap() else {
            panic!("source depth")
        };
        let r = rows(
            &std::fs::read(root().join(case["frames"][0]["alpha"].as_str().unwrap())).unwrap(),
            p.frame.width,
            p.depth,
        );
        for rotation in [0, 90, 180, 270] {
            let normalized = match rotation {
                0 => r.clone(),
                90 => transpose(r.clone(), Transpose::Clock),
                180 => flip(r.clone(), true, true),
                _ => transpose(r.clone(), Transpose::CClock),
            };
            let insets = if normalized[0].len() > 2 {
                [2, 0, 0, 0]
            } else {
                [0, 2, 0, 0]
            };
            let cropped: Rows = if insets[0] != 0 {
                normalized.iter().map(|r| r[2..].to_vec()).collect()
            } else {
                normalized[2..].to_vec()
            };
            let (w, h) = (cropped[0].len(), cropped.len());
            let expected = double(flip(cropped, true, false));
            let geometry = VideoGeometry {
                horizontal_flip: true,
                scale: Some([w * 2, h * 2]),
                ..Default::default()
            };
            for media in [false, true] {
                let out = if media {
                    geometry.apply_packed_display_media(&p, rotation, insets)
                } else {
                    geometry.apply_packed_display(&p, rotation, insets)
                }
                .unwrap();
                assert_eq!((out.frame.width, out.frame.height), (w * 2, h * 2));
                assert_eq!(
                    words(out.alpha.as_ref().unwrap(), out.depth),
                    expected.iter().flatten().copied().collect::<Vec<_>>()
                );
            }
        }
    }
}
#[test]
fn oblique_rotation_and_padding_keep_opaque_samples_and_full_precision() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("ffv1-alpha.json")).unwrap()).unwrap();
    for case in m["opaque_rotations"].as_array().unwrap() {
        let mut reader = NativeReader::software(
            Cursor::new(std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            32 << 20,
        )
        .unwrap();
        let RawFrame::Planar(p) = reader.read_frame_raw().unwrap().unwrap() else {
            panic!("source depth")
        };
        for angle in [37.0, -37.0, 90.0, 180.0, 270.0] {
            let geometry = VideoGeometry {
                rotate: Some(fvid::media_info::RotateAngle { degrees: angle }),
                ..Default::default()
            };
            let out = geometry.apply_packed(&p).unwrap();
            let maximum = ((1u32 << p.depth) - 1) as u16;
            assert!(
                words(out.alpha.as_ref().unwrap(), p.depth)
                    .iter()
                    .all(|&v| v == maximum)
            );
            let color = geometry
                .apply(&color_only(&p), p.frame.width, p.frame.height)
                .unwrap();
            assert_eq!(out.frame.data, color.data);
        }
    }
}
#[test]
fn malformed_storage_geometry_and_fill_fail_without_mutating_the_source() {
    let mut reader = NativeReader::software(
        Cursor::new(std::fs::read(root().join("ffv1-alpha-420-10-coder1-v1.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    let RawFrame::Planar(p) = reader.read_frame_raw().unwrap().unwrap() else {
        panic!("alpha")
    };
    let color = p.frame.data.clone();
    let alpha = p.alpha.clone();
    for g in [
        VideoGeometry {
            crop: Some([1, 0, 2, 2]),
            ..Default::default()
        },
        VideoGeometry {
            scale: Some([0, 2]),
            ..Default::default()
        },
        VideoGeometry {
            rotate: Some(fvid::media_info::RotateAngle { degrees: f64::NAN }),
            ..Default::default()
        },
    ] {
        assert!(g.apply_packed(&p).is_err());
        assert_eq!(p.frame.data, color);
        assert_eq!(p.alpha, alpha);
    }
    assert!(
        VideoGeometry::default()
            .apply_packed_display(&p, 45, [0; 4])
            .is_err()
    );
    assert!(
        VideoGeometry::default()
            .apply_packed_display(&p, 0, [u32::MAX; 4])
            .is_err()
    );
    let mut p = PackedPlanar::with_alpha(
        fvid::native_geometry::GeometryFrame {
            width: p.frame.width,
            height: p.frame.height,
            subsampling: p.frame.subsampling,
            data: p.frame.data.clone(),
        },
        p.depth,
        p.colour,
        p.alpha.clone(),
    )
    .unwrap();
    p.alpha.as_mut().unwrap().pop();
    assert!(VideoGeometry::default().apply_packed(&p).is_err());
    let rgb = fvid::native_geometry::GeometryFrame {
        width: 1,
        height: 1,
        subsampling: None,
        data: vec![0; 3],
    };
    assert!(fvid_media::owned_rotate::rotate_rgb_with_fill(&rgb, 37., 8, 256).is_err());
}

#[test]
fn transformed_alpha_frames_round_trip_through_the_owned_ffv1_codec() {
    for case in cases()
        .into_iter()
        .filter(|c| c["version"] == 3 && c["coder"] == 1)
    {
        let mut reader = NativeReader::software(
            Cursor::new(std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            32 << 20,
        )
        .unwrap();
        let RawFrame::Planar(p) = reader.read_frame_raw().unwrap().unwrap() else {
            panic!("source depth")
        };
        let (w, h) = (p.frame.width, p.frame.height);
        let geometry = VideoGeometry {
            crop: Some([2, 0, w - 2, h]),
            horizontal_flip: true,
            transpose: Some(Transpose::Clock),
            pad: Some([h + 4, w + 2, 2, 2]),
            scale: Some([(h + 4) * 2, (w + 2) * 2]),
            ..Default::default()
        };
        let out = geometry.apply_packed(&p).unwrap();
        let packet = fvid_media::owned_ffv1_encoder::encode_with_alpha(
            &out.frame,
            out.depth,
            out.alpha.as_ref().unwrap(),
        )
        .unwrap();
        let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::new(
            out.frame.width,
            out.frame.height,
            32 << 20,
        )
        .unwrap();
        let decoded = decoder.decode(&packet).unwrap();
        assert_eq!(decoded.depth, out.depth);
        assert_eq!(decoded.frame.subsampling, out.frame.subsampling);
        assert_eq!(decoded.frame.data, out.frame.data);
        assert_eq!(decoded.alpha, out.alpha);
    }
}

#[test]
fn stored_matroska_orientation_and_crop_drive_the_alpha_geometry_api() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("ffv1-alpha.json")).unwrap()).unwrap();
    let display = &m["display_geometry"][0];
    let source = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["file"] == display["source"])
        .unwrap();
    let mut reader = NativeReader::software(
        Cursor::new(std::fs::read(root().join(display["file"].as_str().unwrap())).unwrap()),
        32 << 20,
    )
    .unwrap();
    for frame in source["frames"].as_array().unwrap() {
        let RawFrame::Planar(p) = reader.read_frame_raw().unwrap().unwrap() else {
            panic!("raw source depth")
        };
        assert_eq!(reader.rotation(), 90);
        assert_eq!(reader.insets(), [0, 2, 0, 0]);
        assert_eq!(reader.dimensions(), [2, 4]);
        let geometry = VideoGeometry {
            horizontal_flip: true,
            scale: Some([4, 4]),
            ..Default::default()
        };
        let out = geometry
            .apply_packed_display(&p, reader.rotation(), reader.insets())
            .unwrap();
        assert_eq!((out.frame.width, out.frame.height, out.depth), (4, 4, 16));
        let alpha = words(
            &std::fs::read(root().join(frame["alpha"].as_str().unwrap())).unwrap(),
            16,
        );
        let expected = double(vec![vec![alpha[2], alpha[6]], vec![alpha[3], alpha[7]]]);
        assert_eq!(
            words(out.alpha.as_ref().unwrap(), 16),
            expected.iter().flatten().copied().collect::<Vec<_>>()
        );
        let color = words(
            &std::fs::read(root().join(frame["color"].as_str().unwrap())).unwrap(),
            16,
        );
        let mut expected = Vec::new();
        for row in [[2, 6], [3, 7]] {
            let expanded = row
                .into_iter()
                .flat_map(|pixel| [&color[pixel * 3..pixel * 3 + 3]; 2])
                .flatten()
                .copied()
                .collect::<Vec<_>>();
            expected.extend_from_slice(&expanded);
            expected.extend_from_slice(&expanded);
        }
        assert_eq!(words(&out.frame.data, 16), expected);
    }
    assert!(reader.read_frame_raw().unwrap().is_none());
}
