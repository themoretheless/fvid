//! Saved skip disparity and source-motion selection across multiple texture CUs.
use fvid::{
    container::mp4::Limits,
    playback::video::{
        mp4::{LayeredMp4VideoReader, Mp4VideoReader},
        native::{NativeReader, RawFrame},
    },
};
use std::{io::Cursor, path::Path, time::Duration};
#[test]
fn multicu_iv_preserves_saved_refinement_and_uses_own_reference_pixels() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("multicu-iv.json")).unwrap()).unwrap();
    let golden: Vec<u16> = std::fs::read(root.join("multicu-iv.y16"))
        .unwrap()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 48);
    let mut row_restarts = 0;
    let mut changed_selections = 0;
    for case in cases {
        let name = case["file"].as_str().unwrap();
        let size = case["size"].as_u64().unwrap() as usize;
        let side = size / 2;
        let cut = case["cut"].as_i64().unwrap();
        let cols = size / 32;
        let at = case["oracle_offset"].as_u64().unwrap() as usize;
        let blocks = case["blocks"].as_array().unwrap();
        for (i, block) in blocks.iter().enumerate() {
            let base = if i % cols > 0 {
                blocks[i - 1]["refined"].as_i64().unwrap()
            } else {
                0
            };
            assert_eq!(base, block["base"].as_i64().unwrap());
            if i >= cols && i % cols == 0 && blocks[i - cols]["refined"].as_i64().unwrap() != 0 {
                row_restarts += 1;
            }
            let horizontal = block["refined"].as_i64().unwrap();
            for dx in [0, 8, 16, 24] {
                let origin = (i % cols * 32) as i64 + dx;
                let shifted =
                    (((origin + 4 + ((horizontal + 2) >> 2)) >> 3) << 3).clamp(0, size as i64 - 1);
                let original = (((origin + 4) >> 3) << 3).clamp(0, size as i64 - 1);
                changed_selections += usize::from((shifted % 32 < cut) != (original % 32 < cut));
            }
            let x = (i % cols * 32) as i64;
            let y = (i / cols * 32) as i64;
            let depth = [0, 31]
                .into_iter()
                .flat_map(|dy| {
                    [0, 31].into_iter().map(move |dx| {
                        (7 * (x + ((base + 2) >> 2) + dx).clamp(0, size as i64 - 1)
                            + 11 * (y + dy)
                            + 37)
                            % 256
                    })
                })
                .max()
                .unwrap();
            assert_eq!(depth, block["depth"].as_i64().unwrap());
            assert_eq!(
                (case["scale"].as_i64().unwrap() * depth
                    + case["offset"].as_i64().unwrap() * 256
                    + 64)
                    >> 7,
                block["refined"].as_i64().unwrap()
            );
        }
        let expected = |id: u8, poc: i64, component: usize| -> Vec<u16> {
            let n = if component == 0 { size } else { side };
            if poc == 1 {
                let start = at
                    + if id == 5 { size * size * 3 / 2 } else { 0 }
                    + [0, size * size, size * size + side * side][component];
                return golden[start..start + n * n].to_vec();
            }
            (0..n)
                .flat_map(|y| {
                    (0..n).map(move |x| {
                        ((if component == 0 {
                            3 * x + 5 * y + 17
                        } else {
                            13 * x
                                + 7 * y
                                + 29
                                + 83 * (component - 1)
                                + if id == 5 { [71, 113][component - 1] } else { 0 }
                        }) % 256) as u16
                    })
                })
                .collect()
        };
        let data = std::fs::read(root.join(name)).unwrap();
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
                let poc = i64::from(frame.decoded.poc);
                seen.push((id, poc));
                assert_eq!(frame.presentation_time.ticks, poc);
                if id == 3 {
                    continue;
                }
                let picture = &frame.decoded.picture;
                for c in 0..3 {
                    let wanted = expected(id, poc, c);
                    assert_eq!(picture.planes[c].samples().len(), wanted.len());
                    for (i, (&a, &b)) in picture.planes[c].samples().iter().zip(&wanted).enumerate()
                    {
                        assert_eq!(a, b, "{name}: layer{id} POC{poc} c{c} index{i}");
                    }
                }
                if poc == 1 {
                    for y in (0..size).step_by(8) {
                        for x in (0..size).step_by(8) {
                            let block = &blocks[y / 32 * cols + x / 32];
                            let refined = block["refined"].as_i64().unwrap();
                            let lookup = if id == 0 {
                                x as i64
                            } else {
                                ((((x / 8 * 8 + 4) as i64 + ((refined + 2) >> 2)) >> 3) << 3)
                                    .clamp(0, size as i64 - 1)
                            };
                            let dy = if lookup % 32 < cut { 4 } else { -4 };
                            let motion =
                                picture.prediction_motion_at([x as u32, y as u32]).unwrap();
                            assert_eq!(motion[0].unwrap().reference, 0);
                            assert_eq!(motion[0].unwrap().mv, [0, dy], "{name}: motion {x},{y}");
                            assert!(motion[1].is_none());
                            assert_eq!(
                                picture
                                    .prediction_is_skipped_at([x as u32, y as u32])
                                    .unwrap(),
                                id == 5
                            );
                            let saved = picture
                                .inter_view_disparity_at([x as u32, y as u32])
                                .unwrap();
                            assert_eq!(saved.is_some(), id == 5);
                            if let Some(saved) = saved {
                                assert_eq!(saved.vector.reference_view, 0);
                                assert_eq!(saved.vector.mv, [block["base"].as_i64().unwrap(), 0]);
                                assert_eq!(saved.refined_mv, [refined, 0]);
                            }
                        }
                    }
                }
            }
            assert_eq!(seen, [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)]);
            reader.rewind();
        }
        let check = |data: &[u8], poc| {
            let wanted: Vec<_> = (0..3)
                .flat_map(|c| expected(5, poc, c))
                .map(|v| v as u8)
                .collect();
            assert_eq!(data.len(), wanted.len());
            for (i, (&a, &b)) in data.iter().zip(&wanted).enumerate() {
                assert_eq!(a, b, "{name}: packed POC{poc} index{i}");
            }
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
            assert_eq!(seen, [0, 1]);
            common.rewind();
        }
        let mut native = NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
        for target in [40, 0, 40, 40] {
            let RawFrame::Planar(frame) = native
                .seek_raw(Duration::from_millis(target))
                .unwrap()
                .unwrap()
            else {
                panic!("{name}: planar frame")
            };
            check(&frame.frame.data, if target == 0 { 0 } else { 1 });
        }
    }
    assert!(
        changed_selections > 0,
        "refinement must change actual source motion selection"
    );
    assert!(
        row_restarts > 0,
        "B1 across CTU rows must not provide saved skip-IV fallback"
    );
}
