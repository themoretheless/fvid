use std::path::Path;
fn manifest() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/playback-errors/aac-ssr-ps.json")).unwrap()
}
fn artifact(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
        .collect()
}
fn video(c: &serde_json::Value) -> Vec<u8> {
    artifact(c["video"]["file"].as_str().unwrap())
}
#[test]
fn ssr_ps_fixture_core_controls_accept_all_authored_window_schedules() {
    let m = manifest();
    assert_eq!(m["controls"].as_array().unwrap().len(), 3);
    for case in m["controls"].as_array().unwrap() {
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut pcm).unwrap();
        assert_eq!(pcm.len(), 3072 * 4);
        assert!(pcm.iter().all(|s| *s == 0));
    }
}
#[test]
fn ssr_ps_payloads_reach_nonzero_independent_stereo_pcm_with_real_eof() {
    use fvid_media::owned_aac::{aac_sbr_dsp::OutputRate, aac_sbr_ps::Decoder, bits::BitReader};
    let m = manifest();
    for (rate, mode) in [(24000, OutputRate::Core), (48000, OutputRate::Double)] {
        let mut decoder = Decoder::default();
        let zero = vec![0.0; 1024];
        let mut actual = Vec::new();
        for payload in m["payloads"].as_array().unwrap() {
            let raw = hex(payload.as_str().unwrap());
            let mut bits = BitReader::new(&raw);
            let kind = bits.read(4).unwrap();
            assert!(matches!(kind, 13 | 14));
            if let Some(frame) = decoder
                .read(&mut bits, raw.len() * 8, kind == 14, &zero, 48000, 16, mode)
                .unwrap()
            {
                actual.extend(
                    frame.pcm[0]
                        .iter()
                        .zip(&frame.pcm[1])
                        .flat_map(|(&l, &r)| [l, r]),
                );
            }
        }
        while let Some(frame) = decoder.finish().unwrap() {
            actual.extend(
                frame.pcm[0]
                    .iter()
                    .zip(&frame.pcm[1])
                    .flat_map(|(&l, &r)| [l, r]),
            );
        }
        assert!(decoder.finish().unwrap().is_none());
        let gold = artifact(&format!("aac-ssr-ps-{rate}-reference.f64le"));
        assert_eq!(actual.len() * 8, gold.len());
        assert!(actual.iter().any(|v| v.abs() > 1e-6));
        for (i, (a, b)) in actual.iter().zip(gold.chunks_exact(8)).enumerate() {
            let b = f64::from_le_bytes(b.try_into().unwrap());
            assert!((a - b).abs() < 5e-12, "sample {i}: {a} vs {b}");
        }
    }
}
#[test]
fn combined_ssr_ps_reproduces_the_profile_gate_instead_of_a_container_or_fill_error() {
    use fvid_media::owned_aac::aac_ps_native::NativePsAacDecoder;
    let m = manifest();
    let expected = m["error"].as_str().unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 12);
    for case in m["cases"].as_array().unwrap() {
        let error = NativePsAacDecoder::new(&hex(case["asc"].as_str().unwrap()))
            .err()
            .unwrap();
        assert_eq!(error.to_string(), expected);
        let error = fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{}: {error}", case["name"]);
    }
}
#[test]
#[ignore = "SSR core alignment plus PS lookahead and both EOF frames are not implemented"]
fn combined_ssr_ps_native_waveform_acceptance() {
    let m = manifest();
    for case in m["cases"].as_array().unwrap() {
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut pcm).unwrap();
        let gold = artifact(case["reference"].as_str().unwrap());
        assert_eq!(pcm.len(), case["samples"].as_u64().unwrap() as usize * 8);
        assert_eq!(gold.len(), pcm.len() * 2);
        for (i, (a, b)) in pcm.chunks_exact(4).zip(gold.chunks_exact(8)).enumerate() {
            let a = f32::from_le_bytes(a.try_into().unwrap()) as f64;
            let b = f64::from_le_bytes(b.try_into().unwrap());
            assert!(
                (a - b).abs() < 1e-9,
                "{} sample {i}: {a} vs {b}",
                case["name"]
            );
        }
    }
}
