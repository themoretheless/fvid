//! Native mixed-role B-picture depth refinement with independent scalar pixels.
use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
use std::{io::Cursor, path::Path};
#[test]
fn refined_b_di_iv_handle_signed_camera_weights_reordering_and_software_seek() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("texture-refinement-b.json")).unwrap())
            .unwrap();
    let filters: [[i64; 8]; 4] = [
        [0, 0, 0, 64, 0, 0, 0, 0],
        [-1, 4, -10, 58, 17, -5, 1, 0],
        [-1, 4, -11, 40, 40, -11, 4, -1],
        [0, 1, -5, 17, 58, -10, 4, -1],
    ];
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 120);
    let mut rounding_witnesses = 0;
    let mut changed_iv = 0;
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let mode = case["mode"].as_str().unwrap();
        let disparity = if case["enabled"].as_bool().unwrap() {
            (case["scale"].as_i64().unwrap() * 254 + 64) >> 7
        } else {
            0
        };
        assert_eq!(disparity, case["disparity"].as_i64().unwrap());
        let weighted = case["weighted"].as_bool().unwrap();
        let pattern = |x: i64, y: i64, future: bool| {
            (3 * x.clamp(0, 31) + 5 * y.clamp(0, 31) + 17 + if future { 97 } else { 0 }) % 256
        };
        let cut = mode.strip_prefix("amp").map(|v| v.parse::<i64>().unwrap());
        let source = |x: i64, y: i64| {
            let x = x.clamp(0, 31);
            let y = y.clamp(0, 31);
            let direction = if cut.is_some_and(|cut| x >= cut) {
                -1
            } else {
                1
            };
            let a = pattern(x, y + direction, false);
            let b = pattern(x, y - 2 * direction, true);
            match mode {
                "intra" => pattern(x, y, false),
                "l0" => a,
                "l1" => b,
                "bi" | "amp8" | "amp24" => (a + b + 1) / 2,
                _ => panic!("mode"),
            }
        };
        let target = |x: i64, y: i64| {
            if let Some(cut) = cut {
                let lookup = ((((x / 8 * 8 + 4) + ((disparity + 2) >> 2)) >> 3) << 3).clamp(0, 31);
                let direction = if lookup < cut { 1 } else { -1 };
                let a = pattern(x, y + direction, false);
                let b = pattern(x, y - 2 * direction, true);
                return if weighted {
                    ((a + 3 * b + 4) >> 2).clamp(0, 255)
                } else {
                    (a + b + 1) >> 1
                };
            }
            let base = x + disparity.div_euclid(4);
            let sum: i64 = filters[disparity.rem_euclid(4) as usize]
                .iter()
                .enumerate()
                .map(|(i, &w)| w * source(base + i as i64 - 3, y))
                .sum();
            // DI uses the same current-view source in L0 index1 and L1 index2.
            // Explicit denom1 weights are (2,-3) and (0,9); retain filter precision.
            if weighted {
                ((2 * sum + 128 * (-3 + 9 + 1)) >> 8).clamp(0, 255)
            } else {
                ((sum + 32) >> 6).clamp(0, 255)
            }
        };
        if let Some(cut) = cut {
            for x in [4, 12, 20, 28] {
                let shifted = ((((x + ((disparity + 2) >> 2)) >> 3) << 3).clamp(0, 31)) < cut;
                if shifted != (x < cut) {
                    changed_iv += 1;
                }
            }
        } else if weighted {
            for y in 0..32 {
                for x in 0..32 {
                    let sum: i64 = filters[disparity.rem_euclid(4) as usize]
                        .iter()
                        .enumerate()
                        .map(|(i, &w)| w * source(x + disparity.div_euclid(4) + i as i64 - 3, y))
                        .sum();
                    let prematurely_rounded = ((sum + 32) >> 6).clamp(0, 255);
                    let wrong = ((2 * prematurely_rounded + 2 * 7) >> 2).clamp(0, 255);
                    if target(x, y) != wrong {
                        rounding_witnesses += 1;
                    }
                }
            }
        }
        let data = std::fs::read(root.join(name)).unwrap();
        let container =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(data.clone()), Limits::default())
                .unwrap();
        let timing: Vec<_> = (0..3)
            .map(|i| {
                let sample = container.tracks()[0].samples.get(i).unwrap();
                (sample.dts, sample.pts)
            })
            .collect();
        assert_eq!(timing, [(0, 0), (1, 2), (2, 1)], "{name}: encoded DTS/PTS");
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(data.clone()),
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
                assert_eq!(frame.presentation_time.ticks, i64::from(poc));
                if id == 3 {
                    continue;
                }
                if id == 5 && poc == 1 {
                    assert_eq!(
                        frame.decoded.binding.pps.pps.weighted_biprediction,
                        weighted
                    );
                    for y in (0..32).step_by(8) {
                        for x in (0..32).step_by(8) {
                            let motion =
                                frame.decoded.picture.prediction_motion_at([x, y]).unwrap();
                            for list in 0..2 {
                                let vector = motion[list].unwrap();
                                if let Some(cut) = cut {
                                    let lookup =
                                        ((((x / 8 * 8 + 4) as i64 + ((disparity + 2) >> 2)) >> 3)
                                            << 3)
                                            .clamp(0, 31);
                                    let direction = if lookup < cut { 1 } else { -1 };
                                    assert_eq!(vector.reference, 0);
                                    assert_eq!(
                                        vector.mv,
                                        [
                                            0,
                                            if list == 0 {
                                                4 * direction
                                            } else {
                                                -8 * direction
                                            }
                                        ]
                                        .map(|v| v as i64)
                                    );
                                } else {
                                    assert_eq!(vector.reference, if list == 0 { 1 } else { 2 });
                                    assert_eq!(vector.mv, [disparity, 0]);
                                }
                            }
                            let saved = frame
                                .decoded
                                .picture
                                .inter_view_disparity_at([x, y])
                                .unwrap();
                            assert_eq!(saved.is_some(), cut.is_some());
                            if let Some(saved) = saved {
                                assert_eq!(saved.vector.mv, [0, 0]);
                                assert_eq!(saved.refined_mv, [disparity, 0]);
                            }
                        }
                    }
                }
                for y in 0..32 {
                    for x in 0..32 {
                        let expected = if poc == 1 {
                            if id == 0 {
                                source(x as i64, y as i64)
                            } else {
                                target(x as i64, y as i64)
                            }
                        } else {
                            pattern(x as i64, y as i64, poc == 2)
                        };
                        assert_eq!(
                            frame.decoded.picture.planes[0].samples()[y * 32 + x],
                            expected as u16,
                            "{name} layer{id} POC{poc} x{x} y{y}"
                        );
                    }
                }
                for plane in &frame.decoded.picture.planes[1..] {
                    assert!(plane.samples().iter().all(|&v| v == 128), "{name}: chroma");
                }
            }
            assert_eq!(
                seen,
                [
                    (0, 0),
                    (3, 0),
                    (5, 0),
                    (0, 1),
                    (3, 1),
                    (5, 1),
                    (0, 2),
                    (3, 2),
                    (5, 2)
                ],
                "{name}"
            );
            reader.rewind();
        }
        use fvid::playback::video::{
            mp4::Mp4VideoReader,
            native::{NativeReader, RawFrame},
        };
        let check = |data: &[u8], poc: i64| {
            assert_eq!(data.len(), 1536);
            for y in 0..32 {
                for x in 0..32 {
                    let wanted = if poc == 1 {
                        target(x as i64, y as i64)
                    } else {
                        pattern(x as i64, y as i64, poc == 2)
                    };
                    assert_eq!(data[y * 32 + x], wanted as u8, "{name} packed POC{poc}");
                }
            }
            assert!(data[1024..].iter().all(|&v| v == 128));
        };
        let mut common =
            Mp4VideoReader::open_software(Cursor::new(data.clone()), Default::default(), 64 << 20)
                .unwrap();
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = common.read_frame().unwrap() {
                assert_eq!(common.selected_layer_id(), Some(5));
                let poc = frame.presentation_time.ticks;
                check(&frame.packed.unwrap().frame.data, poc);
                seen.push(poc);
            }
            assert_eq!(seen, [0, 1, 2]);
            common.rewind();
        }
        let mut native = NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
        for target_ms in [40, 80, 0, 40, 40] {
            let RawFrame::Planar(frame) = native
                .seek_raw(std::time::Duration::from_millis(target_ms))
                .unwrap()
                .unwrap()
            else {
                panic!("{name}: planar frame")
            };
            check(
                &frame.frame.data,
                if target_ms == 0 {
                    0
                } else if target_ms == 80 {
                    2
                } else {
                    1
                },
            );
        }
    }
    assert!(
        rounding_witnesses > 0,
        "weighted cases must detect premature per-predictor rounding"
    );
    assert!(
        changed_iv > 0,
        "depth refinement must change selected B source vectors"
    );
}
