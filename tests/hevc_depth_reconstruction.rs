//! Native reconstruction primitives, not complete 3D entropy/picture acceptance.
use fvid::codec::{bits::BitReader, hevc_depth_tables::Tables};
use std::path::{Path, PathBuf};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(
        &std::fs::read(root().join("hevc-depth-reconstruction/manifest.json")).unwrap(),
    )
    .unwrap()
}
fn tables(c: &serde_json::Value) -> Tables {
    let data = std::fs::read(root().join(format!(
        "hevc-depth-tables/{}.bin",
        c["source"].as_str().unwrap()
    )))
    .unwrap();
    let ids: Vec<u8> = c["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u8)
        .collect();
    Tables::read(
        &mut BitReader::new(&data),
        c["bits"].as_u64().unwrap() as usize,
        c["depth"].as_u64().unwrap() as u8,
        &ids,
        1 << 20,
    )
    .unwrap()
    .unwrap()
}
fn values(v: &serde_json::Value) -> Vec<u16> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u16)
        .collect()
}
#[test]
fn full_depth_domains_match_independent_nearest_and_zero_padding() {
    for c in manifest()["cases"].as_array().unwrap() {
        let tables = tables(c);
        let name = c["name"].as_str().unwrap();
        let layer = c["layer"].as_u64().unwrap() as u8;
        let lookup =
            std::fs::read(root().join(format!("hevc-depth-reconstruction/{name}-lookup.bin")))
                .unwrap();
        for (i, b) in lookup.chunks_exact(2).enumerate() {
            assert_eq!(
                tables.reconstruction_value(layer, i).unwrap(),
                u16::from_le_bytes(b.try_into().unwrap()),
                "{name}/{i}"
            );
        }
        assert!(
            tables
                .reconstruction_value(layer, 1usize << tables.bit_depth)
                .is_err()
        );
        if c["inverse"].as_bool().unwrap() {
            let inverse =
                std::fs::read(root().join(format!("hevc-depth-reconstruction/{name}-inverse.bin")))
                    .unwrap();
            for (i, b) in inverse.chunks_exact(2).enumerate() {
                assert_eq!(
                    tables.prediction_index(layer, i as u16).unwrap(),
                    u16::from_le_bytes(b.try_into().unwrap()) as usize,
                    "{name}/{i}"
                );
            }
            for p in c["predictions"].as_array().unwrap() {
                assert_eq!(
                    tables
                        .dc_prediction(
                            layer,
                            p["sample"].as_u64().unwrap() as u16,
                            p["offset"].as_i64().unwrap()
                        )
                        .unwrap(),
                    p["expected"].as_str().unwrap().parse::<i128>().unwrap(),
                    "{name}"
                );
            }
        } else {
            assert!(
                tables
                    .prediction_index(layer, 0)
                    .unwrap_err()
                    .to_string()
                    .contains("empty depth lookup")
            );
        }
    }
}
#[test]
fn dc_assignment_matches_scalar_blocks_and_preserves_padding() {
    for c in manifest()["cases"].as_array().unwrap() {
        let tables = tables(c);
        let layer = c["layer"].as_u64().unwrap() as u8;
        for b in c["blocks"].as_array().unwrap() {
            let input = values(&b["input"]);
            let expected = values(&b["expected"]);
            let mut buffer = vec![0u16; 64];
            for y in 0..4 {
                buffer[(y + 1) * 8 + 2..(y + 1) * 8 + 6].copy_from_slice(&input[y * 4..y * 4 + 4]);
            }
            let before = buffer.clone();
            tables
                .assign_dc_offset(
                    layer,
                    &mut buffer,
                    8,
                    [2, 1],
                    4,
                    b["offset"].as_i64().unwrap(),
                )
                .unwrap();
            for y in 0..8 {
                for x in 0..8 {
                    let target = if (1..5).contains(&y) && (2..6).contains(&x) {
                        expected[(y - 1) * 4 + x - 2]
                    } else {
                        before[y * 8 + x]
                    };
                    assert_eq!(buffer[y * 8 + x], target, "{} x{x} y{y}", c["name"]);
                }
            }
        }
    }
}
#[test]
fn malformed_blocks_and_empty_tables_fail_before_mutation() {
    let m = manifest();
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["source"] == "variable")
        .unwrap();
    let t = tables(c);
    let layer = c["layer"].as_u64().unwrap() as u8;
    for (stride, origin, size) in [
        (8, [6, 0], 4),
        (8, [0, 6], 4),
        (8, [usize::MAX, 0], 4),
        (usize::MAX, [0, 1], 4),
        (8, [0, 0], 0),
        (8, [0, 0], 3),
    ] {
        let mut data = vec![7; 64];
        let before = data.clone();
        assert!(
            t.assign_dc_offset(layer, &mut data, stride, origin, size, 1)
                .is_err()
        );
        assert_eq!(data, before);
    }
    let mut data = vec![7; 16];
    data[14] = 256;
    let before = data.clone();
    assert!(
        t.assign_dc_offset(layer, &mut data, 4, [0, 0], 4, 1)
            .is_err()
    );
    assert_eq!(data, before);
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["source"] == "empty")
        .unwrap();
    let t = tables(c);
    let layer = c["layer"].as_u64().unwrap() as u8;
    let mut data = vec![7; 16];
    let before = data.clone();
    assert!(
        t.assign_dc_offset(layer, &mut data, 4, [0, 0], 4, 1)
            .is_err()
    );
    assert_eq!(data, before);
}
