use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
use std::io::Cursor;
#[test]
fn native_residual_weights_reconstruct_coded_temporal_and_inter_view_cus() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("residual-p.json")).unwrap()).unwrap();
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 96);
    let mut clipped = 0usize;
    let mut negative_difference = 0usize;
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let size = case["size"].as_u64().unwrap() as usize;
        let bytes = std::fs::read(root.join(name)).unwrap();
        let mut reader =
            LayeredMp4VideoReader::open(Cursor::new(bytes), Limits::default(), 1, 0, 64 << 20)
                .unwrap();
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                let decoded = &frame.decoded;
                seen.push((decoded.layer_id, decoded.poc));
                if decoded.layer_id != 5 || decoded.poc != 1 {
                    continue;
                }
                for component in 0..3 {
                    let side = if component == 0 { size } else { size / 2 };
                    for y in 0..side {
                        for x in 0..side {
                            let (lx, ly) = if component == 0 {
                                (x, y)
                            } else {
                                (x * 2, y * 2)
                            };
                            let weight = case["weights"][(ly / 32) * (size / 32) + lx / 32]
                                .as_u64()
                                .unwrap();
                            let temporal = ly >= 32 || lx < 32;
                            let source = if component == 0 {
                                ((3 * x + 5 * y + 17) % 256) as i32
                            } else {
                                ((13 * x + 7 * y + 29 + 83 * (component - 1)) % 256) as i32
                            };
                            let current = if component == 0 {
                                (source + case["source_bias"].as_i64().unwrap() as i32)
                                    .clamp(0, 255)
                            } else {
                                source
                            };
                            let own = if component == 0 {
                                source / 2 + 30
                            } else {
                                (source + [71, 113][component - 1]) % 256
                            };
                            let expected = if weight == 0 {
                                if temporal { own } else { current }
                            } else {
                                {
                                    let (base, difference) = if temporal {
                                        (own, current - source)
                                    } else {
                                        (current, own - source)
                                    };
                                    (base * 64 + ((difference * 64) >> (weight - 1)) + 32) >> 6
                                }
                            };
                            clipped += usize::from(expected < 0 || expected > 255);
                            negative_difference += usize::from(weight != 0 && current < source);
                            assert_eq!(
                                decoded.picture.planes[component].samples()[y * side + x],
                                expected.clamp(0, 255) as u16,
                                "{name} c{component} {x},{y} weight{weight}"
                            );
                        }
                    }
                }
            }
            assert_eq!(
                seen,
                [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)],
                "{name}"
            );
            reader.rewind();
        }
    }
    assert!(
        clipped > 0,
        "coded residual output must exercise final clipping"
    );
    assert!(
        negative_difference > 0,
        "coded residual must exercise negative source differences"
    );
}

#[test]
fn coded_fractional_residual_matches_two_tap_oracle_and_saved_motion() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("residual-fractional.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 528);
    assert!(manifest["different_from_eight_tap"].as_u64().unwrap() > 0);
    assert!(manifest["half_precision_witnesses"].as_u64().unwrap() > 0);
    let golden: Vec<u16> = std::fs::read(root.join("residual-fractional.y16"))
        .unwrap()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let size = case["size"].as_u64().unwrap() as usize;
        let mv = [
            case["mv"][0].as_i64().unwrap(),
            case["mv"][1].as_i64().unwrap(),
        ];
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
                let decoded = &frame.decoded;
                seen.push((decoded.layer_id, decoded.poc));
                if decoded.layer_id != 5 || decoded.poc != 1 {
                    continue;
                }
                let mut at = case["oracle_offset"].as_u64().unwrap() as usize;
                for component in 0..3 {
                    let samples = decoded.picture.planes[component].samples();
                    for (i, (&actual, &expected)) in samples
                        .iter()
                        .zip(&golden[at..at + samples.len()])
                        .enumerate()
                    {
                        assert_eq!(actual, expected, "{name} c{component} sample{i}");
                    }
                    at += samples.len();
                }
                for y in (0..size).step_by(32) {
                    for x in (0..size).step_by(32) {
                        let vector = decoded
                            .picture
                            .prediction_motion_at([x as u32, y as u32])
                            .unwrap()[0]
                            .unwrap();
                        let first = x == 0 && y == 0;
                        assert_eq!(
                            vector.reference,
                            if first {
                                0
                            } else {
                                case["reference"].as_u64().unwrap() as u8
                            },
                            "{name} {x},{y}: reference"
                        );
                        assert_eq!(
                            vector.mv,
                            if first { [0, 0] } else { mv },
                            "{name} {x},{y}: motion"
                        );
                    }
                }
            }
            assert_eq!(
                seen,
                [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)],
                "{name}"
            );
            reader.rewind();
        }
    }
}
