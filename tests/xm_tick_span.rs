#[cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
#[cfg(feature = "player")]
use std::io::Cursor;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-tick-span")
}
#[cfg(feature = "player")]
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
#[cfg(feature = "player")]
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
fn tick_spans_match_independent_equal_tempo_controls() {
    for case in cases() {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        let actual = fvid::container::xm::Module::parse(&bytes, &Default::default()).unwrap();
        let expected = fvid::container::xm::Module::parse(&control, &Default::default()).unwrap();
        let wanted = std::time::Duration::from_micros(case["score_us"].as_u64().unwrap());
        assert_eq!(
            expected.duration(),
            wanted,
            "{}: independent exact-tick control",
            case["name"]
        );
        assert_eq!(actual.duration(), wanted, "{}", case["name"]);
        println!(
            "{}: score actual={:?},independent control={wanted:?}",
            case["name"],
            actual.duration()
        );
    }
}

#[test]
#[cfg(feature = "player")]
fn equivalent_tempos_publish_identical_pcm_and_endpoints() {
    for case in cases() {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        let actual =
            fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes.clone()), Default::default())
                .unwrap();
        let expected = fvid::playback_xm::XmAudioReader::open(
            Cursor::new(control.clone()),
            Default::default(),
        )
        .unwrap();
        let millis = case["duration_ms"].as_u64().unwrap();
        assert_eq!(
            expected.duration(),
            Some(std::time::Duration::from_millis(millis))
        );
        assert_eq!(
            actual.duration(),
            Some(std::time::Duration::from_millis(millis))
        );
        let pcm = render(bytes, 44100, 2);
        let reference = render(control, 44100, 2);
        assert!(pcm == reference, "{}", case["name"]);
        assert_eq!(pcm.len(), reference.len(), "{}", case["name"]);
    }
}

#[cfg(feature = "player")]
fn fixed_window(bytes: &[u8], pts: i64, rate: u32, channels: u16) -> Vec<u8> {
    let module = fvid::container::xm::Module::parse(bytes, &Default::default()).unwrap();
    let mut decoder = fvid::codec::xm_decoder::XmDecoder::new(bytes, rate, channels).unwrap();
    // Both independently authored first rows intend100ms. Fix the external
    // presentation window to isolate a subrow deadline from source row length.
    let out = decoder
        .decode_packet(&module.packet(0), pts, 100)
        .unwrap()
        .unwrap()
        .packet;
    assert_eq!((out.timebase_num, out.timebase_den), (1, rate));
    assert_eq!(out.pts, (pts as u128 * rate as u128 / 1000) as u64);
    assert_eq!(decoder.ignored().effects, 0);
    out.data
}

#[test]
#[cfg(feature = "player")]
fn multi_tick_deadlines_match_the_same_fixed_window() {
    video_control();
    for case in cases().into_iter().filter(|c| c["kind"] == "deadline") {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        for pts in [0, 1000] {
            let actual = fixed_window(&bytes, pts, 44100, 2);
            let expected = fixed_window(&control, pts, 44100, 2);
            assert_eq!(actual.len(), expected.len());
            assert!(actual == expected, "{}: deadline differs", case["name"]);
        }
    }
}

#[test]
fn equivalent_tempo_rows_preserve_core_seek_boundaries() {
    for case in cases() {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        let actual = fvid::container::xm::Module::parse(&bytes, &Default::default()).unwrap();
        let expected = fvid::container::xm::Module::parse(&control, &Default::default()).unwrap();
        for row in expected.timeline() {
            for target in [
                row.start_microseconds.saturating_sub(1),
                row.start_microseconds,
                row.start_microseconds + 1,
            ] {
                assert_eq!(
                    actual.packet_at_microsecond(target),
                    expected.packet_at_microsecond(target),
                    "{} at{target}us",
                    case["name"]
                );
            }
        }
        assert_eq!(actual.duration(), expected.duration());
        assert_eq!(actual.packets(), expected.packets());
        for index in 0..actual.packets() {
            assert_eq!(
                actual.packet_time(index),
                expected.packet_time(index),
                "{} packet{index}",
                case["name"]
            );
        }
    }
}

#[test]
#[cfg(feature = "player")]
fn equivalent_tempos_match_independent_full_pcm_and_published_endpoints() {
    video_control();
    for case in cases() {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        for rate in [11025, 44100] {
            for channels in [1, 2] {
                let actual = render(bytes.clone(), rate, channels);
                let expected = render(control.clone(), rate, channels);
                assert!(
                    actual == expected,
                    "{} rate{rate} channels{channels}: equal tempo ratios differ",
                    case["name"]
                );
                let frames = case["duration_ms"].as_u64().unwrap() * rate as u64 / 1000;
                assert_eq!(
                    actual.len(),
                    frames as usize * channels as usize * 4,
                    "{}",
                    case["name"]
                );
            }
        }
        let source =
            fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes), Default::default()).unwrap();
        assert_eq!(
            source.duration(),
            Some(std::time::Duration::from_millis(
                case["duration_ms"].as_u64().unwrap()
            ))
        );
    }
}

#[test]
#[cfg(feature = "player")]
fn whole_tick_span_deadlines_match_fixed_presentation_windows() {
    for case in cases().into_iter().filter(|c| c["kind"] == "deadline") {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        for pts in [0, 1000] {
            for rate in [11025, 44100] {
                for channels in [1, 2] {
                    assert!(
                        fixed_window(&bytes, pts, rate, channels)
                            == fixed_window(&control, pts, rate, channels),
                        "{} pts{pts} rate{rate} channels{channels}",
                        case["name"]
                    );
                }
            }
        }
    }
}

#[cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
#[cfg(feature = "player")]
fn whole_tick_spans_actual_worker_preserves_clock_pcm_seek_and_eof() {
    video_control();
    for case in cases() {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        let mut reference =
            fvid::playback_xm::XmAudioReader::open(Cursor::new(control), Default::default())
                .unwrap();
        let end = reference.duration().unwrap();
        let post = case["post_row"].as_u64().unwrap() as usize;
        let target = std::time::Duration::from_micros(reference.module().packet_time(post).0);
        let mut decoder = reference.make_decoder().unwrap();
        let mut expected = Vec::new();
        while let Some(packet) = reference.next_packet().unwrap() {
            let out = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
                .unwrap()
                .packet;
            expected.push((out.presentation_time(), out.data));
        }
        for (at, suffix) in [
            (target, &expected[post..]),
            (end, &expected[expected.len()..]),
        ] {
            let source = fvid::playback_xm::XmAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap();
            capture::worker_check_with_seek_expected(Box::new(source), &expected, at, suffix);
        }
    }
}
