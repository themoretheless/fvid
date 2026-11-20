use fvid_media::owned_aac::{aac_ep::Config, config::AudioSpecificConfig};
use serde_json::Value;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn bits(raw: &[u8]) -> Vec<u8> {
    raw.iter()
        .flat_map(|&b| (0..8).rev().map(move |i| (b >> i) & 1))
        .collect()
}
fn packet<'a>(blob: &'a [u8], row: &Value) -> &'a [u8] {
    let a = row["offset"].as_u64().unwrap() as usize;
    &blob[a..a + row["bytes"].as_u64().unwrap() as usize]
}
#[test]
fn exact_class_bits_keep_non_octet_lengths_and_present_empty_identity() {
    let m: Value = serde_json::from_slice(
        &std::fs::read(root().join("aac-ep-class-boundaries.json")).unwrap(),
    )
    .unwrap();
    for c in m["vectors"].as_array().unwrap() {
        let (ep, _) = Config::read_prefix(&hex(c["config"].as_str().unwrap()), 0).unwrap();
        let raw = hex(c["packet"].as_str().unwrap());
        let frames = ep
            .decode_class_frames(&raw)
            .unwrap_or_else(|e| panic!("{c}: {e}"));
        assert_eq!(frames.len(), 1);
        let frame = &frames[0];
        assert_eq!(frame.set_index, 0);
        assert_eq!(frame.classes.len(), c["classes"].as_array().unwrap().len());
        for (actual, expected) in frame.classes.iter().zip(c["classes"].as_array().unwrap()) {
            assert_eq!(
                actual.class_index,
                expected["class_index"].as_u64().unwrap() as usize
            );
            assert_eq!(
                actual.bits,
                expected["bits"]
                    .as_str()
                    .unwrap()
                    .bytes()
                    .map(|b| b - b'0')
                    .collect::<Vec<_>>()
            );
        }
        let total = c["total"].as_u64().unwrap() as usize;
        if total % 8 != 0 {
            assert!(
                ep.decode_raw_access_unit(&raw)
                    .unwrap_err()
                    .to_string()
                    .contains("not octet aligned")
            );
        } else {
            let output = ep.decode_raw_access_unit(&raw).unwrap();
            assert_eq!(
                bits(&output),
                frame
                    .classes
                    .iter()
                    .flat_map(|c| c.bits.clone())
                    .collect::<Vec<_>>()
            );
        }
    }
}
#[test]
fn class_boundaries_and_indices_survive_all_multi_source_movie_protection_modes() {
    let dir = root().join("aac-ep-concatenated");
    let m: Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let blob = std::fs::read(dir.join("packets.bin")).unwrap();
    let plain = std::fs::read(dir.join("plain-packets.bin")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let ep = AudioSpecificConfig::parse(&hex(c["asc"].as_str().unwrap()))
            .unwrap()
            .protection
            .unwrap();
        for row in c["frames"].as_array().unwrap() {
            let frames = ep
                .decode_class_frames(packet(&blob, row))
                .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
            assert_eq!(frames.len(), c["source_frames"].as_u64().unwrap() as usize);
            let choice = row["choice"].as_u64().unwrap() as usize;
            let expected = &c["configs"][choice]["classes"];
            for (frame, source) in frames.iter().zip(row["units"].as_array().unwrap()) {
                assert_eq!(frame.set_index, choice);
                assert_eq!(frame.classes.len(), expected.as_array().unwrap().len());
                let mut at = 0;
                let source = bits(packet(&plain, source));
                for (index, (class, descriptor)) in frame
                    .classes
                    .iter()
                    .zip(expected.as_array().unwrap())
                    .enumerate()
                {
                    let len = descriptor["length"].as_u64().unwrap() as usize;
                    assert_eq!(class.class_index, index);
                    assert_eq!(
                        class.bits,
                        source[at..at + len],
                        "{} class {index}",
                        c["name"]
                    );
                    at += len;
                }
                assert_eq!(at, source.len());
            }
        }
    }
}

#[test]
fn optional_and_grouped_classes_preserve_identity_and_original_information() {
    for name in ["aac-ep-optional", "aac-ep-rs-groups", "aac-ep-multiclass"] {
        let dir = root().join(name);
        let m: Value =
            serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let blob = std::fs::read(dir.join("packets.bin")).unwrap();
        let plain = std::fs::read(dir.join("plain-packets.bin")).unwrap();
        for c in m["cases"].as_array().unwrap() {
            let ep = AudioSpecificConfig::parse(&hex(c["asc"].as_str().unwrap()))
                .unwrap()
                .protection
                .unwrap();
            for row in c["frames"].as_array().unwrap() {
                let frames = ep
                    .decode_class_frames(packet(&blob, row))
                    .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
                assert_eq!(frames.len(), 1);
                let frame = &frames[0];
                let choice = row["choice"].as_u64().unwrap() as usize;
                assert_eq!(frame.set_index, choice);
                let present = row["present"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|v| v.as_u64().unwrap() as usize)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_else(|| (0..ep.sets[choice].classes.len()).collect());
                assert_eq!(
                    frame
                        .classes
                        .iter()
                        .map(|c| c.class_index)
                        .collect::<Vec<_>>(),
                    present,
                    "{}",
                    c["name"]
                );
                let a = row["plain_offset"].as_u64().unwrap() as usize;
                let b = a + row["plain_bytes"].as_u64().unwrap() as usize;
                assert_eq!(
                    frame
                        .classes
                        .iter()
                        .flat_map(|c| c.bits.clone())
                        .collect::<Vec<_>>(),
                    bits(&plain[a..b]),
                    "{}",
                    c["name"]
                );
                for class in &frame.classes {
                    if let fvid_media::owned_aac::aac_ep::Length::Fixed(n) =
                        ep.sets[choice].classes[class.class_index].length
                    {
                        assert_eq!(
                            class.bits.len(),
                            n as usize,
                            "{} class {}",
                            c["name"],
                            class.class_index
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn class_recovery_never_exposes_partial_results_on_protection_failure() {
    let mut checked = 0;
    for name in [
        "aac-ep-concatenated",
        "aac-ep-optional",
        "aac-ep-rs-groups",
        "aac-ep-multiclass",
        "aac-ep-ps",
    ] {
        let dir = root().join(name);
        let m: Value =
            serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let blob = std::fs::read(dir.join("packets.bin")).unwrap();
        for c in m["malformed"].as_array().unwrap() {
            let expected = c["error"].as_str().unwrap();
            if !expected.starts_with("EP ") {
                continue;
            }
            let ep = AudioSpecificConfig::parse(&hex(c["asc"].as_str().unwrap()))
                .unwrap()
                .protection
                .unwrap();
            let index = c["bad_index"].as_u64().unwrap_or(3) as usize;
            let error = ep
                .decode_class_frames(packet(&blob, &c["frames"][index]))
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{}: {error}", c["name"]);
            checked += 1;
        }
    }
    assert!(checked > 100);
}
