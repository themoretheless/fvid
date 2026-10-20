use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-row-budget")
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    m["cases"].as_array().unwrap().clone()
}
fn limits(rows: usize) -> fvid::container::xm::Limits {
    fvid::container::xm::Limits {
        rows,
        ..Default::default()
    }
}
#[test]
fn every_timeline_row_respects_the_limit_even_when_it_exits_the_order_list() {
    for case in cases() {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let max = case["short_budget"].as_u64().unwrap() as usize;
        let result = fvid::container::xm::Module::parse(&bytes, &limits(max));
        assert!(result.is_err(), "{} exceeded rows={max}", case["file"]);
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("module does not end within")
        );
    }
}
#[test]
fn exact_row_limits_allow_natural_end_jump_and_one_pass_loop() {
    for case in cases() {
        let n = case["rows"].as_u64().unwrap() as usize;
        for file in ["file", "control"] {
            let bytes = std::fs::read(root().join(case[file].as_str().unwrap())).unwrap();
            for max in [n, n + 1] {
                let parsed = fvid::container::xm::Module::parse(&bytes, &limits(max)).unwrap();
                assert_eq!(parsed.timeline().len(), n);
            }
        }
    }
    let looped = std::fs::read(root().join("one-pass-loop.xm")).unwrap();
    assert_eq!(
        fvid::container::xm::Module::parse(&looped, &limits(1))
            .unwrap()
            .timeline()
            .len(),
        1
    );
}

#[cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
#[cfg(feature = "player")]
#[test]
fn reader_refuses_short_budgets_and_exact_budget_scores_play_rewind_seek_to_eof() {
    use fvid::audio::{AudioDecode, AudioStream};
    use std::{io::Cursor, time::Duration};
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(root().join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    for case in cases() {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        let n = case["rows"].as_u64().unwrap() as usize;
        let refusal =
            fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes.clone()), limits(n - 1));
        assert!(refusal.is_err());
        assert!(
            refusal
                .err()
                .unwrap()
                .to_string()
                .contains("module does not end within")
        );
        let mut reference =
            fvid::playback_xm::XmAudioReader::open(Cursor::new(control), limits(n)).unwrap();
        let total = reference.duration().unwrap();
        assert_eq!(total, Duration::from_millis((n as u64 + 1) * 120));
        let mut decoder = fvid::codec::xm_decoder::XmDecoder::new(
            reference.extra_data(),
            reference.sample_rate(),
            2,
        )
        .unwrap();
        let mut expected = Vec::new();
        while let Some(packet) = reference.next_packet().unwrap() {
            let frame = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
                .unwrap();
            expected.push((frame.packet.presentation_time(), frame.packet.data));
        }
        for (target, suffix) in [
            (Duration::from_millis(120), &expected[1..]),
            (total, &expected[expected.len()..]),
        ] {
            let actual =
                fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes.clone()), limits(n))
                    .unwrap();
            capture::worker_check_with_seek_expected(Box::new(actual), &expected, target, suffix);
        }
    }
}
