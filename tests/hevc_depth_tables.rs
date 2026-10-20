use fvid::codec::{bits::BitReader, hevc_depth_tables::Tables};
use std::path::Path;
fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-depth-tables")
}
fn cases() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
#[test]
fn authored_depth_tables_match_literal_sets_and_prediction() {
    for c in cases()["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let data = std::fs::read(root().join(format!("{name}.bin"))).unwrap();
        let mut reader = BitReader::new(&data);
        let ids: Vec<u8> = c["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        let result = Tables::read(
            &mut reader,
            c["bits"].as_u64().unwrap() as usize,
            c["depth"].as_u64().unwrap() as u8,
            &ids,
            1 << 20,
        );
        if let Some(error) = c["error"].as_str() {
            assert_eq!(result.unwrap_err().to_string(), error, "{name}");
            assert_eq!(reader.position(), 0);
            continue;
        }
        let actual = result.unwrap();
        assert_eq!(
            reader.position(),
            c["bits"].as_u64().unwrap() as usize,
            "{name}"
        );
        if c["expected"].is_null() {
            assert!(actual.is_none());
            continue;
        }
        let actual = actual.unwrap();
        for (layer, expected) in actual.layers.iter().zip(c["expected"].as_array().unwrap()) {
            let expected = expected.as_array().map(|v| {
                v.iter()
                    .map(|v| v.as_u64().unwrap() as u16)
                    .collect::<Vec<_>>()
            });
            assert_eq!(layer.values, expected, "{name}");
            if let Some(values) = expected {
                for (i, value) in values.iter().enumerate() {
                    assert_eq!(actual.value(layer.id, i).unwrap(), *value);
                }
                assert!(actual.value(layer.id, values.len()).is_err());
            } else {
                assert_eq!(actual.value(layer.id, 7).unwrap(), 7);
            }
        }
    }
}
#[test]
fn every_truncated_field_rolls_back_and_budget_refusal_is_transactional() {
    for c in cases()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["error"].is_null())
    {
        let data =
            std::fs::read(root().join(format!("{}.bin", c["name"].as_str().unwrap()))).unwrap();
        let end = c["bits"].as_u64().unwrap() as usize;
        let ids: Vec<u8> = c["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        let depth = c["depth"].as_u64().unwrap() as u8;
        let cuts: Vec<_> = if end < 1024 {
            (0..end).collect()
        } else {
            (0..32).chain(end - 32..end).collect()
        };
        for cut in cuts {
            let mut reader = BitReader::new(&data);
            assert!(
                Tables::read(&mut reader, cut, depth, &ids, 1 << 20).is_err(),
                "{} cut{cut}",
                c["name"]
            );
            assert_eq!(reader.position(), 0);
        }
        if !ids.is_empty() {
            let mut reader = BitReader::new(&data);
            assert!(
                Tables::read(&mut reader, end, depth, &ids, 0)
                    .unwrap_err()
                    .to_string()
                    .contains("budget")
            );
            assert_eq!(reader.position(), 0);
        }
    }
}
#[test]
fn active_depth_playback_remains_a_refusal_until_3d_integration() {
    use fvid::{container::mp4::Limits, playback_mp4::Mp4VideoReader};
    let file = root()
        .parent()
        .unwrap()
        .join("hevc-3d-pps-active-synthetic.mp4");
    let error = Mp4VideoReader::open_software(
        std::fs::File::open(file).unwrap(),
        Limits::default(),
        16 << 20,
    )
    .err()
    .unwrap();
    assert_eq!(
        error.to_string(),
        "HEVC active depth lookup tables are not implemented"
    );
}

#[test]
fn predicted_table_budget_counts_live_scratch_and_validates_layer_identity() {
    use fvid::codec::hevc_depth_tables::Layer;
    let m = cases();
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "predicted-xor")
        .unwrap();
    let data = std::fs::read(root().join("predicted-xor.bin")).unwrap();
    let end = c["bits"].as_u64().unwrap() as usize;
    let slots = 2 * std::mem::size_of::<Layer>();
    let mut reader = BitReader::new(&data);
    assert!(
        Tables::read(&mut reader, end, 8, &[3, 7], slots + 19)
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
    assert_eq!(reader.position(), 0);
    assert!(
        Tables::read(&mut reader, end, 8, &[3, 7], slots + 20)
            .unwrap()
            .is_some()
    );
    for ids in [[3, 3], [3, 63], [3, 64]] {
        let mut reader = BitReader::new(&data);
        assert_eq!(
            Tables::read(&mut reader, end, 8, &ids, 1 << 20)
                .unwrap_err()
                .to_string(),
            "invalid HEVC DLT depth-layer IDs"
        );
        assert_eq!(reader.position(), 0);
    }
}
#[test]
fn misaligned_fields_preserve_the_prefix_on_failure() {
    let m = cases();
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "variable")
        .unwrap();
    let data = std::fs::read(root().join("variable.bin")).unwrap();
    let end = c["bits"].as_u64().unwrap() as usize;
    let mut shifted = vec![0u8; (end + 3).div_ceil(8)];
    for i in 0..end {
        if data[i / 8] & (1 << (7 - i % 8)) != 0 {
            shifted[(i + 3) / 8] |= 1 << (7 - (i + 3) % 8);
        }
    }
    for cut in 3..end + 3 {
        let mut reader = BitReader::new(&shifted);
        reader.read(3).unwrap();
        assert!(Tables::read(&mut reader, cut, 8, &[7], 1 << 20).is_err());
        assert_eq!(reader.position(), 3);
    }
    let mut reader = BitReader::new(&shifted);
    reader.read(3).unwrap();
    let parsed = Tables::read(&mut reader, end + 3, 8, &[7], 1 << 20)
        .unwrap()
        .unwrap();
    assert_eq!(parsed.layers[0].values, Some(vec![1, 2, 4, 9]));
    assert_eq!(reader.position(), end + 3);
    assert!(parsed.value(63, 0).is_err());
}
