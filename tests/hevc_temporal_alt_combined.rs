use fvid::playback::video::{
    mp4::LayeredMp4VideoReader,
    native::{NativeReader, RawFrame},
};
use std::{io::Cursor, path::Path, time::Duration};

#[test]
fn alternative_reference_is_published_only_for_selected_col_not_combined_candidates() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("temporal-alt-combined-manifest.json")).unwrap(),
    )
    .unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 15);
    for case in cases {
        let name = case["file"].as_str().unwrap();
        let size = case["size"].as_u64().unwrap() as usize;
        let data = std::fs::read(root.join(name)).unwrap();
        let refs: [u8; 2] = std::array::from_fn(|i| case["references"][i].as_u64().unwrap() as u8);
        let mv: [i64; 2] = std::array::from_fn(|i| case["motion"][i].as_i64().unwrap());
        // Scalar sample oracle uses labeled past/future/current pictures,
        // independently of the decoder's candidate and reference-list logic.
        let pixel = |x: usize, y: usize, refs: [u8; 2], mv: [i64; 2]| -> u16 {
            let clamp = |v: i64| v.clamp(0, size as i64 - 1) as usize;
            let temporal = |row: usize, future: bool| {
                ((x * 7 + row * 11 + 37 + if future { 97 } else { 0 }) % 256) as u16
            };
            let source = |row: usize| {
                (temporal(clamp(row as i64 + 1), false) + temporal(clamp(row as i64 - 2), true) + 1)
                    / 2
            };
            let samples: [u16; 2] = std::array::from_fn(|list| {
                let row = clamp(y as i64 + mv[list]);
                if refs[list] == 0 {
                    source(row)
                } else {
                    temporal(row, list == 1)
                }
            });
            (samples[0] + samples[1] + 1) / 2
        };
        let index = case["index"].as_u64().unwrap();
        if index == 2 || index == 3 {
            // The fixture must distinguish keeping Col's initial indices in
            // combined entries from prematurely publishing both alternatives.
            assert!(
                (0..32).any(|y| (32..64).any(|x| pixel(x, y, refs, mv) != pixel(x, y, [1, 1], mv))),
                "{name}: combined probe lacks a distinguishing pixel"
            );
        }
        if index == 1 {
            assert!(
                (0..32).any(|y| (32..64).any(|x| pixel(x, y, refs, mv) != pixel(x, y, [0, 0], mv))),
                "{name}: Col probe lacks a distinguishing pixel"
            );
        }
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(data.clone()),
            Default::default(),
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
                if id != 8 || poc != 1 {
                    continue;
                }
                let picture = &frame.decoded.picture;
                // Establish the spatial source first, then qualify the probe.
                for y in 0..32usize {
                    for x in 0..64usize {
                        let (expected_refs, expected_mv) = if x < 32 {
                            ([1, 1], [2, -2])
                        } else {
                            (refs, mv)
                        };
                        assert_eq!(
                            picture.planes[0].samples()[y * size + x],
                            pixel(x, y, expected_refs, expected_mv),
                            "{name}: pixel{x},{y}"
                        );
                        let motion = picture.prediction_motion_at([x as u32, y as u32]).unwrap();
                        for list in 0..2 {
                            let v = motion[list].unwrap();
                            assert_eq!(
                                v.reference, expected_refs[list],
                                "{name}: ref L{list} at{x},{y}"
                            );
                            assert_eq!(
                                v.mv,
                                [0, expected_mv[list] * 4],
                                "{name}: MV L{list} at{x},{y}"
                            );
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
        let mut native = NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
        for target in [40, 0, 80, 40, 40] {
            let frame = native
                .seek_raw(Duration::from_millis(target))
                .unwrap()
                .unwrap();
            if target != 40 {
                continue;
            }
            let RawFrame::Planar(packed) = frame else {
                panic!("{name}: expected planar")
            };
            for y in 0..32 {
                for x in 32..64 {
                    assert_eq!(
                        packed.frame.data[y * size + x],
                        pixel(x, y, refs, mv) as u8,
                        "{name}: seek probe{x},{y}"
                    );
                }
            }
        }
    }
}
