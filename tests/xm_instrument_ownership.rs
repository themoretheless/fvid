#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/xm-instrument-ownership")
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
#[test]
fn instrument_selection_keeps_the_playing_sample_until_a_note() {
    for case in cases() {
        let actual = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        assert_eq!(
            render(actual, 44_100, 2).0,
            render(control, 44_100, 2).0,
            "{}",
            case["name"]
        );
    }
}
#[test]
fn instrument_selection_preserves_independent_active_and_future_pcm() {
    for case in cases() {
        let actual = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        for rate in [11_025, 44_100] {
            for channels in [1, 2] {
                let (pcm, ignored) = render(actual.clone(), rate, channels);
                let (expected, other) = render(control.clone(), rate, channels);
                assert_eq!((ignored, other), (0, 0));
                assert_eq!(
                    pcm, expected,
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
fn instrument_ownership_worker_preserves_independent_score_through_rewind_seek_and_eof() {
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

#[test]
fn an_unselected_note_stays_silent_and_reports_the_missing_selection() {
    let bytes = std::fs::read(root().join("unselected-note.xm")).unwrap();
    let mut source =
        fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes), Default::default()).unwrap();
    let mut decoder =
        fvid::codec::xm_decoder::XmDecoder::new(source.extra_data(), source.sample_rate(), 2)
            .unwrap();
    for index in 0..3 {
        let p = source.next_packet().unwrap().unwrap();
        let frame = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        let audible = frame
            .packet
            .data
            .chunks_exact(4)
            .any(|sample| f32::from_le_bytes(sample.try_into().unwrap()) != 0.0);
        assert_eq!(audible, index == 2);
        assert_eq!(decoder.ignored().missing_samples, 1);
    }
}
