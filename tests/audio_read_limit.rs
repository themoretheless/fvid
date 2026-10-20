#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(kind: &str, bytes: Vec<u8>, budget: usize) -> fvid::Result<Box<dyn AudioStream>> {
    let source: Box<dyn AudioStream> = match kind {
        "wav" => Box::new(fvid::playback_wav::WavAudioReader::open(
            Cursor::new(bytes),
            fvid::playback_wav::Limits {
                file_bytes: budget,
                ..Default::default()
            },
        )?),
        "aiff" => Box::new(fvid::playback_aiff::AiffAudioReader::open(
            Cursor::new(bytes),
            fvid::playback_aiff::Limits {
                file_bytes: budget,
                ..Default::default()
            },
        )?),
        "au" => Box::new(fvid::playback_au::AuAudioReader::open(
            Cursor::new(bytes),
            fvid::playback_au::Limits {
                file_bytes: budget,
                ..Default::default()
            },
        )?),
        "avi" => Box::new(fvid::playback_avi_audio::AviAudioReader::open(
            Cursor::new(bytes),
            fvid::container::avi::Limits {
                file_bytes: budget,
                ..Default::default()
            },
        )?),
        "aac" => Box::new(fvid::playback_aac::AacAudioReader::open(
            Cursor::new(bytes),
            fvid::playback_aac::Limits {
                file_bytes: budget,
                ..Default::default()
            },
        )?),
        "ac3" => Box::new(fvid::playback_ac3::Ac3AudioReader::open(
            Cursor::new(bytes),
            fvid::playback_ac3::Limits {
                file_bytes: budget,
                ..Default::default()
            },
        )?),
        "mp3" => Box::new(fvid::playback_mp3::Mp3AudioReader::open(
            Cursor::new(bytes),
            fvid::playback_mp3::Limits {
                file_bytes: budget,
                ..Default::default()
            },
        )?),
        "flac" => Box::new(fvid::playback_flac::FlacAudioReader::open(
            Cursor::new(bytes),
            fvid::playback_flac::Limits {
                file_bytes: budget,
                ..Default::default()
            },
        )?),
        "smf" => Box::new(fvid::playback_smf::SmfAudioReader::open(
            Cursor::new(bytes),
            fvid::container::smf::Limits {
                file_bytes: budget,
                ..Default::default()
            },
        )?),
        "xm" => Box::new(fvid::playback_xm::XmAudioReader::open(
            Cursor::new(bytes),
            fvid::container::xm::Limits {
                file_bytes: budget,
                ..Default::default()
            },
        )?),
        _ => unreachable!(),
    };
    Ok(source)
}
fn packets(mut source: Box<dyn AudioStream>) -> Vec<(i64, i64, Vec<u8>)> {
    let mut packets = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        packets.push((p.pts, p.duration, p.data));
    }
    source.validate_eof().unwrap();
    packets
}
#[test]
fn maximum_audio_file_budget_does_not_overflow_the_extra_byte_probe() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let base = root.join("playback-errors/audio-read-limit");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut wrong = Vec::new();
    let video = std::fs::read(base.join("video.mkv")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(video), 32 << 20).unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    for case in manifest["cases"].as_array().unwrap() {
        let kind = case["kind"].as_str().unwrap();
        let path = if let Some(existing) = case["existing_fixture"].as_str() {
            root.join(existing)
        } else {
            base.join(case["file"].as_str().unwrap())
        };
        let bytes = std::fs::read(path).unwrap();
        let normal = open(kind, bytes.clone(), 1 << 20).unwrap();
        let meta = (
            normal.sample_rate(),
            normal.channels(),
            normal.timescale(),
            normal.duration(),
        );
        assert!(meta.0 > 0 && meta.1 > 0);
        let reference = packets(normal);
        assert!(!reference.is_empty(), "{kind} valid encoded control");
        assert!(
            packets(open(kind, bytes.clone(), bytes.len()).unwrap()) == reference,
            "{kind} exact byte limit changed packets"
        );
        let error = match open(kind, bytes.clone(), bytes.len() - 1) {
            Err(error) => error,
            Ok(_) => panic!("{kind} undersized budget accepted"),
        };
        let message = error.to_string().to_lowercase();
        assert!(
            ["limit", "budget", "large", "exceed", "over"]
                .iter()
                .any(|needle| message.contains(needle)),
            "{kind} expected byte-budget refusal, got{error}"
        );
        for budget in [usize::MAX - 1, usize::MAX] {
            let source = match std::panic::catch_unwind(|| open(kind, bytes.clone(), budget)) {
                Ok(Ok(source)) => source,
                Ok(Err(error)) => {
                    wrong.push(format!(
                        "{kind} maximum{budget} refused valid input: {error}"
                    ));
                    continue;
                }
                Err(_) => {
                    wrong.push(format!(
                        "{kind} maximum{budget} overflowed extra-byte probe"
                    ));
                    continue;
                }
            };
            assert_eq!(
                (
                    source.sample_rate(),
                    source.channels(),
                    source.timescale(),
                    source.duration()
                ),
                meta,
                "{kind} maximum budget metadata"
            );
            assert!(
                packets(source) == reference,
                "{kind} maximum read changed packet bytes or clocks"
            );
        }
        for malformed in [Vec::new(), vec![0]] {
            assert!(
                open(kind, malformed, usize::MAX).is_err(),
                "{kind} malformed input accepted at maximum budget"
            );
        }
        if let Some(expected) = case["expected"].as_str() {
            let expected = std::fs::read(base.join(expected)).unwrap();
            let mut source = open(kind, bytes.clone(), 1 << 20).unwrap();
            let mut decoder = source.make_decoder().unwrap();
            let mut actual = Vec::new();
            let mut intervals = Vec::new();
            while let Some(p) = source.next_packet().unwrap() {
                let frame = decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                    .unwrap();
                actual.extend_from_slice(&frame.packet.data);
                intervals.push((frame.packet.presentation_time(), frame.packet.data));
            }
            assert_eq!(actual, expected, "{kind} exact native decode control");
            if !wrong.iter().any(|failure| failure.starts_with(kind)) {
                capture::worker_check_with_seek(
                    open(kind, bytes, usize::MAX).unwrap(),
                    &intervals,
                    Some(Duration::ZERO),
                );
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "maximum configured limit overflowed a valid synthetic source: {wrong:?}"
    );
}
