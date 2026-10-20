#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-note-cut")
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
fn note_cut_is_supported_and_matches_active_or_no_op_control() {
    for case in cases() {
        let actual = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        let (pcm, ignored) = render(actual, 44_100, 2);
        let (expected, _) = render(control, 44_100, 2);
        assert_eq!(ignored, 0);
        assert_eq!(pcm, expected, "{}", case["name"]);
    }
}
#[test]
fn note_cut_matches_independent_volume_zero_at_tick_boundaries() {
    for case in cases() {
        let actual = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        for rate in [11_025, 44_100] {
            for channels in [1, 2] {
                let (pcm, ignored) = render(actual.clone(), rate, channels);
                let (expected, other_ignored) = render(control.clone(), rate, channels);
                assert_eq!(other_ignored, 0);
                assert_eq!(ignored, 0, "{}", case["name"]);
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
fn note_cut_actual_worker_preserves_independent_pcm_through_rewind_seek_and_eof() {
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
        let (expected, ignored) = render(control, 44_100, 2);
        assert_eq!(ignored, 0);
        let mut source =
            fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes.clone()), Default::default())
                .unwrap();
        let total = source.duration().unwrap();
        assert_eq!(total, Duration::from_millis(480));
        let mut packets = Vec::new();
        let mut cursor = 0usize;
        while let Some(packet) = source.next_packet().unwrap() {
            let start = u128::try_from(packet.pts).unwrap() * 44_100 / 1000;
            let end =
                (u128::try_from(packet.pts).unwrap() + packet.duration as u128) * 44_100 / 1000;
            let count = usize::try_from(end - start).unwrap() * 8;
            let at = Duration::from_nanos((start * 1_000_000_000 / 44_100) as u64);
            packets.push((at, expected[cursor..cursor + count].to_vec()));
            cursor += count;
        }
        assert_eq!(cursor, expected.len());
        for (target, suffix) in [
            (Duration::from_millis(120), &packets[1..]),
            (total, &packets[packets.len()..]),
        ] {
            let source = fvid::playback_xm::XmAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap();
            capture::worker_check_with_seek_expected(Box::new(source), &packets, target, suffix);
        }
    }
}
