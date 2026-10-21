#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/xm-key-off-volume-state")
}
fn render(bytes: Vec<u8>, rate: u32, channels: u16) -> Vec<u8> {
    let mut source =
        fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes), Default::default()).unwrap();
    let mut decoder =
        fvid::codec::xm_decoder::XmDecoder::new(source.extra_data(), rate, channels).unwrap();
    assert_eq!(decoder.ignored().envelopes, 0);
    assert_eq!(decoder.ignored().fades, 0);
    let mut pcm = Vec::new();
    while let Some(packet) = source.next_packet().unwrap() {
        let output = decoder
            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
            .unwrap()
            .unwrap();
        assert_eq!(output.packet.timebase_den, rate);
        pcm.extend(output.packet.data);
    }
    assert_eq!(decoder.ignored().effects, 0);
    assert_eq!(decoder.ignored().volume_columns, 0);
    assert_eq!(decoder.ignored().missing_samples, 0);
    assert_eq!(decoder.ignored().unplayable_notes, 0);
    pcm
}
fn cases() -> Vec<serde_json::Value> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    manifest["cases"].as_array().unwrap().clone()
}
fn video_control() {
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(root().join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
}
#[test]
fn no_envelope_key_off_preserves_channel_volume_and_explicit_restore() {
    for case in cases() {
        let pcm = render(
            std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap(),
            44_100,
            2,
        );
        let expected = render(
            std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap(),
            44_100,
            2,
        );
        assert_eq!(pcm.len(), expected.len(), "{}", case["name"]);
        assert!(
            pcm == expected,
            "{}: key-off PCM differs from independent volume control",
            case["name"]
        );
        let first = pcm.iter().zip(&expected).position(|(a, b)| a != b);
        let post = case["post_row"].as_u64().unwrap() as usize * 5292;
        if let Some(byte) = first {
            assert!(
                byte / 8 >= post,
                "{}: unrelated earlier PCM difference",
                case["name"]
            );
        }
        let start = (post + 2646) * 8;
        let end = (post + 5292) * 8;
        let active = |data: &[u8]| {
            data[start..end]
                .chunks_exact(4)
                .any(|sample| f32::from_le_bytes(sample.try_into().unwrap()) != 0.0)
        };
        let (actual_active, expected_active) = (active(&pcm), active(&expected));
        match case["kind"].as_str().unwrap() {
            "unexpected-note-resume" => assert_eq!(
                (actual_active, expected_active),
                (false, false),
                "{}",
                case["name"]
            ),
            "blocked-volume-restore" => assert_eq!(
                (actual_active, expected_active),
                (true, true),
                "{}",
                case["name"]
            ),
            _ => assert_eq!(actual_active, expected_active, "{}", case["name"]),
        }
        println!(
            "{}: PCM differs={},first differing frame={:?},post-row audible actual={actual_active},control={expected_active}",
            case["name"],
            pcm != expected,
            first.map(|b| b / 8)
        );
    }
}
#[test]
fn no_envelope_key_off_matches_independent_volume_cut_and_restore_controls() {
    video_control();
    for case in cases() {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        for rate in [11_025, 44_100] {
            for channels in [1, 2] {
                let actual = render(bytes.clone(), rate, channels);
                let expected = render(control.clone(), rate, channels);
                let first = actual.iter().zip(&expected).position(|(a, b)| a != b);
                assert!(
                    actual == expected,
                    "{} rate{rate} channels{channels}: first differing PCM byte {first:?}",
                    case["name"]
                );
            }
        }
    }
}

#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn no_envelope_key_off_actual_worker_preserves_volume_state_through_seek_and_eof() {
    use std::time::Duration;
    video_control();
    for case in cases() {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        let expected = render(control, 44_100, 2);
        let mut source =
            fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes.clone()), Default::default())
                .unwrap();
        let total = source.duration().unwrap();
        assert_eq!(
            total,
            Duration::from_millis(case["duration_ms"].as_u64().unwrap())
        );
        let mut packets = Vec::new();
        let mut cursor = 0usize;
        while let Some(packet) = source.next_packet().unwrap() {
            let start = u128::try_from(packet.pts).unwrap() * 44_100 / 1000;
            let end =
                (u128::try_from(packet.pts).unwrap() + packet.duration as u128) * 44_100 / 1000;
            let count = usize::try_from(end - start).unwrap() * 8;
            packets.push((
                Duration::from_nanos((start * 1_000_000_000 / 44_100) as u64),
                expected[cursor..cursor + count].to_vec(),
            ));
            cursor += count;
        }
        assert_eq!(cursor, expected.len());
        let post = case["post_row"].as_u64().unwrap() as usize;
        let target = Duration::from_millis(post as u64 * 120);
        for (target, suffix) in [
            (target, &packets[post..]),
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
