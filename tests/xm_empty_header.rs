#[cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
#[cfg(feature = "player")]
use std::io::Cursor;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/xm-empty-header")
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
fn short_empty_headers_parse_with_their_full_controls() {
    for case in cases() {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let result = fvid::container::xm::Module::parse(&bytes, &Default::default());
        assert!(result.is_ok(), "{}: {result:?}", case["name"]);
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        assert!(fvid::container::xm::Module::parse(&control, &Default::default()).is_ok());
    }
}

#[test]
fn malformed_empty_instrument_headers_are_refused() {
    for name in ["too-short-28", "nonempty-29", "truncated-29", "huge-header"] {
        let bytes = std::fs::read(root().join(format!("bad-{name}.xm"))).unwrap();
        assert!(
            fvid::container::xm::Module::parse(&bytes, &Default::default()).is_err(),
            "{name}"
        );
    }
}

#[test]
#[cfg(feature = "player")]
fn short_empty_headers_match_full_header_metadata_and_pcm() {
    video_control();
    for case in cases() {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(case["control"].as_str().unwrap())).unwrap();
        let actual = fvid::container::xm::Module::parse(&bytes, &Default::default()).unwrap();
        let expected = fvid::container::xm::Module::parse(&control, &Default::default()).unwrap();
        assert_eq!(
            actual.instruments.len(),
            case["instruments"].as_u64().unwrap() as usize
        );
        assert_eq!(
            format!("{:?}", actual.instruments),
            format!("{:?}", expected.instruments),
            "{}",
            case["name"]
        );
        for rate in [11_025, 44_100] {
            for channels in [1, 2] {
                let pcm = render(bytes.clone(), rate, channels);
                let reference = render(control.clone(), rate, channels);
                assert!(
                    pcm == reference,
                    "{} rate{rate} channels{channels}",
                    case["name"]
                );
                let audible = pcm
                    .chunks_exact(4)
                    .any(|s| f32::from_le_bytes(s.try_into().unwrap()) != 0.0);
                assert_eq!(
                    audible,
                    !case["empty_only"].as_bool().unwrap(),
                    "{}",
                    case["name"]
                );
            }
        }
    }
}

#[test]
fn short_empty_headers_preserve_resource_caps_and_specific_bad_header_refusals() {
    use fvid::container::xm::{Limits, Module};
    let only = std::fs::read(root().join("empty-only-29.xm")).unwrap();
    let limits = Limits {
        instruments: 1,
        samples: 0,
        sample_frames: 0,
        ..Default::default()
    };
    assert!(Module::parse(&only, &limits).is_ok());
    assert!(
        Module::parse(
            &only,
            &Limits {
                instruments: 0,
                ..limits
            }
        )
        .is_err()
    );
    let first = std::fs::read(root().join("first-29.xm")).unwrap();
    assert!(
        Module::parse(
            &first,
            &Limits {
                samples: 0,
                ..Default::default()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("over the 0 limit")
    );
    assert!(
        Module::parse(
            &first,
            &Limits {
                sample_frames: 0,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(
        Module::parse(
            &only,
            &Limits {
                file_bytes: only.len() - 1,
                ..Default::default()
            }
        )
        .is_err()
    );
    for (name, part) in [
        ("too-short-28", "29"),
        ("nonempty-29", "241"),
        ("truncated-29", "past the end"),
        ("huge-header", "past the end"),
    ] {
        let bytes = std::fs::read(root().join(format!("bad-{name}.xm"))).unwrap();
        let error = Module::parse(&bytes, &Default::default())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(part) || (name == "huge-header" && error.contains("overflows")),
            "{name}: {error}"
        );
    }
}

#[cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
#[cfg(feature = "player")]
fn short_empty_headers_actual_worker_preserves_alignment_seek_and_eof() {
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
