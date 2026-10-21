#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/xm-invalid-defaults")
}
fn render(bytes: Vec<u8>, rate: u32, channels: u16) -> Vec<u8> {
    render_with_ignored(bytes, rate, channels).0
}
fn render_with_ignored(
    bytes: Vec<u8>,
    rate: u32,
    channels: u16,
) -> (Vec<u8>, fvid::codec::xm_decoder::Ignored) {
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
    assert_eq!(decoder.ignored().unplayable_notes, 0);
    (pcm, decoder.ignored())
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
fn invalid_instrument_components_preserve_channel_defaults() {
    for case in cases() {
        let (pcm, ignored) = render_with_ignored(
            std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap(),
            44_100,
            2,
        );
        let control = render(
            std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap(),
            44_100,
            2,
        );
        let post = case["post_row"].as_u64().unwrap() as usize * 5292 * 8;
        assert_eq!(
            &pcm[..post],
            &control[..post],
            "{}: earlier unrelated PCM difference",
            case["name"]
        );
        assert!(
            pcm == control,
            "{}: invalid component restored defaults",
            case["name"]
        );
        assert_eq!(
            ignored.missing_samples,
            case["expected_missing"].as_u64().unwrap() as u32,
            "{}",
            case["name"]
        );
        let region = post + 64 * 8..post + 5292 * 8;
        let audible = |data: &[u8]| {
            data.chunks_exact(4)
                .any(|b| f32::from_le_bytes(b.try_into().unwrap()) != 0.0)
        };
        match case["kind"].as_str().unwrap() {
            "unexpected-unmute" => {
                assert!(!audible(&pcm[region.clone()]));
                assert!(!audible(&control[region]));
            }
            "unexpected-pan-reset" => {
                let right = |data: &[u8]| {
                    data.chunks_exact(8)
                        .any(|b| f32::from_le_bytes(b[4..8].try_into().unwrap()) != 0.0)
                };
                assert!(audible(&control[region.clone()]));
                assert!(!right(&pcm[region.clone()]));
                assert!(!right(&control[region]));
            }
            "silent-control" => assert!(!audible(&pcm[region])),
            _ => assert!(audible(&pcm[region])),
        }
        println!(
            "{}: PCM differs={},missing diagnostics={}",
            case["name"],
            pcm != control,
            ignored.missing_samples
        );
    }
}

#[test]
fn invalid_instrument_components_match_normalized_valid_notes_and_commands() {
    video_control();
    for case in cases() {
        for rate in [11_025, 44_100] {
            for channels in [1, 2] {
                let (actual, ignored) = render_with_ignored(
                    std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap(),
                    rate,
                    channels,
                );
                let expected = render(
                    std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap(),
                    rate,
                    channels,
                );
                assert!(
                    actual == expected,
                    "{} rate{rate} channels{channels}: invalid instrument changed defaults",
                    case["name"]
                );
                assert_eq!(
                    ignored.missing_samples,
                    case["expected_missing"].as_u64().unwrap() as u32,
                    "{}",
                    case["name"]
                );
            }
        }
    }
}

#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn invalid_instrument_components_actual_worker_preserves_normalized_pcm_seek_and_eof() {
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
