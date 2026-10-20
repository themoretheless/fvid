use fvid::{
    codec::{
        hevc_motion::{
            self, Reference, Vector, disparity::CameraTransform, texture::Identity,
            view_synthesis as vsp,
        },
        hevc_picture::Picture,
    },
    container::mp4::Limits,
    playback::video::mp4::LayeredMp4VideoReader,
};
use std::sync::Arc;
fn picture(name: &str, layer: u8, poc: i32) -> Arc<Picture> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let mut reader = LayeredMp4VideoReader::open(
        std::io::Cursor::new(std::fs::read(root.join(name)).unwrap()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    while let Some(frame) = reader.read_frame().unwrap() {
        if frame.decoded.layer_id == layer && frame.decoded.poc == poc {
            return frame.decoded.picture.clone();
        }
    }
    panic!("missing owned source picture");
}
#[test]
fn vsp_depth_to_subblock_prediction_matches_independent_scalar_luma_oracle() {
    let depth = picture("disparity-depth-source.mp4", 3, 1);
    let texture = picture("inter-view-temporal-scaling-on.mp4", 0, 1);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("vsp-luma-oracles.json")).unwrap())
            .unwrap();
    let data = std::fs::read(root.join("vsp-luma-oracles.y16")).unwrap();
    let expected: Vec<u16> = data
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes(p.try_into().unwrap()))
        .collect();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 48);
    let mut phases = std::collections::BTreeSet::new();
    let mut splits = std::collections::BTreeSet::new();
    for case in cases {
        let rect = [0, 1, 2, 3].map(|i| case["rect"][i].as_u64().unwrap() as u32);
        let values = [0, 1, 2, 3].map(|i| case["values"][i].as_i64().unwrap() as i32);
        let vector = [0, 1].map(|i| case["vector"][i].as_i64().unwrap());
        let list = case["list"].as_u64().unwrap() as usize;
        let camera = CameraTransform::new(values, 2, 8).unwrap();
        let wrong = Identity { view: 9, poc: 1 };
        let right = Identity { view: 0, poc: 1 };
        let l0 = if list == 0 {
            vec![wrong, right, right]
        } else {
            vec![wrong]
        };
        let l1 = [right];
        let lists = [&l0[..], &l1[..]];
        let block = [rect[2] as usize, rect[3] as usize];
        let budget = vsp::required_bytes(block).unwrap();
        assert!(
            vsp::derive(
                Some(&depth),
                Some(camera),
                lists,
                true,
                0,
                rect,
                vector,
                budget - 1
            )
            .is_err()
        );
        let result = vsp::derive(
            Some(&depth),
            Some(camera),
            lists,
            true,
            0,
            rect,
            vector,
            budget,
        )
        .unwrap()
        .unwrap();
        let horizontal = case["horizontal"].as_bool().unwrap();
        splits.insert(horizontal);
        assert_eq!(
            result.subblock_size(),
            if horizontal { [8, 4] } else { [4, 8] }
        );
        let disparities: Vec<i64> = case["disparities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_i64().unwrap())
            .collect();
        assert_eq!(result.samples(), disparities);
        phases.extend(result.samples().iter().map(|d| d.rem_euclid(4)));
        let mut references = [vec![], vec![]];
        references[list] = (0..if list == 0 { 3 } else { 1 })
            .map(|_| Reference {
                layer_id: 0,
                long_term: true,
                poc: 1,
                picture: Some(texture.clone()),
            })
            .collect();
        let mut output = vec![0u16; block[0] * block[1]];
        for i in 0..result.subblock_count() {
            let (sub, motion) = result.subblock(i).unwrap();
            let active = motion[list].unwrap();
            assert_eq!(active.reference, if list == 0 { 1 } else { 0 });
            assert!(motion[1 - list].is_none());
            let motion = motion.map(|v| {
                v.map(|v| Vector {
                    reference: v.reference,
                    mv: v.mv,
                })
            });
            let pixels =
                hevc_motion::predict(&references, motion, sub, 0, 8, None, &mut Vec::new())
                    .unwrap();
            for y in 0..sub[3] as usize {
                let target = (sub[1] as usize - rect[1] as usize + y) * block[0] + sub[0] as usize
                    - rect[0] as usize;
                output[target..target + sub[2] as usize]
                    .copy_from_slice(&pixels[y * sub[2] as usize..(y + 1) * sub[2] as usize]);
            }
        }
        assert!(result.subblock(result.subblock_count()).is_err());
        assert!(result.motion_at(block).is_err());
        let offset = case["offset"].as_u64().unwrap() as usize;
        assert_eq!(output, expected[offset..offset + output.len()], "{case}");
    }
    assert_eq!(phases.len(), 4);
    assert_eq!(splits.len(), 2);
    assert!(
        vsp::derive(None, None, [&[], &[]], true, 0, [0, 0, 8, 8], [0, 0], 0)
            .unwrap()
            .is_none()
    );
}

