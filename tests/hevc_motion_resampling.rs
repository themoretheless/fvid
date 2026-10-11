//! Motion-only processing supports monochrome without requiring chroma arrays.
use fvid::codec::hevc_resample::MotionSource;
use fvid::codec::{
    hevc_motion::{Motion, Vector},
    hevc_pps::ReferenceLocation,
    hevc_resample::MotionResampling,
    hevc_sps::Sps,
};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
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
                        mv: std::array::from_fn(|i| entry[c]["mv"][i].as_i64().unwrap() as i16),
                    })
                }
            })
        })
        .collect()
}
#[test]
fn monochrome_motion_grid_matches_signed_scaling_and_edge_oracles() {
    let m: serde_json::Value =
        serde_json::from_slice(&data("hevc-motion-resampling.json")).unwrap();
    let base = Sps::parse(&data("hevc-layered-sps-active-base.sps"), 1 << 20).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["prefix"].as_str().unwrap();
        let params = |prefix: &str| {
            let mut s = base.clone();
            s.chroma_format = 0;
            s.dimensions =
                std::array::from_fn(|i| c[format!("{prefix}_size")][i].as_u64().unwrap() as u32);
            s.depth =
                std::array::from_fn(|i| c[format!("{prefix}_depth")][i].as_u64().unwrap() as u8);
            s
        };
        let current = params("current");
        let reference = params("reference");
        let location = ReferenceLocation {
            layer_id: 0,
            scaled_offsets: Some(std::array::from_fn(|i| {
                c["scaled"][i].as_i64().unwrap() as i32
            })),
            region_offsets: Some(std::array::from_fn(|i| {
                c["region"][i].as_i64().unwrap() as i32
            })),
            phases: None,
        };
        let transform = MotionResampling::new(&current, &reference, &location).unwrap();
        let bytes = data(&format!("{name}.y4m"));
        let mut at = bytes.iter().position(|&v| v == b'\n').unwrap() + 1;
        let depth = reference.depth[0];
        assert!(
            std::str::from_utf8(&bytes[..at])
                .unwrap()
                .contains(&format!(
                    "Cmono{}",
                    if depth == 8 {
                        String::new()
                    } else {
                        depth.to_string()
                    }
                ))
        );
        for frame in c["frames"].as_array().unwrap() {
            assert_eq!(&bytes[at..at + 6], b"FRAME\n");
            at += 6;
            let values: Vec<u16> = (0..reference.dimensions[0] * reference.dimensions[1])
                .map(|_| {
                    let v = if depth == 8 {
                        u16::from(bytes[at])
                    } else {
                        u16::from_le_bytes([bytes[at], bytes[at + 1]])
                    };
                    at += if depth == 8 { 1 } else { 2 };
                    v
                })
                .collect();
            assert_eq!(serde_json::to_value(values).unwrap(), frame["samples"]);
            let motion = motions(&frame["input"]);
            let snapshot = motion.clone();
            let output = transform
                .motion(MotionSource::Grid(&motion), 1 << 20)
                .unwrap();
            assert_eq!(output, motions(&frame["output"]), "{name}");
            assert_eq!(motion, snapshot);
            assert_eq!(
                transform
                    .motion(MotionSource::Grid(&motion), 1 << 20)
                    .unwrap(),
                output
            );
            assert!(
                transform
                    .motion(MotionSource::Grid(&motion), 0)
                    .unwrap_err()
                    .to_string()
                    .contains("budget")
            );
            assert!(
                transform
                    .motion(MotionSource::Grid(&motion[1..]), 1 << 20)
                    .is_err()
            );
            if name.ends_with("identity") {
                assert_eq!(output, motion);
            }
        }
        assert_eq!(at, bytes.len());
    }
}
