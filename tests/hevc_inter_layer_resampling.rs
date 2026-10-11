//! Reference-sample resampling acceptance; not inter-layer picture decoding.
use fvid::codec::hevc_motion::{Motion, Vector};
use fvid::codec::hevc_resample::MotionSource;
use fvid::codec::{hevc_pps::ReferenceLocation, hevc_resample::Resampling, hevc_sps::Sps};
use std::collections::BTreeSet;
fn motions(v: &serde_json::Value) -> Vec<Motion> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            std::array::from_fn(|c| {
                if entry[c].is_null() {
                    None
                } else {
                    Some(Vector {
                        reference: entry[c]["reference"].as_u64().unwrap() as u8,
                        mv: array::<2>(&entry[c]["mv"]).map(|v| v as i16),
                    })
                }
            })
        })
        .collect()
}
fn motion_json(m: &[Motion]) -> serde_json::Value {
    serde_json::Value::Array(
        m.iter()
            .map(|entry| {
                serde_json::Value::Array(
                    entry
                        .iter()
                        .map(|v| match v {
                            None => serde_json::Value::Null,
                            Some(v) => serde_json::json!({"reference":v.reference,"mv":v.mv}),
                        })
                        .collect(),
                )
            })
            .collect(),
    )
}

fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn array<const N: usize>(v: &serde_json::Value) -> [i64; N] {
    std::array::from_fn(|i| v[i].as_i64().unwrap())
}
fn video(case: &serde_json::Value, reference: &Sps) -> Vec<[Vec<u16>; 3]> {
    let name = case["prefix"].as_str().unwrap();
    let [rw, rh] = reference.dimensions.map(|v| v as usize);
    let rs = match reference.chroma_format {
        1 => [2, 2],
        2 => [2, 1],
        _ => [1, 1],
    };
    let counts = [rw * rh, rw * rh / rs[0] / rs[1], rw * rh / rs[0] / rs[1]];
    let bytes = data(&format!("{name}.y4m"));
    let mut at = bytes.iter().position(|&v| v == b'\n').unwrap() + 1;
    let storage_depth = *reference.depth.iter().max().unwrap();
    let suffix = format!(
        "C{}{}\n",
        match reference.chroma_format {
            1 => "420",
            2 => "422",
            _ => "444",
        },
        if storage_depth == 8 {
            String::new()
        } else {
            format!("p{storage_depth}")
        }
    );
    assert!(
        std::str::from_utf8(&bytes[..at])
            .unwrap()
            .ends_with(&suffix)
    );
    let mut frames = Vec::new();
    for frame in case["frames"].as_array().unwrap() {
        assert_eq!(&bytes[at..at + 6], b"FRAME\n");
        at += 6;
        let source: [Vec<u16>; 3] = std::array::from_fn(|c| {
            (0..counts[c])
                .map(|_| {
                    let sample = if storage_depth == 8 {
                        u16::from(bytes[at])
                    } else {
                        u16::from_le_bytes([bytes[at], bytes[at + 1]])
                    };
                    at += if storage_depth == 8 { 1 } else { 2 };
                    sample
                })
                .collect()
        });
        assert_eq!(serde_json::to_value(&source).unwrap(), frame["input"]);
        frames.push(source);
    }
    assert_eq!(at, bytes.len());
    frames
}
#[test]
fn inter_layer_resampling_matches_coordinates_planes_and_fixed_point_oracles() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&data("hevc-inter-layer-resampling.json")).unwrap();
    let base = Sps::parse(&data("hevc-layered-sps-active-base.sps"), 1 << 20).unwrap();
    let mut coverage: [BTreeSet<i64>; 2] = std::array::from_fn(|_| BTreeSet::new());
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["prefix"].as_str().unwrap();
        let parameters = |prefix: &str| {
            let mut s = base.clone();
            s.dimensions = array::<2>(&case[format!("{prefix}_size")]).map(|v| v as u32);
            s.depth = array::<2>(&case[format!("{prefix}_depth")]).map(|v| v as u8);
            s.chroma_format = case[format!("{prefix}_format")].as_u64().unwrap() as u8;
            s.separate_colour_plane = false;
            s
        };
        let reference = parameters("reference");
        let current = parameters("current");
        let location = ReferenceLocation {
            layer_id: 0,
            scaled_offsets: Some(array::<4>(&case["scaled"]).map(|v| v as i32)),
            region_offsets: Some(array::<4>(&case["region"]).map(|v| v as i32)),
            phases: if case["phases"].is_null() {
                None
            } else {
                Some(array::<4>(&case["phases"]).map(|v| v as u32))
            },
        };
        let frames = video(case, &reference);
        let result = Resampling::new(&current, &reference, &location);
        if let Some(message) = case["error"].as_str() {
            let e = result.unwrap_err();
            assert!(matches!(e, fvid::Error::Invalid(_)), "{name}: {e}");
            assert!(e.to_string().contains(message), "{name}: {e}");
            continue;
        }
        let operation = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        let sub = |f: u8| match f {
            1 => [2, 2],
            2 => [2, 1],
            _ => [1, 1],
        };
        let cs = sub(current.chroma_format);
        let [w, h] = current.dimensions.map(|v| v as usize);
        let sizes = [[w, h], [w / cs[0], h / cs[1]], [w / cs[0], h / cs[1]]];
        for chroma in [false, true] {
            let c = usize::from(chroma);
            for y in 0..sizes[c][1] {
                for x in 0..sizes[c][0] {
                    let p = operation.coordinate(chroma, [x, y]).unwrap();
                    assert_eq!(
                        p,
                        [
                            case["coordinates"][c][0][x].as_i64().unwrap(),
                            case["coordinates"][c][1][y].as_i64().unwrap()
                        ],
                        "{name} {chroma}/{x}/{y}"
                    );
                    coverage[c].extend(p.map(|v| v & 15));
                }
            }
            assert!(operation.coordinate(chroma, [sizes[c][0], 0]).is_err());
        }
        for (frame, source) in case["frames"].as_array().unwrap().iter().zip(frames) {
            let motion = motions(&frame["motion_input"]);
            let motion_snapshot = motion.clone();
            let mapped_motion = operation
                .motion(MotionSource::Grid(&motion), 1 << 20)
                .unwrap();
            let intra = operation.motion(MotionSource::Intra, 1 << 20).unwrap();
            assert!(intra.iter().all(|m| *m == [None; 2]));
            assert_eq!(intra.len(), mapped_motion.len());
            assert!(operation.motion(MotionSource::Grid(&[]), 1 << 20).is_err());
            if motion.iter().all(|m| *m == [None; 2]) {
                assert_eq!(intra, mapped_motion);
            }

            assert_eq!(
                motion_json(&mapped_motion),
                frame["motion_output"],
                "{name} motion"
            );
            assert_eq!(
                operation
                    .motion(MotionSource::Grid(&motion), 1 << 20)
                    .unwrap(),
                mapped_motion
            );
            assert_eq!(motion, motion_snapshot);
            assert!(
                operation
                    .motion(MotionSource::Grid(&motion), 0)
                    .unwrap_err()
                    .to_string()
                    .contains("budget")
            );
            assert!(
                operation
                    .motion(MotionSource::Grid(&motion[1..]), 1 << 20)
                    .is_err()
            );
            if name.ends_with("motion-noninteger") {
                let mut other = location.clone();
                other.phases = Some([1, 2, 8, 8]);
                assert_eq!(
                    Resampling::new(&current, &reference, &other)
                        .unwrap()
                        .motion(MotionSource::Grid(&motion), 1 << 20)
                        .unwrap(),
                    mapped_motion
                );
            }
            let snapshot = source.clone();
            let input = || std::array::from_fn(|c| source[c].as_slice());
            let output = operation
                .picture(input(), 1 << 20)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let replay = operation.picture(input(), 1 << 20).unwrap();
            for c in 0..3 {
                assert!(output[c].complete());
                assert_eq!(output[c].dimensions(), sizes[c]);
                assert_eq!(
                    serde_json::to_value(output[c].samples()).unwrap(),
                    frame["output"][c],
                    "{name} component {c}"
                );
                assert_eq!(output[c].samples(), replay[c].samples());
            }
            assert_eq!(source, snapshot);
            let insufficient = sizes.iter().map(|s| s[0] * s[1] * 3).sum::<usize>() - 1;
            assert!(
                operation
                    .picture(input(), insufficient)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("budget")
            );
            let mut short = input();
            short[1] = &source[1][1..];
            assert!(operation.picture(short, 1 << 20).is_err());
        }
    }
    for phases in coverage {
        assert_eq!(phases, (0..16).collect());
    }
}
