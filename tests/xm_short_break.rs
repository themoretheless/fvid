#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-short-break")
}
fn render(bytes: Vec<u8>, rate: u32, channels: u16) -> (Vec<u8>, u32) {
    let mut source =
        fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes), Default::default()).unwrap();
    let mut decoder =
        fvid::codec::xm_decoder::XmDecoder::new(source.extra_data(), rate, channels).unwrap();
    let mut pcm = Vec::new();
    while let Some(packet) = source.next_packet().unwrap() {
        let output = decoder
            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
            .unwrap()
            .unwrap();
        assert_eq!(output.packet.timebase_den, rate);
        pcm.extend(output.packet.data);
    }
    (pcm, decoder.ignored().effects)
}
fn cases() -> Vec<serde_json::Value> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    manifest["cases"].as_array().unwrap().clone()
}
fn trace(bytes: &[u8]) -> Vec<[usize; 2]> {
    let module = fvid::container::xm::Module::parse(bytes, &Default::default()).unwrap();
    module.timeline().iter().map(|r| [r.order, r.row]).collect()
}
fn expected_trace(case: &serde_json::Value) -> Vec<[usize; 2]> {
    case["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            [
                r[0].as_u64().unwrap() as usize,
                r[1].as_u64().unwrap() as usize,
            ]
        })
        .collect()
}
#[test]
fn short_pattern_break_preserves_every_destination() {
    for case in cases() {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        assert_eq!(trace(&bytes), expected_trace(&case), "{}", case["name"]);
        assert_eq!(
            render(bytes, 44_100, 2).0,
            render(control, 44_100, 2).0,
            "{}",
            case["name"]
        );
    }
}
#[test]
fn short_pattern_break_matches_literal_navigation_and_flattened_pcm() {
    for case in cases() {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        assert_eq!(trace(&bytes), expected_trace(&case), "{}", case["name"]);
        for rate in [11_025, 44_100] {
            for channels in [1, 2] {
                let (actual, ignored) = render(bytes.clone(), rate, channels);
                let (expected, other) = render(control.clone(), rate, channels);
                assert_eq!((ignored, other), (0, 0));
                assert_eq!(
                    actual, expected,
                    "{} rate{rate} channels{channels}",
                    case["name"]
                );
            }
        }
    }
}

#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn short_pattern_break_worker_preserves_independent_score_through_rewind_seek_and_eof() {
    use std::time::Duration;
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
        let mut source =
            fvid::playback_xm::XmAudioReader::open(Cursor::new(control), Default::default())
                .unwrap();
        let total = source.duration().unwrap();
        let mut decoder =
            fvid::codec::xm_decoder::XmDecoder::new(source.extra_data(), source.sample_rate(), 2)
                .unwrap();
        let mut expected = Vec::new();
        while let Some(p) = source.next_packet().unwrap() {
            let out = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            expected.push((out.packet.presentation_time(), out.packet.data));
        }
        for (target, suffix) in [
            (Duration::from_millis(120), &expected[1..]),
            (total, &expected[expected.len()..]),
        ] {
            let actual = fvid::playback_xm::XmAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap();
            assert_eq!(actual.duration(), Some(total), "{}", case["name"]);
            capture::worker_check_with_seek_expected(Box::new(actual), &expected, target, suffix);
        }
    }
}
