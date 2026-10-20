use fvid::{
    container::mp4::Limits,
    playback::video::mp4::{LayeredMp4VideoReader, Mp4VideoReader},
};
use std::{io::Cursor, path::Path};
#[test]
fn depth_iv_b_references_weights_pixels_and_reordered_player_output() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("depth-iv-b-manifest.json")).unwrap())
            .unwrap();
    let bytes = std::fs::read(root.join("depth-iv-b-oracles.y16")).unwrap();
    let expected: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes(p.try_into().unwrap()))
        .collect();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 64);
    for case in cases {
        let name = case["file"].as_str().unwrap();
        let size = case["size"].as_u64().unwrap() as usize;
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
        assert_eq!(
            timing,
            [(0, 0), (1, 2), (2, 1)],
            "{name}: coded decode/presentation order"
        );
        let source_offset = case["offsets"][0].as_u64().unwrap() as usize;
        let target_offset = case["offsets"][1].as_u64().unwrap() as usize;
        let expected_source = &expected[source_offset..source_offset + size * size];
        let expected_target = &expected[target_offset..target_offset + size * size];
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(data.clone()),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                let id = frame.decoded.layer_id;
                let poc = frame.decoded.poc;
                seen.push((id, poc));
                assert_eq!(frame.presentation_time.ticks, i64::from(poc), "{name}: pts");
                assert_eq!(frame.duration.ticks, 1);
                let samples = frame.decoded.picture.planes[0].samples();
                if poc == 1 && id == 3 {
                    assert_eq!(samples, expected_source, "{name}: source B pixels");
                } else if poc == 1 && id == 8 {
                    assert_eq!(samples, expected_target, "{name}: target B pixels");
                } else {
                    for y in 0..size {
                        for x in 0..size {
                            let value = if id == 0 {
                                128
                            } else {
                                ((x * 7 + y * 11 + 37 + if poc == 2 { 97 } else { 0 }) % 256) as u16
                            };
                            assert_eq!(
                                samples[y * size + x],
                                value,
                                "{name}: layer{id} POC{poc} pixel{x},{y}"
                            );
                        }
                    }
                }
                if poc == 1 && (id == 3 || id == 8) {
                    for y in (0..size).step_by(8) {
                        for x in (0..size).step_by(8) {
                            let motion = frame
                                .decoded
                                .picture
                                .prediction_motion_at([x as u32, y as u32])
                                .unwrap();
                            if id == 3 {
                                if case["intra"].as_bool().unwrap() {
                                    assert_eq!(motion, [None; 2]);
                                    continue;
                                }
                                let mode = case["mode"].as_str().unwrap();
                                for list in 0..2 {
                                    let active = mode == "bi"
                                        || mode == "l0" && list == 0
                                        || mode == "l1" && list == 1;
                                    assert_eq!(
                                        motion[list].is_some(),
                                        active,
                                        "{name}: source list{list}"
                                    );
                                    if let Some(v) = motion[list] {
                                        assert_eq!(v.reference, 0);
                                        assert_eq!(
                                            v.mv,
                                            [0, if list == 0 { 4 } else { -8 }],
                                            "{name}: source MV"
                                        );
                                    }
                                }
                            } else {
                                assert_eq!(
                                    frame.decoded.binding.pps.pps.weighted_biprediction,
                                    case["weighted"].as_bool().unwrap()
                                );
                                for list in 0..2 {
                                    let vector = motion[list].unwrap();
                                    assert_eq!(
                                        vector.reference,
                                        case["references"][list].as_u64().unwrap() as u8,
                                        "{name}: target list{list}"
                                    );
                                    assert_eq!(
                                        vector.mv,
                                        [0, case["motion"][list].as_i64().unwrap() * 4],
                                        "{name}: target MV{x},{y}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
            assert_eq!(
                seen,
                [
                    (0, 0),
                    (3, 0),
                    (5, 0),
                    (8, 0),
                    (0, 1),
                    (3, 1),
                    (5, 1),
                    (8, 1),
                    (0, 2),
                    (3, 2),
                    (5, 2),
                    (8, 2)
                ]
            );
            reader.rewind();
        }
        let y_plane = |poc: i64| -> Vec<u8> {
            if poc == 1 {
                return expected_target.iter().map(|&p| p as u8).collect();
            }
            (0..size)
                .flat_map(|y| {
                    (0..size).map(move |x| {
                        ((x * 7 + y * 11 + 37 + if poc == 2 { 97 } else { 0 }) % 256) as u8
                    })
                })
                .collect()
        };
        let mut common =
            Mp4VideoReader::open_software(Cursor::new(data.clone()), Limits::default(), 64 << 20)
                .unwrap();
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = common
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: common {e}"))
            {
                assert_eq!(common.selected_layer_id(), Some(8));
                let poc = frame.presentation_time.ticks;
                seen.push(poc);
                let packed = frame.packed.unwrap();
                assert_eq!(
                    &packed.frame.data[..size * size],
                    y_plane(poc),
                    "{name}: presentation POC{poc}"
                );
                assert!(packed.frame.data[size * size..].iter().all(|&p| p == 128));
            }
            assert_eq!(seen, [0, 1, 2]);
            common.rewind();
        }
        use fvid::playback::video::native::{NativeReader, RawFrame};
        let mut native = NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
        // Reuse one reader: stale current-AU references or output queues must
        // not leak across backward, forward, or repeated seeks to the B frame.
        for poc in [1, 0, 2, 1, 1, 0, 2, 1] {
            let frame = native
                .seek_raw(std::time::Duration::from_millis(poc * 40))
                .unwrap_or_else(|e| panic!("{name}: seek POC{poc}: {e}"))
                .unwrap();
            let RawFrame::Planar(packed) = frame else {
                panic!("expected native depth planar frame")
            };
            assert_eq!(
                &packed.frame.data[..size * size],
                y_plane(poc as i64),
                "{name}: repeated seek POC{poc}"
            );
            assert!(packed.frame.data[size * size..].iter().all(|&p| p == 128));
        }
    }
}