#[test]
fn vsp_nonconstant_chroma_matches_all_eighth_sample_phases() {
    let depth = picture("disparity-depth-source.mp4", 3, 1);
    let texture = picture("vsp-chroma-source.mp4", 0, 1);
    for component in 1..=2 {
        assert_eq!(texture.planes[component].dimensions(), [16, 16]);
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(
                    texture.planes[component].samples()[y * 16 + x],
                    ((x * 13 + y * 7 + 29 + (component - 1) * 83) % 256) as u16
                );
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("vsp-chroma-oracles.json")).unwrap())
            .unwrap();
    let data = std::fs::read(root.join("vsp-chroma-oracles.y16")).unwrap();
    let expected: Vec<u16> = data
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes(p.try_into().unwrap()))
        .collect();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 72);
    let mut phases = std::collections::BTreeSet::new();
    let mut splits = std::collections::BTreeSet::new();
    let mut clipped = false;
    for case in cases {
        let rect = [0, 1, 2, 3].map(|i| case["rect"][i].as_u64().unwrap() as u32);
        let values = [0, 1, 2, 3].map(|i| case["values"][i].as_i64().unwrap() as i32);
        let vector = [0, 1].map(|i| case["vector"][i].as_i64().unwrap());
        let list = case["list"].as_u64().unwrap() as usize;
        let camera = CameraTransform::new(values, 2, 8).unwrap();
        let wrong = Identity { view: 9, poc: 1 };
        let right = Identity { view: 0, poc: 1 };
        let l0 = if list == 0 {
            vec![wrong, right, right]
        } else {
            vec![wrong]
        };
        let l1 = [right];
        let block = [rect[2] as usize, rect[3] as usize];
        let result = vsp::derive(
            Some(&depth),
            Some(camera),
            [&l0, &l1],
            true,
            0,
            rect,
            vector,
            vsp::required_bytes(block).unwrap(),
        )
        .unwrap()
        .unwrap();
        splits.insert(result.subblock_size());
        phases.extend(result.samples().iter().map(|d| d.rem_euclid(8)));
        let mut references = [vec![], vec![]];
        references[list] = (0..if list == 0 { 3 } else { 1 })
            .map(|_| Reference {
                layer_id: 0,
                long_term: true,
                poc: 1,
                picture: Some(texture.clone()),
            })
            .collect();
        let cw = block[0] / 2;
        let ch = block[1] / 2;
        for component in 1..=2 {
            let mut output = vec![0u16; cw * ch];
            for i in 0..result.subblock_count() {
                let (sub, motion) = result.subblock(i).unwrap();
                let motion = motion.map(|v| {
                    v.map(|v| Vector {
                        reference: v.reference,
                        mv: v.mv,
                    })
                });
                let pixels = hevc_motion::predict(
                    &references,
                    motion,
                    sub,
                    component,
                    8,
                    None,
                    &mut Vec::new(),
                )
                .unwrap();
                let sw = sub[2] as usize / 2;
                for y in 0..sub[3] as usize / 2 {
                    let target = ((sub[1] - rect[1]) as usize / 2 + y) * cw
                        + (sub[0] - rect[0]) as usize / 2;
                    output[target..target + sw].copy_from_slice(&pixels[y * sw..(y + 1) * sw]);
                }
            }
            let offset = case["offset"].as_u64().unwrap() as usize + (component - 1) * cw * ch;
            assert_eq!(
                output,
                expected[offset..offset + output.len()],
                "component {component}: {case}"
            );
            clipped |= output.iter().any(|&p| p == 0 || p == 255);
        }
    }
    assert_eq!(phases.len(), 8);
    assert_eq!(splits.len(), 2);
    assert!(clipped);
}

#[test]
fn owned_nonconstant_chroma_source_preserves_all_views_timing_and_rewind() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge/vsp-chroma-source.mp4");
    let mut reader = LayeredMp4VideoReader::open(
        std::io::Cursor::new(std::fs::read(path).unwrap()),
        Limits::default(),
        1,
        0,
        64 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        let mut seen = Vec::new();
        while let Some(frame) = reader.read_frame().unwrap() {
            seen.push((frame.decoded.layer_id, frame.decoded.poc));
            assert_eq!(frame.presentation_time.ticks, i64::from(frame.decoded.poc));
            assert_eq!(frame.duration.ticks, 1);
            let picture = &frame.decoded.picture;
            for component in 0..3 {
                let side = if component == 0 { 32 } else { 16 };
                assert_eq!(picture.planes[component].dimensions(), [side, side]);
                for y in 0..side {
                    for x in 0..side {
                        let expected = if component == 0 {
                            (x * 3 + y * 5 + 17) % 256
                        } else {
                            (x * 13 + y * 7 + 29 + (component - 1) * 83) % 256
                        };
                        assert_eq!(
                            picture.planes[component].samples()[y * side + x],
                            expected as u16
                        );
                    }
                }
            }
        }
        assert_eq!(seen, [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)]);
        reader.rewind();
    }
}
