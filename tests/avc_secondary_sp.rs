use fvid::codec::{avc_decoder::AvcDecoder, avc_transform::switching_chroma_420};
use serde_json::Value;
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/avc-secondary-sp.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn secondary_sp_chroma_matches_normative_matrix_oracle() {
    for c in manifest()["cases"].as_array().unwrap() {
        let p = std::array::from_fn(|i| c["prediction"][i].as_u64().unwrap() as u16);
        let dc = std::array::from_fn(|i| c["dc"][i].as_i64().unwrap() as i32);
        let ac = std::array::from_fn(|b| {
            std::array::from_fn(|i| c["ac"][b][i].as_i64().unwrap() as i32)
        });
        let expected: [u16; 64] =
            std::array::from_fn(|i| c["expected"][i].as_u64().unwrap() as u16);
        assert_eq!(
            switching_chroma_420(&p, &dc, &ac, c["qs"].as_u64().unwrap() as u8).unwrap(),
            expected,
            "{c}"
        );
    }
    // Normative 8-441 copies Hadamard DC without primary-SP dequantization.
    assert_eq!(
        switching_chroma_420(&[128; 64], &[0; 4], &[[0; 16]; 4], 0).unwrap(),
        [26; 64]
    );
    assert!(switching_chroma_420(&[256; 64], &[0; 4], &[[0; 16]; 4], 0).is_err());
    assert!(switching_chroma_420(&[128; 64], &[0; 4], &[[0; 16]; 4], 40).is_err());
    assert!(switching_chroma_420(&[128; 64], &[i32::MAX; 4], &[[0; 16]; 4], 0).is_err());
    let mut ac = [[0; 16]; 4];
    ac[1][0] = 1;
    assert!(switching_chroma_420(&[128; 64], &[0; 4], &ac, 0).is_err());
}
#[test]
fn secondary_sp_skip_and_signed_residual_frames_match_normative_oracle_after_reset() {
    for c in manifest()["videos"].as_array().unwrap() {
        let mut decoder =
            AvcDecoder::new(&hex(c["configuration"].as_str().unwrap()), 1 << 20).unwrap();
        let expected = fixture(c["reference"].as_str().unwrap());
        for _ in 0..2 {
            let mut actual = vec![];
            for p in c["packets"].as_array().unwrap() {
                decoder
                    .decode(&hex(p.as_str().unwrap()))
                    .unwrap()
                    .unwrap()
                    .write_planar(&mut actual)
                    .unwrap();
            }
            assert_eq!(actual, expected, "{}", c["file"]);
            decoder.reset();
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn secondary_sp_software_mp4_seek_and_rewind_match_normative_oracle() {
    for c in manifest()["videos"].as_array().unwrap() {
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(fixture(c["file"].as_str().unwrap())),
            Default::default(),
            1 << 20,
        )
        .unwrap();
        assert!(!reader.hardware_accelerated());
        let expected = fixture(c["reference"].as_str().unwrap());
        for target in [None, Some(2), Some(3), None] {
            if let Some(target) = target {
                assert_eq!(reader.seek_to_sync(target), 0);
            } else {
                reader.rewind();
            }
            let mut actual = vec![];
            while let Some(f) = reader.read_frame().unwrap() {
                actual.extend(
                    f.picture
                        .y
                        .iter()
                        .chain(&f.picture.cb)
                        .chain(&f.picture.cr)
                        .map(|v| *v as u8),
                );
            }
            assert_eq!(actual, expected, "{}", c["file"]);
        }
    }
}
