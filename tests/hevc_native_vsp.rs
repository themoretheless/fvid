//! Native VSP admission and decoding; expanded exact oracles accompany the author.
use fvid::{
    container::mp4::Limits,
    playback::video::{
        mp4::{LayeredMp4VideoReader, Mp4VideoReader},
        native::{NativeReader, RawFrame},
    },
};
use std::{io::Cursor, time::Duration};
#[test]
fn native_vsp_coded_target_decodes_all_current_au_components() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let mut reader = LayeredMp4VideoReader::open(
        std::io::Cursor::new(std::fs::read(root.join("native-vsp-first.mp4")).unwrap()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    let mut seen = Vec::new();
    while let Some(frame) = reader.read_frame().unwrap() {
        seen.push((frame.decoded.layer_id, frame.decoded.poc));
    }
    assert_eq!(seen, [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)]);
}

#[test]
fn native_vsp_matches_independent_subblock_pixels_and_motion() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("native-vsp.json")).unwrap()).unwrap();
    let pixels: Vec<u16> = std::fs::read(root.join("native-vsp.y16"))
        .unwrap()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let motions: Vec<i64> = std::fs::read(root.join("native-vsp-motion.i64"))
        .unwrap()
        .chunks_exact(8)
        .map(|b| i64::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 144);
    assert!(manifest["wrong_refinement_witnesses"].as_u64().unwrap() > 0);
    for case in cases {
        let name = case["file"].as_str().unwrap();
        let size = case["size"].as_u64().unwrap() as usize;
        let at = case["oracle_offset"].as_u64().unwrap() as usize;
        let mt = case["motion_offset"].as_u64().unwrap() as usize;
        let mut reader = LayeredMp4VideoReader::open(
            std::io::Cursor::new(std::fs::read(root.join(name)).unwrap()),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                let id = frame.decoded.layer_id;
                let poc = frame.decoded.poc;
                seen.push((id, poc));
                if id != 5 || poc != 1 {
                    continue;
                }
                let picture = &frame.decoded.picture;
                let mut start = at;
                for c in 0..3 {
                    let samples = picture.planes[c].samples();
                    for (i, (&actual, &expected)) in samples
                        .iter()
                        .zip(&pixels[start..start + samples.len()])
                        .enumerate()
                    {
                        assert_eq!(actual, expected, "{name}: plane{c} sample{i}");
                    }
                    start += samples.len();
                }
                assert_eq!(start - at, size * size * 3 / 2);
                for y in (0..size).step_by(8) {
                    for x in (0..size).step_by(8) {
                        let pos = [x as u32, y as u32];
                        let motion = picture.prediction_motion_at(pos).unwrap();
                        assert_eq!(motion[0].unwrap().reference, 1, "{name}: reference {x},{y}");
                        assert_eq!(
                            motion[0].unwrap().mv,
                            [motions[mt + y * size + x], 0],
                            "{name}: motion {x},{y}"
                        );
                        assert!(motion[1].is_none());
                        assert_eq!(picture.prediction_is_skipped_at(pos).unwrap(), true);
                        assert!(picture.inter_view_disparity_at(pos).unwrap().is_none());
                    }
                }
            }
            assert_eq!(
                seen,
                [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)],
                "{name}"
            );
            reader.rewind();
        }
        let expected = |poc| -> Vec<u8> {
            if poc == 1 {
                return pixels[at..at + size * size * 3 / 2]
                    .iter()
                    .map(|&v| v as u8)
                    .collect();
            }
            (0..3)
                .flat_map(|c| {
                    let side = if c == 0 { size } else { size / 2 };
                    (0..side).flat_map(move |y| {
                        (0..side).map(move |x| {
                            ((if c == 0 {
                                3 * x + 5 * y + 17
                            } else {
                                13 * x + 7 * y + 29 + 83 * (c - 1) + [71, 113][c - 1]
                            }) % 256) as u8
                        })
                    })
                })
                .collect()
        };
        let data = std::fs::read(root.join(name)).unwrap();
        let mut common =
            Mp4VideoReader::open_software(Cursor::new(data.clone()), Default::default(), 64 << 20)
                .unwrap();
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = common
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                assert_eq!(common.selected_layer_id(), Some(5));
                let poc = frame.presentation_time.ticks;
                assert_eq!(
                    frame.packed.unwrap().frame.data,
                    expected(poc),
                    "{name}: playback POC{poc}"
                );
                seen.push(poc);
            }
            assert_eq!(seen, [0, 1], "{name}: presentation order");
            common.rewind();
        }
        let mut native = NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
        for target in [40, 0, 40, 40] {
            let RawFrame::Planar(frame) = native
                .seek_raw(Duration::from_millis(target))
                .unwrap()
                .unwrap()
            else {
                panic!("{name}: planar output");
            };
            assert_eq!(
                frame.frame.data,
                expected(if target == 0 { 0 } else { 1 }),
                "{name}: seek {target}ms"
            );
        }
    }
}

