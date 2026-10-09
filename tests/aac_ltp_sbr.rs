use serde_json::Value;
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn ltp_sbr_explicit_sync_and_implicit_mp4_clocks_match_scalar_pcm() {
    let m: Value = serde_json::from_slice(&bytes("aac-ltp-sbr.json")).unwrap();
    for case in m["cases"].as_array().unwrap() {
        let rate = case["rate"].as_u64().unwrap();
        let gold = bytes(&format!("aac-ltp-sbr-{rate}-reference.f64le"));
        let control = bytes(&format!("aac-ltp-sbr-{rate}-inactive-control.f64le"));
        let source = bytes(case["video"]["file"].as_str().unwrap());
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&source, &mut pcm).unwrap();
        assert_eq!(pcm.len() * 2, gold.len());
        let mut prediction_peak = 0f64;
        for (i, ((actual, reference), inactive)) in pcm
            .chunks_exact(4)
            .zip(gold.chunks_exact(8))
            .zip(control.chunks_exact(8))
            .enumerate()
        {
            let actual = f32::from_le_bytes(actual.try_into().unwrap()) as f64;
            let reference = f64::from_le_bytes(reference.try_into().unwrap());
            let inactive = f64::from_le_bytes(inactive.try_into().unwrap());
            assert!(
                (actual - reference).abs() < 1e-9,
                "rate={rate} signal={} sample={i}: {actual} vs {reference}",
                case["signal"]
            );
            prediction_peak = prediction_peak.max((actual - inactive).abs());
        }
        assert!(
            prediction_peak > 1e-5,
            "fixture must exercise LTP through SBR: {prediction_peak}"
        );
        let mut owned = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&source),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_rate, rate as u32);
        assert_eq!(owned, pcm);
        for (from, to) in [(130, 200), (10, 60), (130, 200)] {
            let mut ranged = Vec::new();
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&source),
                &mut ranged,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                ranged,
                &pcm[(from * rate / 1000 * 4) as usize..(to * rate / 1000 * 4) as usize]
            );
        }
    }
}
#[test]
fn ltp_adts_discovers_sbr_and_matches_explicit_mp4_output() {
    let data = bytes("aac-ltp-sbr-synthetic.aac");
    let mut expected = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(
        &bytes("aac-ltp-sbr-48000-explicit-synthetic.mp4"),
        &mut expected,
    )
    .unwrap();
    let mut output = Vec::new();
    let stats = fvid_media::owned_aac::decode_adts_pcm(
        Cursor::new(&data),
        &mut output,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        (stats.sample_rate, stats.channels, stats.sample_frames),
        (48000, 1, 12288)
    );
    assert_eq!(output, expected);
    let mut root = Vec::new();
    fvid::native_media::decode_aac_pcm_interval(&data, &mut root, &Default::default(), None)
        .unwrap();
    assert_eq!(root, expected);
    let mut ranged = Vec::new();
    fvid_media::owned_aac::decode_adts_pcm(
        Cursor::new(&data),
        &mut ranged,
        Some((Duration::from_millis(130), Duration::from_millis(200))),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(ranged, &expected[6240 * 4..9600 * 4]);
}
#[test]
fn ltp_sbr_corrupt_video_reports_crc_failure() {
    let mut pcm = Vec::new();
    let error = fvid::native_media::decode_mp4_aac_pcm(
        &bytes("aac-ltp-sbr-bad-crc-synthetic.mp4"),
        &mut pcm,
    )
    .unwrap_err();
    assert!(error.to_string().contains("SBR CRC mismatch"), "{error}");
}
