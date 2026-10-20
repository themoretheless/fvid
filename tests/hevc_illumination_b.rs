use fvid::{
    container::mp4::Limits,
    playback::video::{
        mp4::LayeredMp4VideoReader,
        native::{NativeReader, RawFrame},
    },
};
use std::{io::Cursor, time::Duration};
#[test]
fn b_illumination_compensates_each_active_list_before_combination() {
    check("illumination-b", 96);
}
#[test]
fn b_fractional_illumination_preserves_per_list_rounding_and_mixed_references() {
    check("illumination-b-fractional", 144);
}
#[test]
fn b_illumination_merge_zero_disable_preserves_entropy_alignment_and_gating() {
    check("illumination-merge-zero-b", 192);
}
fn check(prefix: &str, count: usize) {
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
    assert_eq!(cases.len(), count);
    if prefix == "illumination-b-fractional" {
        assert!(
            cases
                .iter()
                .map(|c| c["precision_witnesses"].as_u64().unwrap())
                .sum::<u64>()
                > 0,
            "matrix must distinguish full-precision bipred from premature per-list rounding"
        );
    }
    if prefix == "illumination-merge-zero-b" {
        assert!(manifest["suppression_witnesses"].as_u64().unwrap() > 0);
    }
    let mut changed = 0;
    for case in cases {
        let name = case["file"].as_str().unwrap();
        let size = case["size"].as_u64().unwrap() as usize;
        let at = case["oracle_offset"].as_u64().unwrap() as usize;
        changed += case["changed_samples"].as_u64().unwrap();
        if case["mode"] == "temporal" {
            assert_eq!(case["changed_samples"].as_u64().unwrap(), 0);
        }
        let gain: [i64; 3] = std::array::from_fn(|i| case["target_luma"][i].as_i64().unwrap());
        let expected = |poc: i32| -> Vec<u16> {
            if poc == 1 {
                return golden[at..at + size * size * 3 / 2].to_vec();
            }
            let bias = if poc == 2 { 97 } else { 0 };
            (0..3)
                .flat_map(|c| {
                    let side = if c == 0 { size } else { size / 2 };
                    (0..side).flat_map(move |y| {
                        (0..side).map(move |x| {
                            if c == 0 {
                                ((((3 * x + 5 * y + 17 + bias) % 256) as i64 * gain[0]) / gain[1]
                                    + gain[2])
                                    .clamp(0, 255) as u16
                            } else {
                                ((13 * x + 7 * y + 29 + 83 * (c - 1) + [71, 113][c - 1] + bias)
                                    % 256) as u16
                            }
                        })
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
                if frame.decoded.layer_id != 5 {
                    continue;
                }
                let poc = frame.decoded.poc;
                seen.push(poc);
                let picture = &frame.decoded.picture;
                let wanted = expected(poc);
                let mut position = 0;
                for c in 0..3 {
                    let samples = picture.planes[c].samples();
                    for (i, (&a, &b)) in samples
                        .iter()
                        .zip(&wanted[position..position + samples.len()])
                        .enumerate()
                    {
                        assert_eq!(a, b, "{name}: POC{poc} c{c} sample{i}");
                    }
                    position += samples.len();
                }
                if poc == 1 {
                    for y in (0..size).step_by(32) {
                        for x in (0..size).step_by(32) {
                            let motion =
                                picture.prediction_motion_at([x as u32, y as u32]).unwrap();
                            for list in 0..2 {
                                let wanted = if x == 0 && y == 0 {
                                    Some(0)
                                } else {
                                    case["references"][list].as_u64().map(|v| v as u8)
                                };
                                assert_eq!(
                                    motion[list].map(|v| v.reference),
                                    wanted,
                                    "{name}: list{list} {x},{y}"
                                );
                                if let Some(v) = motion[list] {
                                    let cross = !(x == 0 && y == 0)
                                        && wanted == Some(if list == 0 { 1 } else { 2 });
                                    let mv = if cross && case["motion"].is_array() {
                                        std::array::from_fn(|i| {
                                            case["motion"][list][i].as_i64().unwrap()
                                        })
                                    } else {
                                        [0, 0]
                                    };
                                    assert_eq!(v.mv, mv, "{name}: list{list} {x},{y}");
                                }
                            }
                        }
                    }
                }
            }
            assert_eq!(seen, [0, 1, 2], "{name}: presentation order");
            reader.rewind();
        }
        let mut native = NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
        for target in [40, 0, 80, 40, 40] {
            let RawFrame::Planar(frame) = native
                .seek_raw(Duration::from_millis(target))
                .unwrap()
                .unwrap()
            else {
                panic!("{name}: planar B seek")
            };
            assert_eq!(
                frame.frame.data,
                expected((target / 40) as i32)
                    .iter()
                    .map(|&v| v as u8)
                    .collect::<Vec<_>>(),
                "{name}: seek{target}"
            );
        }
    }
    assert!(changed > 0, "IC must change actual mixed-list samples");
}
