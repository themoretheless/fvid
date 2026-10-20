//! Variable-depth refinement across independently authored multi-CTU skip CUs.
use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
use std::{io::Cursor, path::Path};
#[test]
fn variable_depth_refinement_uses_prior_cu_disparity_before_sampling_depth() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("multicu-refinement.json")).unwrap())
            .unwrap();
    assert!(manifest["base_propagation_witnesses"].as_u64().unwrap() > 0);
    let golden: Vec<u16> = std::fs::read(root.join("multicu-refinement.y16"))
        .unwrap()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let mut phases = [false; 4];
    let mut propagated = 0;
    let mut changed_base_results = 0;
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let size = case["size"].as_u64().unwrap() as usize;
        let at = case["oracle_offset"].as_u64().unwrap() as usize;
        let wanted = &golden[at..at + size * size];
        let blocks = case["blocks"].as_array().unwrap();
        let mut vectors: Vec<i64> = Vec::new();
        for (i, block) in blocks.iter().enumerate() {
            let x = block["origin"][0].as_i64().unwrap();
            let y = block["origin"][1].as_i64().unwrap();
            let cols = size / 32;
            let base = if i % cols > 0 {
                vectors[i - 1]
            } else if i >= cols {
                vectors[i - cols]
            } else {
                0
            };
            assert_eq!(base, block["base"].as_i64().unwrap());
            propagated += usize::from(base != 0);
            let depth = [0, 31]
                .into_iter()
                .flat_map(|dy| {
                    [0, 31].into_iter().map(move |dx| {
                        let xx = (x + ((base + 2) >> 2) + dx).clamp(0, size as i64 - 1);
                        (7 * xx + 11 * (y + dy) + 37) % 256
                    })
                })
                .max()
                .unwrap();
            assert_eq!(depth, block["depth"].as_i64().unwrap());
            let vector = (case["scale"].as_i64().unwrap() * depth
                + case["offset"].as_i64().unwrap() * 256
                + 64)
                >> 7;
            assert_eq!(vector, block["vector"].as_i64().unwrap());
            let unshifted_depth = [0, 31]
                .into_iter()
                .flat_map(|dy| {
                    [0, 31].into_iter().map(move |dx| {
                        (7 * (x + dx).clamp(0, size as i64 - 1) + 11 * (y + dy) + 37) % 256
                    })
                })
                .max()
                .unwrap();
            let without_base = (case["scale"].as_i64().unwrap() * unshifted_depth
                + case["offset"].as_i64().unwrap() * 256
                + 64)
                >> 7;
            changed_base_results += usize::from(vector != without_base);
            let a1 = if i % cols > 0 {
                Some(vectors[i - 1])
            } else {
                None
            };
            let b1 = if i >= cols {
                Some(vectors[i - cols])
            } else {
                None
            };
            let b0 = if i >= cols && i % cols + 1 < cols {
                Some(vectors[i - cols + 1])
            } else {
                None
            };
            let mut prefix = Vec::new();
            if let Some(v) = a1 {
                prefix.push(v);
            }
            if b1.is_some() && b1 != a1 {
                prefix.push(b1.unwrap());
            }
            if b0.is_some() && b0 != b1 {
                prefix.push(b0.unwrap());
            }
            let merge_index = if a1 == Some(vector) || b1 == Some(vector) {
                prefix.iter().position(|&v| v == vector).unwrap()
            } else {
                prefix.len()
            };
            assert_eq!(merge_index, block["merge_index"].as_u64().unwrap() as usize);
            vectors.push(vector);
            phases[vector.rem_euclid(4) as usize] = true;
        }
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(std::fs::read(root.join(name)).unwrap()),
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
                if poc == 1 && id == 3 {
                    for y in 0..size {
                        for x in 0..size {
                            assert_eq!(
                                frame.decoded.picture.planes[0].samples()[y * size + x],
                                ((7 * x + 11 * y + 37) % 256) as u16,
                                "{name}: source depth"
                            );
                        }
                    }
                }
                if poc == 1 && id == 5 {
                    let actual_vectors: Vec<_> = blocks
                        .iter()
                        .map(|b| {
                            frame
                                .decoded
                                .picture
                                .prediction_motion_at([
                                    b["origin"][0].as_u64().unwrap() as u32,
                                    b["origin"][1].as_u64().unwrap() as u32,
                                ])
                                .unwrap()[0]
                                .unwrap()
                                .mv[0]
                        })
                        .collect();
                    assert_eq!(actual_vectors, vectors, "{name}: CU vectors");
                    for (i, (&a, &b)) in frame.decoded.picture.planes[0]
                        .samples()
                        .iter()
                        .zip(wanted)
                        .enumerate()
                    {
                        assert_eq!(a, b, "{name}: pixel {},{}", i % size, i / size);
                    }
                    for p in &frame.decoded.picture.planes[1..] {
                        assert!(p.samples().iter().all(|&v| v == 128));
                    }
                    for y in (0..size).step_by(8) {
                        for x in (0..size).step_by(8) {
                            let motion = frame
                                .decoded
                                .picture
                                .prediction_motion_at([x as u32, y as u32])
                                .unwrap();
                            assert_eq!(motion[0].unwrap().reference, 1);
                            assert_eq!(
                                motion[0].unwrap().mv,
                                [vectors[y / 32 * (size / 32) + x / 32], 0],
                                "{name}: vector {x},{y}"
                            );
                            assert!(motion[1].is_none());
                        }
                    }
                }
            }
            assert_eq!(seen, [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)]);
            reader.rewind();
        }
        use fvid::playback::video::{
            mp4::Mp4VideoReader,
            native::{NativeReader, RawFrame},
        };
        let check = |data: &[u8], poc: i64| {
            assert_eq!(data.len(), size * size * 3 / 2);
            for y in 0..size {
                for x in 0..size {
                    let value = if poc == 0 {
                        ((3 * x + 5 * y + 17) % 256) as u16
                    } else {
                        wanted[y * size + x]
                    };
                    assert_eq!(
                        data[y * size + x],
                        value as u8,
                        "{name}: packed POC{poc} x{x} y{y}"
                    );
                }
            }
            assert!(data[size * size..].iter().all(|&v| v == 128));
        };
        let data = std::fs::read(root.join(name)).unwrap();
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
            assert_eq!(seen, [0, 1]);
            common.rewind();
        }
        let mut native = NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
        for target in [40, 0, 40, 40] {
            let RawFrame::Planar(frame) = native
                .seek_raw(std::time::Duration::from_millis(target))
                .unwrap()
                .unwrap()
            else {
                panic!("{name}: planar frame")
            };
            check(&frame.frame.data, if target == 0 { 0 } else { 1 });
        }
    }
    assert!(
        phases.into_iter().all(|v| v),
        "all quarter-luma phases required"
    );
    assert!(propagated > 0);
    assert_eq!(
        changed_base_results,
        manifest["base_propagation_witnesses"].as_u64().unwrap() as usize
    );
}
