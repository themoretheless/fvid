//! Colored mixed-role B depth-refinement acceptance, offline authored pixels.
use fvid::{
    container::mp4::Limits,
    playback::video::{
        mp4::{LayeredMp4VideoReader, Mp4VideoReader},
        native::{NativeReader, RawFrame},
    },
};
use std::{io::Cursor, path::Path, time::Duration};
#[test]
fn colored_b_refinement_matches_both_reconstructed_views_weights_and_software_seeks() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("texture-refinement-b-colored.json")).unwrap(),
    )
    .unwrap();
    let golden: Vec<u16> = std::fs::read(root.join("texture-refinement-b-colored.y16"))
        .unwrap()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 396);
    assert!(manifest["rounding_witnesses"].as_u64().unwrap() > 0);
    let mut phases = [false; 8];
    let mut distinct = 0;
    for case in cases {
        let name = case["file"].as_str().unwrap();
        let enabled = case["enabled"].as_bool().unwrap();
        let disparity = if enabled {
            (case["scale"].as_i64().unwrap() * 254 + 256) >> 9
        } else {
            0
        };
        assert_eq!(disparity, case["disparity"].as_i64().unwrap());
        phases[disparity.rem_euclid(8) as usize] = true;
        let source_offset = case["offsets"][0].as_u64().unwrap() as usize;
        let target_offset = case["offsets"][1].as_u64().unwrap() as usize;
        let source = &golden[source_offset..source_offset + 1536];
        let target = &golden[target_offset..target_offset + 1536];
        distinct += usize::from(source[1024..] != target[1024..]);
        let expected = |id: u8, poc: i64, component: usize| -> Vec<u16> {
            let side = if component == 0 { 32 } else { 16 };
            if poc == 1 {
                let offset = [0, 1024, 1280][component];
                return (if id == 0 { source } else { target })[offset..offset + side * side]
                    .to_vec();
            }
            let mut values = Vec::new();
            for y in 0..side {
                for x in 0..side {
                    let future = usize::from(poc == 2);
                    let value = match component {
                        0 => 3 * x + 5 * y + 17 + 97 * future,
                        1 => 13 * x + 7 * y + 29 + if id == 5 { 71 } else { 0 } + 97 * future,
                        _ => 17 * x + 11 * y + 83 + if id == 5 { 113 } else { 0 } + 194 * future,
                    };
                    values.push((value % 256) as u16);
                }
            }
            values
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
                if id == 5 {
                    assert_eq!(
                        frame.decoded.binding.pps.pps.weighted_biprediction,
                        case["weights"].as_str() != Some("none")
                    );
                }
                for component in 0..3 {
                    assert_eq!(
                        frame.decoded.picture.planes[component].samples(),
                        expected(id, poc, component),
                        "{name} layer{id} POC{poc} c{component}"
                    );
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
        let check = |data: &[u8], poc| {
            let wanted: Vec<u8> = (0..3)
                .flat_map(|c| expected(5, poc, c))
                .map(|v| v as u8)
                .collect();
            assert_eq!(data, wanted, "{name} packed POC{poc}");
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
                .seek_raw(Duration::from_millis(target_ms))
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
        phases.into_iter().all(|v| v),
        "all eighth-chroma phases required"
    );
    assert!(
        distinct > 0,
        "copying source colors must not pass target IV/weights"
    );
}
