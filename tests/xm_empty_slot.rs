#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-empty-slot")
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
fn missing_valid_instrument_slots_preserve_explicit_empty_slot_pcm() {
    for case in cases() {
        let (pcm, ignored) = render_with_ignored(
            std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap(),
            44_100,
            2,
        );
        let (control, control_ignored) = render_with_ignored(
            std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap(),
            44_100,
            2,
        );
        assert_eq!(pcm.len(), control.len());
        let post = case["post_row"].as_u64().unwrap() as usize * 5292 * 8;
        assert_eq!(
            &pcm[..post],
            &control[..post],
            "{}: earlier unrelated difference",
            case["name"]
        );
        assert!(
            pcm == control,
            "{}: absent slot replays prior sample",
            case["name"]
        );
        if case["kind"] == "missing-valid-slot" || case["kind"] == "initial-empty-slot" {
            assert!(
                control[post..3 * 5292 * 8].iter().all(|&b| b == 0),
                "empty slot must have no PCM"
            );
        }
        println!(
            "{}: PCM differs={},missing diagnostics actual={},empty control={}",
            case["name"],
            pcm != control,
            ignored.missing_samples,
            control_ignored.missing_samples
        );
    }
}

#[test]
fn missing_valid_slots_match_explicit_empty_instruments_and_report_once_per_cell() {
    video_control();
    for case in cases() {
        for rate in [11_025, 44_100] {
            for channels in [1, 2] {
                let (actual, ignored) = render_with_ignored(
                    std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap(),
                    rate,
                    channels,
                );
                let control = render(
                    std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap(),
                    rate,
                    channels,
                );
                assert!(
                    actual == control,
                    "{} rate{rate} channels{channels}: missing slot replayed old sample",
                    case["name"]
                );
                let missing = match case["kind"].as_str().unwrap() {
                    "missing-valid-slot" | "initial-empty-slot" => 2,
                    "ignored-out-of-format-slot" => 1,
                    _ => 0,
                };
                assert_eq!(ignored.missing_samples, missing, "{}", case["name"]);
            }
        }
    }
}

#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn empty_slot_actual_worker_preserves_selection_recovery_seek_and_eof() {
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
