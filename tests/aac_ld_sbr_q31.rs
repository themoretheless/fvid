use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-sbr-q31")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn manifest() -> Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn near(actual: &[u8], expected: &[u8], name: &str) {
    assert_eq!(actual.len(), expected.len(), "{name}");
    let mut worst = 0f32;
    for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!(a.is_finite());
        worst = worst.max((a - b).abs());
    }
    assert!(worst < 1e-7, "{name}: {worst}");
}
#[test]
fn q31_and_temporal_return_match_independent_pcm_in_mp4_and_matroska() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 18);
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let mut mp4 = Vec::new();
        let mut mkv = Vec::new();
        let stats =
            fvid::native_media::decode_mp4_aac_pcm(&read(c["video"].as_str().unwrap()), &mut mp4)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(stats.sample_rate, 48000);
        assert_eq!(stats.channels, 2);
        assert_eq!(stats.sample_frames, c["samples"].as_u64().unwrap());
        fvid::native_media::decode_matroska_aac_pcm_interval(
            &read(c["file"].as_str().unwrap()),
            &mut mkv,
            None,
        )
        .unwrap();
        near(&mp4, &read(c["golden"].as_str().unwrap()), name);
        assert_eq!(mp4, mkv, "{name}");
    }
}

fn asc(c: &Value) -> Vec<u8> {
    c["asc"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn packet<'a>(blob: &'a [u8], c: &Value, i: usize) -> &'a [u8] {
    let f = &c["frames"][i];
    let at = f["offset"].as_u64().unwrap() as usize;
    &blob[at..at + f["bytes"].as_u64().unwrap() as usize]
}
#[test]
fn policies_survive_reset_and_checkpoint_and_cross_policy_restore_is_transactional() {
    use fvid_media::owned_aac::{AacDecodeOptions, NativeAacDecoder, aac_eld_sbr::NoisePolicy};
    let m = manifest();
    let blob = read("packets.bin");
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["level"] == 31)
    {
        let config = asc(c);
        for hint in [None, Some(48000)] {
            let mut compatible = NativeAacDecoder::new_with_options(
                &config,
                AacDecodeOptions {
                    output_sample_rate: hint,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut strict = NativeAacDecoder::new_with_options(
                &config,
                AacDecodeOptions {
                    ld_sbr_noise: NoisePolicy::Strict,
                    output_sample_rate: hint,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut frontend = fvid::codec::aac_native::NativeAacDecoder::new_with_options(
                &config,
                fvid::codec::aac_native::AacDecodeOptions {
                    ld_sbr_noise: NoisePolicy::Strict,
                    output_sample_rate: hint,
                    ..Default::default()
                },
            )
            .unwrap();
            for _ in 0..2 {
                for i in 0..3 {
                    let a = compatible.decode(packet(&blob, c, i)).unwrap();
                    assert_eq!(a, strict.decode(packet(&blob, c, i)).unwrap());
                    assert_eq!(a, frontend.decode(packet(&blob, c, i)).unwrap());
                }
                let checkpoint = compatible.checkpoint();
                let strict_checkpoint = strict.checkpoint();
                assert!(strict.restore(&checkpoint).is_err());
                assert!(compatible.restore(&strict_checkpoint).is_err());
                for _ in 0..2 {
                    assert!(
                        strict
                            .decode(packet(&blob, c, 3))
                            .unwrap_err()
                            .to_string()
                            .contains("normative range: 31")
                    );
                }
                assert!(
                    frontend
                        .decode(packet(&blob, c, 3))
                        .unwrap_err()
                        .to_string()
                        .contains("normative range: 31")
                );
                let mut tail = Vec::new();
                for i in 3..8 {
                    tail.extend(compatible.decode(packet(&blob, c, i)).unwrap());
                }
                compatible.restore(&checkpoint).unwrap();
                let mut replay = Vec::new();
                for i in 3..8 {
                    replay.extend(compatible.decode(packet(&blob, c, i)).unwrap());
                }
                assert_eq!(tail, replay);
                compatible.reset();
                strict.reset();
                frontend.reset();
            }
        }
    }
}
#[test]
fn q32_is_still_refused_and_failed_temporal_updates_preserve_q31_history() {
    use fvid_media::owned_aac::NativeAacDecoder;
    let m = manifest();
    let blob = read("packets.bin");
    assert_eq!(m["errors"].as_array().unwrap().len(), 6);
    for bad in m["errors"].as_array().unwrap() {
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["n"] == bad["n"] && c["mode"] == bad["mode"] && c["level"] == 31)
            .unwrap();
        let mut retry = NativeAacDecoder::new(&asc(good)).unwrap();
        let mut control = NativeAacDecoder::new(&asc(good)).unwrap();
        for i in 0..8 {
            if i == 4 {
                for _ in 0..2 {
                    let e = retry.decode(packet(&blob, bad, i)).unwrap_err();
                    assert!(e.to_string().contains("normative range: 32"), "{e}");
                }
            }
            assert_eq!(
                retry.decode(packet(&blob, good, i)).unwrap(),
                control.decode(packet(&blob, good, i)).unwrap()
            );
        }
        let e = fvid::native_media::decode_mp4_aac_pcm(
            &read(bad["video"].as_str().unwrap()),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("normative range: 32"));
        let e = fvid::native_media::decode_matroska_aac_pcm_interval(
            &read(bad["file"].as_str().unwrap()),
            &mut Vec::new(),
            None,
        )
        .unwrap_err();
        assert!(e.to_string().contains("normative range: 32"));
    }
}

#[test]
fn unchanged_original_q31_refusal_movies_are_default_playback_acceptance_now() {
    for n in [480, 512] {
        let original = root().parent().unwrap().join("aac-eld-sbr-noise-range");
        let mut mp4 = Vec::new();
        let mut mkv = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(original.join(format!("n{n}-q31.mp4"))).unwrap(),
            &mut mp4,
        )
        .unwrap();
        fvid::native_media::decode_matroska_aac_pcm_interval(
            &std::fs::read(original.join(format!("n{n}-q31.mkv"))).unwrap(),
            &mut mkv,
            None,
        )
        .unwrap();
        near(
            &mp4,
            &read(&format!("n{n}-invf-q31.f32le")),
            "original Q31 movie",
        );
        assert_eq!(mkv, mp4);
    }
}
