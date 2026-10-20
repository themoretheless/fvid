//! Colored variable-depth refinement across CU boundaries and worker ownership.
use fvid::{
    container::mp4::Limits,
    playback::video::{
        mp4::{LayeredMp4VideoReader, Mp4VideoReader},
        native::{NativeReader, RawFrame},
    },
};
use std::{io::Cursor, path::Path, time::Duration};
#[test]
fn multicu_color_refinement_matches_all_planes_phases_owning_views_and_repeated_seek() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("multicu-refinement-colored.json")).unwrap(),
    )
    .unwrap();
    let golden: Vec<u16> = std::fs::read(root.join("multicu-refinement-colored.y16"))
        .unwrap()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 32);
    let mut phases = [false; 8];
    let mut distinct = 0;
    for case in cases {
        let name = case["file"].as_str().unwrap();
        let size = case["size"].as_u64().unwrap() as usize;
        let side = size / 2;
        let at = case["oracle_offset"].as_u64().unwrap() as usize;
        let vectors: Vec<i64> = case["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["vector"].as_i64().unwrap())
            .collect();
        for &v in &vectors {
            phases[v.rem_euclid(8) as usize] = true;
        }
        let expected = |id: u8, poc: i64, component: usize| -> Vec<u16> {
            let n = if component == 0 { size } else { side };
            if id == 5 && poc == 1 {
                let start = at + [0, size * size, size * size + side * side][component];
                return golden[start..start + n * n].to_vec();
            }
            (0..n)
                .flat_map(|y| {
                    (0..n).map(move |x| {
                        let value = if component == 0 {
                            3 * x + 5 * y + 17
                        } else {
                            13 * x
                                + 7 * y
                                + 29
                                + 83 * (component - 1)
                                + if id == 5 { [71, 113][component - 1] } else { 0 }
                        };
                        (value % 256) as u16
                    })
                })
                .collect()
        };
        distinct += usize::from(expected(5, 1, 1) != expected(5, 0, 1));
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
                assert_eq!(
                    frame.decoded.binding.pps.pps.constrained_intra,
                    case["direct"].as_bool().unwrap()
                );
                if id == 3 {
                    continue;
                }
                for component in 0..3 {
                    let wanted = expected(id, poc, component);
                    let n = if component == 0 { size } else { side };
                    assert_eq!(frame.decoded.picture.planes[component].dimensions(), [n, n]);
                    assert_eq!(
                        frame.decoded.picture.planes[component].samples().len(),
                        wanted.len()
                    );
                    for (i, (&a, &b)) in frame.decoded.picture.planes[component]
                        .samples()
                        .iter()
                        .zip(&wanted)
                        .enumerate()
                    {
                        assert_eq!(a, b, "{name} layer{id} POC{poc} c{component} index{i}");
                    }
                }
                if id == 5 && poc == 1 {
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
                                [vectors[y / 32 * (size / 32) + x / 32], 0]
                            );
                            assert!(motion[1].is_none());
                        }
                    }
                }
            }
            assert_eq!(seen, [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)]);
            reader.rewind();
        }
        let check = |data: &[u8], poc| {
            let wanted: Vec<u8> = (0..3)
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
        phases.into_iter().all(|v| v),
        "all eighth-chroma phases required"
    );
    assert!(distinct > 0);
}
