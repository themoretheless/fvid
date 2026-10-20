use fvid::{
    container::mp4::Limits,
    playback::video::{
        mp4::LayeredMp4VideoReader,
        native::{NativeReader, RawFrame},
    },
};
#[test]
fn native_illumination_matches_independent_cu_boundary_and_pixel_oracle() {
    check("illumination");
}
#[test]
fn native_illumination_nonunity_gain_offset_and_saturation() {
    check("illumination-gain");
}
#[test]
fn native_illumination_fractional_interpolation_and_boundary_rounding() {
    check("illumination-fractional");
}
#[test]
fn native_illumination_vertical_and_two_dimensional_fractional_motion() {
    check("illumination-2d");
}
#[test]
fn native_illumination_merge_zero_disable_omits_flags_and_compensation() {
    check("illumination-merge-zero-p");
}
fn check(prefix: &str) {
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
    if prefix == "illumination-gain" {
        assert!(manifest["nonunity_gains"].as_u64().unwrap() > 0);
        assert_eq!(manifest["cases"].as_array().unwrap().len(), 24);
    }
    if prefix == "illumination-fractional" {
        let cases = manifest["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 88);
        let phases = cases.iter().fold(0u8, |mask, c| {
            mask | (1u8 << (c["mv_horizontal"].as_i64().unwrap() & 7))
        });
        assert_eq!(phases, 0xfe, "all nonzero chroma phases must be exercised");
        assert!(
            cases
                .iter()
                .any(|c| c["mv_horizontal"].as_i64().unwrap() < 0)
        );
    }
    if prefix == "illumination-2d" {
        let cases = manifest["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 160);
        for (axis, mask) in [("mv_horizontal", 0xff), ("mv_vertical", 0xfe)] {
            let actual = cases
                .iter()
                .fold(0u8, |m, c| m | (1u8 << (c[axis].as_i64().unwrap() & 7)));
            assert_eq!(actual, mask, "{axis}: eighth-sample phase coverage");
        }
    }
    if prefix == "illumination-merge-zero-p" {
        assert_eq!(manifest["cases"].as_array().unwrap().len(), 32);
        assert!(manifest["suppression_witnesses"].as_u64().unwrap() > 0);
    }
    let mut changed = 0;
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let gain: [i64; 3] = case["target_luma"]
            .as_array()
            .map(|a| std::array::from_fn(|i| a[i].as_i64().unwrap()))
            .unwrap_or([1, 1, 0]);
        let size = case["size"].as_u64().unwrap() as usize;
        changed += case["changed_samples"].as_u64().unwrap();
        let data = std::fs::read(root.join(name)).unwrap();
        let mut reader = LayeredMp4VideoReader::open(
            std::io::Cursor::new(data.clone()),
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
                if id != 5 || poc != 1 {
                    continue;
                }
                let picture = &frame.decoded.picture;

                let mut at = case["oracle_offset"].as_u64().unwrap() as usize;
                for c in 0..3 {
                    let samples = picture.planes[c].samples();
                    for (i, (&a, &b)) in samples
                        .iter()
                        .zip(&golden[at..at + samples.len()])
                        .enumerate()
                    {
                        assert_eq!(a, b, "{name}: c{c} sample{i}");
                    }
                    at += samples.len();
                }
                for y in (0..size).step_by(32) {
                    for x in (0..size).step_by(32) {
                        let motion = picture.prediction_motion_at([x as u32, y as u32]).unwrap();
                        let temporal = size > 32 && x == 0 && y == 0;
                        assert_eq!(
                            motion[0].unwrap().reference,
                            if temporal { 0 } else { 1 },
                            "{name}: {x},{y}"
                        );
                        let horizontal = case["mv_horizontal"].as_i64().unwrap_or(0);
                        let vertical = case["mv_vertical"].as_i64().unwrap_or(0);
                        let expected_mv = if temporal {
                            [0, 0]
                        } else {
                            [horizontal, vertical]
                        };
                        assert_eq!(motion[0].unwrap().mv, expected_mv, "{name}: {x},{y}");
                        if case["mv_horizontal"].is_i64() {
                            assert_eq!(
                                picture
                                    .prediction_is_skipped_at([x as u32, y as u32])
                                    .unwrap(),
                                !(x == 32 && y == 0),
                                "{name}: explicit metadata"
                            );
                        }
                        assert!(motion[1].is_none());
                    }
                }
            }
            assert_eq!(seen, [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)]);
            reader.rewind();
        }
        let mut native = NativeReader::software(std::io::Cursor::new(data), 64 << 20).unwrap();
        for target in [40, 0, 40, 40] {
            let RawFrame::Planar(frame) = native
                .seek_raw(std::time::Duration::from_millis(target))
                .unwrap()
                .unwrap()
            else {
                panic!("{name}: planar IC frame")
            };
            let expected: Vec<u8> = if target == 0 {
                (0..3)
                    .flat_map(|c| {
                        let side = if c == 0 { size } else { size / 2 };
                        (0..side).flat_map(move |y| {
                            (0..side).map(move |x| {
                                if c == 0 {
                                    ((((3 * x + 5 * y + 17) % 256) as i64 * gain[0]) / gain[1]
                                        + gain[2])
                                        .clamp(0, 255) as u8
                                } else {
                                    ((13 * x + 7 * y + 29 + 83 * (c - 1) + [71, 113][c - 1]) % 256)
                                        as u8
                                }
                            })
                        })
                    })
                    .collect()
            } else {
                let at = case["oracle_offset"].as_u64().unwrap() as usize;
                golden[at..at + size * size * 3 / 2]
                    .iter()
                    .map(|&v| v as u8)
                    .collect()
            };
            assert_eq!(frame.frame.data, expected, "{name}: IC seek {target}ms");
        }
    }
    assert!(
        changed > 0,
        "IC must change reconstructed samples, not merely parse a flag"
    );
}
