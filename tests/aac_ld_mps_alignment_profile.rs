use fvid_media::owned_aac::NativeAacDecoder;
use serde_json::Value;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-alignment-profile")
}
fn hex(v: &Value) -> Vec<u8> {
    v.as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn packet<'a>(blob: &'a [u8], row: &Value) -> &'a [u8] {
    let start = row["offset"].as_u64().unwrap() as usize;
    &blob[start..start + row["bytes"].as_u64().unwrap() as usize]
}

#[test]
fn zero_alignment_plays_and_nonzero_profile_errors_preserve_retry_state() {
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 5);
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let good = &cases[0];
    assert_eq!(good["alignment"], 0);
    let mut complete = Vec::new();
    let movie = std::fs::read(root().join(good["video"].as_str().unwrap())).unwrap();
    let stats = fvid::native_media::decode_mp4_aac_pcm(&movie, &mut complete).unwrap();
    assert_eq!(stats.sample_frames, good["samples"].as_u64().unwrap());
    assert_eq!(complete.len() as u64, stats.sample_frames * 2 * 4);
    for bad in &cases[1..] {
        let mut reference = NativeAacDecoder::new(&hex(&good["asc"])).unwrap();
        let mut decoder = NativeAacDecoder::new(&hex(&bad["asc"])).unwrap();
        for index in 0..3 {
            let valid = packet(&blob, &good["frames"][index]);
            let expected = reference
                .decode_timed(valid, index as i64 * 512, 512)
                .unwrap();
            if index == 1 {
                let retained = decoder.retained_payload_bytes().unwrap();
                let error = decoder
                    .decode_timed(packet(&blob, &bad["frames"][index]), 512, 512)
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.contains("nonzero LD-MPS time alignment is not permitted"),
                    "{error}"
                );
                assert!(!error.contains("not yet implemented"));
                assert_eq!(decoder.retained_payload_bytes().unwrap(), retained);
            }
            assert_eq!(
                decoder
                    .decode_timed(valid, index as i64 * 512, 512)
                    .unwrap(),
                expected
            );
        }
        assert_eq!(decoder.finish().unwrap(), reference.finish().unwrap());
        let mut discarded = Vec::new();
        let movie = std::fs::read(root().join(bad["video"].as_str().unwrap())).unwrap();
        let error = fvid::native_media::decode_mp4_aac_pcm(&movie, &mut discarded)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("nonzero LD-MPS time alignment is not permitted"),
            "{error}"
        );
    }
}
