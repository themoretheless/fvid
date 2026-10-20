use fvid::{
    container::mp4::Limits,
    playback::video::{
        mp4::LayeredMp4VideoReader,
        native::{NativeReader, RawFrame},
    },
};
use std::{io::Cursor, time::Duration};
#[test]
fn coded_b_residual_lists_preserve_internal_precision_pixels_motion_reorder_and_seek() {
    check("residual-b", 224, false);
}
#[test]
fn fractional_b_temporal_replacement_scales_motion_before_residual_and_explicit_weights() {
    check("residual-b-temporal-fractional", 96, true);
}
#[test]
fn mixed_fractional_b_residual_uses_spatial_disparity_and_opposite_temporal_motion() {
    check("residual-b-mixed-fractional", 240, true);
}
#[test]
fn coded_source_amp_motion_replaces_zero_fallback_and_prefers_l0_for_l1_targets() {
    check("residual-b-reference-motion", 288, true);
}
#[test]
fn distinct_neighbour_motion_uses_a1_before_b1_for_residual_disparity() {
    check("residual-b-variable-neighbour", 64, true);
}
fn check(prefix: &str, count: usize, fractional: bool) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join(format!("{prefix}.json"))).unwrap())
            .unwrap();
    assert_eq!(manifest["cases"].as_array().unwrap().len(), count);
    if fractional {
        let witness = if prefix.contains("variable-neighbour") {
            "priority_witnesses"
        } else if prefix.contains("reference-motion") {
            "zero_witnesses"
        } else if prefix.contains("mixed") {
            "disparity_witnesses"
        } else {
            "scaling_witnesses"
        };
        assert!(manifest[witness].as_u64().unwrap() > 0);
        if prefix.contains("reference-motion") {
            assert!(manifest["l0_witnesses"].as_u64().unwrap() > 0);
            assert!(
                manifest["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|c| c["mode"] == "l1")
                    .map(|c| c["l0_witnesses"].as_u64().unwrap())
                    .sum::<u64>()
                    > 0,
                "L0 source motion must remain preferred for a target with only L1 active"
            );
        }
        if prefix.contains("mixed") {
            assert!(
                manifest["b1_witnesses"].as_u64().unwrap() > 0,
                "B1-only CUs across CTU rows must distinguish spatial disparity from zero"
            );
        }
        for weighted in [false, true] {
            assert!(
                manifest["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|c| c["weighted"] == weighted)
                    .map(|c| c[witness].as_u64().unwrap())
                    .sum::<u64>()
                    > 0,
                "both default and explicit weighting must distinguish {witness}"
            );
        }
    } else {
        assert!(manifest["late_combination_witnesses"].as_u64().unwrap() > 0);
        assert!(
            manifest["original_weight_index_witnesses"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert!(
            manifest["cases"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|c| c["mode"] == "future-past" && c["weighted"] == true)
                .map(|c| c["original_weight_index_witnesses"].as_u64().unwrap())
                .sum::<u64>()
                > 0,
            "temporal base replacement must distinguish original weight indices from target indices"
        );
    }
    let golden: Vec<u16> = std::fs::read(root.join(format!("{prefix}.y16")))
        .unwrap()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let size = case["size"].as_u64().unwrap() as usize;
        let data = std::fs::read(root.join(name)).unwrap();
        let expected = |poc: i32| -> Vec<u16> {
            if poc == 1 {
                let at = case["oracle_offset"].as_u64().unwrap() as usize;
                return golden[at..at + size * size * 3 / 2].to_vec();
            }
            let bias = if poc == 2 { 97 } else { 0 };
            let mut pixels = Vec::new();
            for c in 0..3 {
                let side = if c == 0 { size } else { size / 2 };
                for y in 0..side {
                    for x in 0..side {
                        let value = if c == 0 {
                            ((((3 * x + 5 * y + 17 + bias) % 256) as i32) * 2 - 40).clamp(0, 255)
                                as u16
                        } else {
                            ((13 * x + 7 * y + 29 + 83 * (c - 1) + [71, 113][c - 1] + bias) % 256)
                                as u16
                        };
                        pixels.push(value);
                    }
                }
            }
            pixels
        };
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
            let mut source_seen = false;
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                let decoded = &frame.decoded;
                if prefix.contains("reference-motion") && decoded.layer_id == 0 && decoded.poc == 1
                {
                    source_seen = true;
                    let mut at = case["source_oracle_offset"].as_u64().unwrap() as usize;
                    for c in 0..3 {
                        let samples = decoded.picture.planes[c].samples();
                        for (i, (&a, &b)) in samples
                            .iter()
                            .zip(&golden[at..at + samples.len()])
                            .enumerate()
                        {
                            assert_eq!(a, b, "{name}: source c{c} sample{i}");
                        }
                        at += samples.len();
                    }
                    for y in (0..size).step_by(8) {
                        for x in (0..size).step_by(8) {
                            let sign = if x % 32 < case["cut"].as_u64().unwrap() as usize {
                                1
                            } else {
                                -1
                            };
                            let motion = decoded
                                .picture
                                .prediction_motion_at([x as u32, y as u32])
                                .unwrap();
                            for list in 0..2 {
                                let vector = motion[list].unwrap();
                                assert_eq!(vector.reference, 0, "{name}: source reference");
                                assert_eq!(
                                    vector.mv,
                                    [0, if list == 0 { 4 * sign } else { -8 * sign }],
                                    "{name}: source motion {x},{y} list{list}"
                                );
                            }
                        }
                    }
                }
                if decoded.layer_id != 5 {
                    continue;
                }
                seen.push(decoded.poc);
                let wanted = expected(decoded.poc);
                let mut at = 0;
                for c in 0..3 {
                    let samples = decoded.picture.planes[c].samples();
                    for (i, (&actual, &gold)) in samples
                        .iter()
                        .zip(&wanted[at..at + samples.len()])
                        .enumerate()
                    {
                        assert_eq!(actual, gold, "{name} POC{} c{c} sample{i}", decoded.poc);
                    }
                    at += samples.len();
                }
                if decoded.poc == 1 {
                    for y in (0..size).step_by(if fractional { 8 } else { 32 }) {
                        for x in (0..size).step_by(if fractional { 8 } else { 32 }) {
                            let motion = decoded
                                .picture
                                .prediction_motion_at([x as u32, y as u32])
                                .unwrap();
                            for list in 0..2 {
                                let index = if x < 32 && y < 32 {
                                    Some(0)
                                } else {
                                    case["references"][list].as_u64().map(|v| v as u8)
                                };
                                assert_eq!(
                                    motion[list].map(|v| v.reference),
                                    index,
                                    "{name} {x},{y} list{list}"
                                );
                                if let Some(vector) = motion[list] {
                                    let wanted = if fractional && (x >= 32 || y >= 32) {
                                        [
                                            (if y >= 32 && case["additional_mvs"].is_array() {
                                                &case["additional_mvs"]
                                            } else {
                                                &case["mvs"]
                                            })[list][0]
                                                .as_i64()
                                                .unwrap(),
                                            (if y >= 32 && case["additional_mvs"].is_array() {
                                                &case["additional_mvs"]
                                            } else {
                                                &case["mvs"]
                                            })[list][1]
                                                .as_i64()
                                                .unwrap(),
                                        ]
                                    } else {
                                        [0, 0]
                                    };
                                    assert_eq!(vector.mv, wanted, "{name} {x},{y} list{list}");
                                }
                            }
                        }
                    }
                }
            }
            assert_eq!(seen, [0, 1, 2], "{name}: presentation order");
            if prefix.contains("reference-motion") {
                assert!(source_seen, "{name}: source picture must be checked");
            }
            reader.rewind();
        }
        let mut native = NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
        for target in [40, 0, 80, 40, 40] {
            let RawFrame::Planar(frame) = native
                .seek_raw(Duration::from_millis(target))
                .unwrap_or_else(|e| panic!("{name} seek{target}: {e}"))
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
                "{name} seek{target}"
            );
        }
    }
}