#[test]
fn unavailable_camera_disables_vsp_and_preserves_temporal_prediction() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let mut reader = LayeredMp4VideoReader::open(
        std::io::Cursor::new(std::fs::read(root.join("native-vsp-no-camera.mp4")).unwrap()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    let mut target = None;
    let mut checked = false;
    while let Some(frame) = reader.read_frame().unwrap() {
        if frame.decoded.layer_id != 5 {
            continue;
        }
        let picture = &frame.decoded.picture;
        let samples: Vec<Vec<u16>> = picture
            .planes
            .iter()
            .map(|p| p.samples().to_vec())
            .collect();
        if frame.decoded.poc == 0 {
            target = Some(samples);
        } else {
            assert_eq!(samples, target.as_ref().unwrap().clone());
            let motion = picture.prediction_motion_at([0, 0]).unwrap();
            assert_eq!(motion[0].unwrap().reference, 0);
            assert_eq!(motion[0].unwrap().mv, [0, 0]);
            checked = true;
        }
    }
    assert!(checked);
}

#[test]
fn b_slice_vsp_prefers_l0_and_applies_uni_weights() {
    check_b_vsp(false, false, false, false);
}
#[test]
fn b_slice_vsp_uses_l1_when_l0_has_no_matching_view() {
    check_b_vsp(true, false, false, false);
}
#[test]
fn b_slice_vsp_adds_signed_transform_residual_after_subblock_prediction() {
    check_b_vsp(false, true, false, false);
}
#[test]
fn b_slice_vsp_reconstructs_signed_chroma_and_chroma_only_residual() {
    check_b_vsp(false, true, true, false);
}
#[test]
fn b_slice_vsp_reconstructs_vertical_ac_in_luma_and_chroma() {
    check_b_vsp(false, true, true, true);
}
fn check_b_vsp(l1_only: bool, residual: bool, chroma: bool, ac: bool) {
    let prefix = if ac {
        "native-vsp-ac"
    } else if chroma {
        "native-vsp-residual-chroma"
    } else if residual {
        "native-vsp-residual"
    } else if l1_only {
        "native-vsp-b-l1"
    } else {
        "native-vsp-b"
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join(format!("{prefix}.json"))).unwrap())
            .unwrap();
    let golden: Vec<u16> = std::fs::read(root.join(format!("{prefix}.y16")))
        .unwrap()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(
        cases.len(),
        if chroma {
            40
        } else if residual {
            20
        } else {
            10
        }
    );
    for case in cases {
        let l1_only = if residual {
            case["l1"].as_bool().unwrap()
        } else {
            l1_only
        };
        let name = case["file"].as_str().unwrap();
        let at = case["oracle_offset"].as_u64().unwrap() as usize;
        let data = std::fs::read(root.join(name)).unwrap();
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(data.clone()),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        let mut seen = Vec::new();
        while let Some(frame) = reader
            .read_frame()
            .unwrap_or_else(|e| panic!("{name}: {e}"))
        {
            if frame.decoded.layer_id != 5 {
                continue;
            }
            let poc = frame.decoded.poc;
            seen.push(poc);
            if poc != 1 {
                continue;
            }
            let picture = &frame.decoded.picture;
            let mut start = at;
            for c in 0..3 {
                let samples = picture.planes[c].samples();
                for (i, (&a, &b)) in samples
                    .iter()
                    .zip(&golden[start..start + samples.len()])
                    .enumerate()
                {
                    assert_eq!(a, b, "{name}: plane{c} sample{i}");
                }
                start += samples.len();
            }
            for y in (0..32).step_by(8) {
                for x in (0..32).step_by(8) {
                    let motion = picture.prediction_motion_at([x as u32, y as u32]).unwrap();
                    assert_eq!(
                        picture
                            .prediction_is_skipped_at([x as u32, y as u32])
                            .unwrap(),
                        !residual,
                        "{name}: skip metadata"
                    );
                    let list = usize::from(l1_only);
                    assert_eq!(
                        motion[list].unwrap().reference,
                        if l1_only { 2 } else { 1 },
                        "{name}"
                    );
                    assert!(motion[1 - list].is_none());
                    assert_eq!(
                        motion[usize::from(l1_only)].unwrap().mv,
                        [case["motion"][y * 32 + x].as_i64().unwrap(), 0],
                        "{name}"
                    );
                }
            }
        }
        assert_eq!(seen, [0, 1, 2], "{name}");
        let mut native = NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
        for target in [40, 0, 40, 40] {
            let RawFrame::Planar(frame) = native
                .seek_raw(Duration::from_millis(target))
                .unwrap()
                .unwrap()
            else {
                panic!("{name}: planar B seek");
            };
            let expected: Vec<u8> = if target == 0 {
                (0..32)
                    .flat_map(|y| (0..32).map(move |x| ((3 * x + 5 * y + 17) % 256) as u8))
                    .chain((0..512).map(|i| {
                        if chroma {
                            let c = i / 256;
                            let x = i % 16;
                            let y = (i % 256) / 16;
                            ((13 * x + 7 * y + 29 + 83 * c + [71, 113][c]) % 256) as u8
                        } else {
                            128
                        }
                    }))
                    .collect()
            } else {
                golden[at..at + 1536].iter().map(|&v| v as u8).collect()
            };
            assert_eq!(frame.frame.data, expected, "{name}: B seek {target}ms");
        }
    }
}
