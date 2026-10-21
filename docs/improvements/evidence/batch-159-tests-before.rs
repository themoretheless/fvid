#[cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avi-index-origin")
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
fn parsed(name: &str) -> fvid::container::avi::Avi {
    fvid::container::avi::Avi::parse(&bytes(name), &Default::default()).unwrap()
}
#[test]
fn baseline_isolates_absolute_origin_refusal_from_record_data() {
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let control = parsed(case["control"].as_str().unwrap());
        assert_eq!(control.streams, 2);
        assert_eq!(control.audio()[0].records.len(), 3);
        let data = bytes(name);
        let actual = fvid::container::avi::Avi::parse(&data, &Default::default());
        if case["before_failure"].as_bool().unwrap() {
            let e = actual.unwrap_err().to_string();
            assert!(e.contains("record 0 sits at"), "{name}: {e}");
            eprintln!("specific baseline refusal {name}: {e}");
        } else {
            assert_eq!(
                actual.unwrap().audio()[0].records,
                control.audio()[0].records
            );
        }
    }
}
#[test]
fn malformed_indexes_and_record_budget_still_refuse() {
    for case in manifest()["malformed"].as_array().unwrap() {
        let name = case.as_str().unwrap();
        let error = fvid::container::avi::Avi::parse(&bytes(name), &Default::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("record"), "{name}: {error}");
    }
    let limits = fvid::container::avi::Limits {
        records: 2,
        ..Default::default()
    };
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let error = fvid::container::avi::Avi::parse(&bytes(name), &limits)
            .unwrap_err()
            .to_string();
        assert!(error.contains("more than 2 records"), "{name}: {error}");
    }
}
#[test]
#[ignore = "enable when both documented origins are supported"]
fn both_index_origins_preserve_the_same_validated_records() {
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let actual = parsed(name);
        let control = parsed(case["control"].as_str().unwrap());
        assert_eq!(actual.streams, control.streams);
        let a = &actual.audio()[0];
        let c = &control.audio()[0];
        assert_eq!(a.records, c.records, "{name}");
        assert_eq!(
            (a.format, a.scale, a.rate, a.declared),
            (c.format, c.scale, c.rate, c.declared)
        );
        for i in 0..3 {
            assert_eq!(
                a.record(&bytes(name), i),
                c.record(&bytes(case["control"].as_str().unwrap()), i)
            );
        }
    }
}
#[cfg(feature = "player")]
mod playback {
    use super::capture;
    use super::*;
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    fn video_control() {
        let mut video = fvid::playback_native::NativeReader::software(
            Cursor::new(bytes("video.mkv")),
            32 << 20,
        )
        .unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());
        assert!(video.read_frame_raw().unwrap().is_none());
    }
    fn source(name: &str) -> fvid::playback_avi_audio::AviAudioReader {
        fvid::playback_avi_audio::AviAudioReader::open(Cursor::new(bytes(name)), Default::default())
            .unwrap()
    }
    fn packets(name: &str) -> Vec<(Duration, Vec<u8>)> {
        let mut input = source(name);
        let mut decoder = input.make_decoder().unwrap();
        let mut out = Vec::new();
        while let Some(packet) = input.next_packet().unwrap() {
            let output = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
                .unwrap();
            out.push((output.packet.presentation_time(), output.packet.data));
        }
        out
    }
    #[test]
    fn independent_unindexed_controls_decode_literal_predictors() {
        video_control();
        for case in manifest()["cases"].as_array().unwrap() {
            let name = case["control"].as_str().unwrap();
            let pcm: Vec<_> = packets(name).into_iter().flat_map(|(_, pcm)| pcm).collect();
            assert_eq!(pcm, bytes(case["expected"].as_str().unwrap()), "{name}");
        }
    }
    #[test]
    #[ignore = "enable with origin fix"]
    fn indexed_audio_matches_literal_pcm_and_actual_worker_navigation() {
        video_control();
        for case in manifest()["cases"].as_array().unwrap() {
            let name = case["file"].as_str().unwrap();
            let expected = packets(case["control"].as_str().unwrap());
            assert_eq!(packets(name), expected, "{name}");
            let literal = bytes(case["expected"].as_str().unwrap());
            assert_eq!(
                expected
                    .iter()
                    .flat_map(|(_, p)| p.iter().copied())
                    .collect::<Vec<_>>(),
                literal
            );
            let frames = case["frames"].as_i64().unwrap();
            let block = case["block_frames"].as_i64().unwrap();
            let mut input = source(name);
            assert_eq!(input.duration(), Some(input.time_of(frames)));
            assert_eq!(input.seek_to(block + 1), block);
            let packet = input.next_packet().unwrap().unwrap();
            assert_eq!((packet.pts, packet.duration), (block, block));
            for at in [frames, frames + 1, i64::MAX] {
                assert_eq!(input.seek_to(at), frames);
                assert!(input.next_packet().unwrap().is_none());
            }
            capture::worker_check_with_seek(
                Box::new(source(name)),
                &expected,
                Some(Duration::from_nanos(block as u64 * 1_000_000_000 / 8000)),
            );
            capture::worker_check_with_seek(
                Box::new(source(name)),
                &expected,
                Some(input.time_of(frames)),
            );
        }
    }
}
