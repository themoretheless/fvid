use serde_json::Value;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn public_ltp_mp4_transitions_and_coupling_match_every_scalar_pcm_sample() {
    for name in ["aac-ltp-transitions", "aac-ltp-phase"] {
        let manifest: Value = serde_json::from_slice(&bytes(&format!("{name}.json"))).unwrap();
        let gold = bytes(&format!("{name}-reference.f32le"));
        for case in manifest["cases"].as_array().unwrap() {
            let mut pcm = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm(
                &bytes(case["video"]["file"].as_str().unwrap()),
                &mut pcm,
            )
            .unwrap();
            let n = case["n"].as_u64().unwrap_or(1024) as usize;
            assert_eq!(pcm.len(), case["frames"].as_array().unwrap().len() * n * 4);
            for (row, actual) in case["frames"]
                .as_array()
                .unwrap()
                .iter()
                .zip(pcm.chunks_exact(n * 4))
            {
                let at = row["reference_offset"].as_u64().unwrap() as usize;
                for (i, (sample, reference)) in actual
                    .chunks_exact(4)
                    .zip(gold[at..at + n * 4].chunks_exact(4))
                    .enumerate()
                {
                    let sample = f32::from_le_bytes(sample.try_into().unwrap());
                    let reference = f32::from_le_bytes(reference.try_into().unwrap());
                    assert!(
                        (sample - reference).abs() < 1e-7,
                        "{name} sample={i}: {sample} vs {reference}"
                    );
                }
            }
        }
    }
}
#[test]
fn public_ltp_cce_absent_target_video_reproduces_specific_routing_error() {
    let mut pcm = Vec::new();
    let error = fvid::native_media::decode_mp4_aac_pcm(
        &bytes("aac-ltp-phase-absent-target-synthetic.mp4"),
        &mut pcm,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("AAC coupling target is absent"),
        "{error}"
    );
    assert!(pcm.is_empty());
}
#[test]
fn public_ltp_stereo_mp4_matches_independent_pair_pcm() {
    let m: Value = serde_json::from_slice(&bytes("aac-ltp-pair.json")).unwrap();
    for case in m["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let gold = bytes(&format!("aac-ltp-pair-{name}-scalar-reference.f32le"));
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(
            &bytes(case["video"]["file"].as_str().unwrap()),
            &mut pcm,
        )
        .unwrap();
        assert_eq!(pcm.len(), gold.len());
        for (i, (sample, reference)) in pcm.chunks_exact(4).zip(gold.chunks_exact(4)).enumerate() {
            let sample = f32::from_le_bytes(sample.try_into().unwrap());
            let reference = f32::from_le_bytes(reference.try_into().unwrap());
            assert!(
                (sample - reference).abs() < 1e-7,
                "{name} sample={i}: {sample} vs {reference}"
            );
        }
    }
}
#[test]
fn public_ltp_pce_cce_gain_and_selection_videos_are_admitted() {
    let m: Value = serde_json::from_slice(&bytes("aac-ltp-coupling.json")).unwrap();
    for case in m["cases"].as_array().unwrap() {
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(
            &bytes(case["video"]["file"].as_str().unwrap()),
            &mut pcm,
        )
        .unwrap();
        assert_eq!(
            pcm.len(),
            12288 * case["channels"].as_u64().unwrap() as usize * 4
        );
        assert!(
            pcm.chunks_exact(4)
                .all(|s| f32::from_le_bytes(s.try_into().unwrap()).is_finite())
        );
    }
}
#[test]
fn owned_ltp_mp4_admission_rejects_low_budget_before_publishing_pcm() {
    let source = bytes("aac-ltp-transitions-1024-synthetic.mp4");
    let mut expected = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        std::io::Cursor::new(&source),
        &mut expected,
        None,
        &Default::default(),
    )
    .unwrap();
    for limit in [1, 16 * 1024 * 1024] {
        let mut output = Vec::new();
        let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&source),
            &mut output,
            None,
            &fvid_media::CopyOptions {
                max_controlled_bytes: Some(limit),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("controlled memory budget exceeded"),
            "{error}"
        );
        assert!(output.is_empty());
    }
    let mut output = Vec::new();
    let actual = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        std::io::Cursor::new(&source),
        &mut output,
        None,
        &fvid_media::CopyOptions {
            max_controlled_bytes: Some(256 * 1024 * 1024),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(actual, stats);
    assert_eq!(output, expected);
}
#[test]
fn ltp_pce_asc_roundtrip_keeps_profile_layout_and_coupling() {
    let m: Value = serde_json::from_slice(&bytes("aac-ltp-coupling.json")).unwrap();
    for case in m["cases"].as_array().unwrap() {
        let raw: Vec<u8> = case["asc"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        let parsed = fvid_media::owned_aac::config::AudioSpecificConfig::parse(&raw).unwrap();
        let program = parsed.program.as_ref().unwrap();
        let roundtrip = fvid_media::owned_aac::config::AudioSpecificConfig::parse(
            &program.audio_specific_config().unwrap(),
        )
        .unwrap();
        assert_eq!(roundtrip.core, parsed.core);
        assert_eq!(roundtrip.program, parsed.program);
    }
}
#[test]
fn ltp_mp4_ranges_after_short_history_match_exact_full_pcm_slices() {
    for name in [
        "aac-ltp-transitions-960-synthetic.mp4",
        "aac-ltp-transitions-1024-synthetic.mp4",
        "aac-ltp-phase-3-synthetic.mp4",
    ] {
        let source = bytes(name);
        let mut full = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&source),
            &mut full,
            None,
            &Default::default(),
        )
        .unwrap();
        for (from, to) in [(170u64, 300u64), (10, 75), (170, 300)] {
            let mut pcm = Vec::new();
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                std::io::Cursor::new(&source),
                &mut pcm,
                Some((
                    std::time::Duration::from_millis(from),
                    std::time::Duration::from_millis(to),
                )),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                pcm,
                &full[from as usize * 24 * 4..to as usize * 24 * 4],
                "{name} {from}..{to} ms"
            );
        }
    }
}
