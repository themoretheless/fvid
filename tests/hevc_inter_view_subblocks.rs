//! IV sub-block component reconstruction from owned coded AMP source motion.
use fvid::{
    codec::{
        hevc_motion::{
            self, Motion, Reference, inter_view_candidate as iv,
            subblock_motion::{Layout, Subblocks},
        },
        hevc_picture::Picture,
    },
    container::mp4::Limits,
    playback::video::mp4::LayeredMp4VideoReader,
};
use std::{io::Cursor, sync::Arc};
#[test]
fn coded_amp_motion_drives_iv_subblocks_and_exact_y_cb_cr_prediction() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("iv-subblocks-oracles.json")).unwrap())
            .unwrap();
    let data = std::fs::read(root.join("iv-subblocks-oracles.y16")).unwrap();
    let expected: Vec<u16> = data
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes(p.try_into().unwrap()))
        .collect();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 200);
    for magnitude in [1, 4] {
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(
                std::fs::read(root.join(format!("iv-subblocks-source-{magnitude}.mp4"))).unwrap(),
            ),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut base: Option<Arc<Picture>> = None;
            let mut source: Option<Arc<Picture>> = None;
            let mut seen = Vec::new();
            while let Some(frame) = reader.read_frame().unwrap() {
                seen.push((frame.decoded.layer_id, frame.decoded.poc));
                if frame.decoded.layer_id == 0 && frame.decoded.poc == 0 {
                    base = Some(frame.decoded.picture.clone());
                }
                if frame.decoded.layer_id == 0 && frame.decoded.poc == 1 {
                    source = Some(frame.decoded.picture.clone());
                }
            }
            assert_eq!(seen, [(0, 0), (3, 0), (0, 1), (3, 1), (0, 2), (3, 2)]);
            let base = base.unwrap();
            let source = source.unwrap();
            for component in 0..3 {
                let side = if component == 0 { 32 } else { 16 };
                for y in 0..side {
                    for x in 0..side {
                        let value = if component == 0 {
                            (x * 3 + y * 5 + 17) % 256
                        } else {
                            (x * 13 + y * 7 + 29 + (component - 1) * 83) % 256
                        };
                        assert_eq!(base.planes[component].samples()[y * side + x], value as u16);
                    }
                }
            }
            for case in cases
                .iter()
                .filter(|c| c["magnitude"].as_i64().unwrap() == magnitude)
            {
                let rect = [0, 1, 2, 3].map(|i| case["rect"][i].as_u64().unwrap() as u32);
                let log = case["log"].as_u64().unwrap() as u8;
                let disparity = [0, 1].map(|i| case["disparity"][i].as_i64().unwrap());
                let layout = Layout::new(rect, [32, 32], log).unwrap();
                let budget = Subblocks::<Motion>::required_bytes(layout).unwrap();
                let bidirectional = log % 2 == 0;
                let l0 = [99, 0];
                let l1 = if bidirectional { vec![0] } else { vec![] };
                let mut calls = Vec::new();
                let result = iv::subblocks(
                    &source,
                    rect,
                    [32, 32],
                    log,
                    |point| {
                        calls.push(point);
                        Ok(disparity)
                    },
                    [&l0, &l1],
                    bidirectional,
                    budget,
                )
                .unwrap();
                let size = [0, 1].map(|i| case["size"][i].as_u64().unwrap() as u32);
                assert_eq!(result.size, size);
                assert_eq!(
                    calls[0],
                    [layout.default_rectangle()[0], layout.default_rectangle()[1]]
                );
                assert_eq!(calls.len(), layout.count() + 1);
                assert_eq!(
                    result.default[0].unwrap().mv,
                    [0, case["default"].as_i64().unwrap()]
                );
                let vectors: Vec<i64> = case["motion"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_i64().unwrap())
                    .collect();
                assert_eq!(result.motion.len(), vectors.len());
                let references = [
                    vec![
                        Reference {
                            layer_id: 0,
                            long_term: false,
                            poc: 99,
                            picture: Some(base.clone()),
                        },
                        Reference {
                            layer_id: 0,
                            long_term: false,
                            poc: 0,
                            picture: Some(base.clone()),
                        },
                    ],
                    if bidirectional {
                        vec![Reference {
                            layer_id: 0,
                            long_term: false,
                            poc: 0,
                            picture: Some(base.clone()),
                        }]
                    } else {
                        vec![]
                    },
                ];
                for (index, motion) in result.motion.iter().enumerate() {
                    assert_eq!(motion[0].unwrap().reference, 1);
                    assert_eq!(motion[0].unwrap().mv, [0, vectors[index]]);
                    assert_eq!(motion[1].is_some(), bidirectional);
                    if let Some(v) = motion[1] {
                        assert_eq!(v.reference, 0);
                        assert_eq!(v.mv, [0, vectors[index]]);
                    }
                }
                for component in 0..3 {
                    let shift = usize::from(component != 0);
                    let width = (rect[2] as usize) >> shift;
                    let height = (rect[3] as usize) >> shift;
                    let mut pixels = vec![0u16; width * height];
                    for (index, motion) in result.motion.iter().enumerate() {
                        let sub = layout.rectangle(index).unwrap();
                        let prediction = hevc_motion::predict(
                            &references,
                            *motion,
                            sub,
                            component,
                            8,
                            None,
                            &mut Vec::new(),
                        )
                        .unwrap();
                        let sw = (sub[2] as usize) >> shift;
                        let sh = (sub[3] as usize) >> shift;
                        for y in 0..sh {
                            let at = (((sub[1] - rect[1]) as usize >> shift) + y) * width
                                + ((sub[0] - rect[0]) as usize >> shift);
                            pixels[at..at + sw].copy_from_slice(&prediction[y * sw..(y + 1) * sw]);
                        }
                    }
                    let offset = case["offsets"][component].as_u64().unwrap() as usize;
                    assert_eq!(
                        pixels,
                        expected[offset..offset + pixels.len()],
                        "component{component}: {case}"
                    );
                    if rect == [0, 0, 32, 32] && log == 3 && disparity == [0, 0] {
                        assert_eq!(pixels, source.planes[component].samples());
                    }
                }
                assert!(
                    iv::subblocks(
                        &source,
                        rect,
                        [32, 32],
                        log,
                        |_| Ok(disparity),
                        [&l0, &l1],
                        bidirectional,
                        budget - 1
                    )
                    .is_err()
                );
            }
            let mut calls = 0;
            let absent = iv::subblocks(
                &source,
                [0, 0, 32, 32],
                [32, 32],
                3,
                |_| {
                    calls += 1;
                    Ok([0, 0])
                },
                [&[123], &[]],
                false,
                std::mem::size_of::<Subblocks<Motion>>(),
            )
            .unwrap();
            assert_eq!(absent.default, [None; 2]);
            assert!(absent.motion.is_empty());
            assert_eq!(calls, 1);
            reader.rewind();
        }
    }
}
