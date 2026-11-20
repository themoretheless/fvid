//! Preserve explicit strict Q31 refusal; default LD-SBR accepts Q31 separately.
use fvid_media::owned_aac::NativeAacDecoder;
use serde_json::Value;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/aac-eld-sbr-noise-range")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn number(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn asc(c: &Value) -> Vec<u8> {
    c["asc"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn packet<'a>(blob: &'a [u8], c: &Value, index: usize) -> &'a [u8] {
    let f = &c["frames"][index];
    let at = number(&f["offset"]);
    &blob[at..at + number(&f["bytes"])]
}
fn near(actual: &[u8], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!(a.is_finite() && (a - b).abs() < 1e-7, "{a} vs {b}");
    }
}
#[test]
fn valid_inverse_filter_controls_preserve_frozen_pcm_in_both_containers() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 2);
    for c in m["cases"].as_array().unwrap() {
        let mut mp4 = Vec::new();
        let mut mkv = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&read(c["video"].as_str().unwrap()), &mut mp4)
            .unwrap();
        fvid::native_media::decode_matroska_aac_pcm_interval(
            &read(c["file"].as_str().unwrap()),
            &mut mkv,
            None,
        )
        .unwrap();
        near(&mp4, &read(c["golden"].as_str().unwrap()));
        assert_eq!(mkv, mp4);
    }
}
fn strict(c: &Value) -> NativeAacDecoder {
    NativeAacDecoder::new_with_options(
        &asc(c),
        fvid_media::owned_aac::AacDecodeOptions {
            ld_sbr_noise: fvid_media::owned_aac::aac_eld_sbr::NoisePolicy::Strict,
            ..Default::default()
        },
    )
    .unwrap()
}
#[test]
fn explicit_strict_policy_refuses_q31_after_valid_crc_and_core_syntax() {
    let m = manifest();
    let blob = read("packets.bin");
    assert_eq!(m["strict_errors"].as_array().unwrap().len(), 2);
    for c in m["strict_errors"].as_array().unwrap() {
        let mut decoder = strict(c);
        for i in 0..number(&c["bad_frame"]) {
            decoder.decode(packet(&blob, c, i)).unwrap();
        }
        let error = decoder
            .decode(packet(&blob, c, number(&c["bad_frame"])))
            .unwrap_err();
        assert!(
            error.to_string().contains(c["error"].as_str().unwrap()),
            "{error}"
        );
    }
}
#[test]
fn late_noise_refusal_retains_core_sbr_crc_filter_and_coefficient_history_for_retry() {
    let m = manifest();
    let blob = read("packets.bin");
    for bad in m["strict_errors"].as_array().unwrap() {
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["n"] == bad["n"])
            .unwrap();
        assert_eq!(asc(good), asc(bad));
        let mut control = strict(good);
        let mut retry = strict(good);
        for i in 0..8 {
            if i == number(&bad["bad_frame"]) {
                assert_ne!(packet(&blob, bad, i), packet(&blob, good, i));
                // The original valid channel/CRC/parser path accepts all preceding AUs;
                // this AU reaches dequantization and reports the reconstructed Q.
                for _ in 0..2 {
                    let error = retry.decode(packet(&blob, bad, i)).unwrap_err();
                    assert!(
                        error.to_string().contains(bad["error"].as_str().unwrap()),
                        "{error}"
                    );
                }
            } else {
                assert_eq!(packet(&blob, bad, i), packet(&blob, good, i));
            }
            assert_eq!(
                retry.decode(packet(&blob, good, i)).unwrap(),
                control.decode(packet(&blob, good, i)).unwrap()
            );
        }
        retry.reset();
        control.reset();
        for i in 0..8 {
            assert_eq!(
                retry.decode(packet(&blob, good, i)).unwrap(),
                control.decode(packet(&blob, good, i)).unwrap()
            );
        }
    }
}
