use std::path::PathBuf;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avi-riff-boundary")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
fn cases(key: &str) -> Vec<serde_json::Value> {
    manifest()[key].as_array().unwrap().clone()
}
fn parsed(name: &str) -> fvid::container::avi::Avi {
    fvid::container::avi::Avi::parse(&bytes(name), &Default::default()).unwrap()
}
#[test]
fn complete_and_existing_truncated_root_profiles_keep_their_records() {
    for case in cases("controls") {
        let n = case["file"].as_str().unwrap();
        let p = parsed(n);
        assert_eq!(p.streams, 2);
        assert_eq!(p.audio()[0].records.len(), 3);
    }
}

#[test]
fn mandatory_avi_lists_must_be_inside_the_declared_riff() {
    for c in cases("cases") {
        let n = c["file"].as_str().unwrap();
        let e = match fvid::container::avi::Avi::parse(&bytes(n), &Default::default()) {
            Ok(_) => panic!("{n}: borrowed data outside RIFF"),
            Err(e) => e.to_string(),
        };
        assert!(
            e.contains("RIFF") || e.contains("no header list") || e.contains("no movi list"),
            "{n}:{e}"
        );
    }
}
#[test]
fn outside_index_cannot_change_the_first_riff_record_index() {
    for c in cases("outside") {
        let n = c["file"].as_str().unwrap();
        let p = parsed(n);
        assert_eq!(p.streams, 2);
        assert_eq!(p.audio()[0].records.len(), 3);
        let exact = fvid::container::avi::Limits {
            file_bytes: bytes(n).len(),
            ..Default::default()
        };
        assert!(fvid::container::avi::Avi::parse(&bytes(n), &exact).is_ok());
        assert!(
            fvid::container::avi::Avi::parse(
                &bytes(n),
                &fvid::container::avi::Limits {
                    file_bytes: exact.file_bytes - 1,
                    ..exact
                }
            )
            .is_err()
        );
    }
}
#[cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
#[cfg(feature = "player")]
mod playback {
    use super::*;
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    fn source(name: &str) -> fvid::playback_avi_audio::AviAudioReader {
        fvid::playback_avi_audio::AviAudioReader::open(Cursor::new(bytes(name)), Default::default())
            .unwrap()
    }
    fn packets(name: &str) -> Vec<(Duration, Vec<u8>)> {
        let mut source = source(name);
        let mut decoder = source.make_decoder().unwrap();
        let mut out = Vec::new();
        while let Some(packet) = source.next_packet().unwrap() {
            let decoded = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
                .unwrap();
            out.push((decoded.packet.presentation_time(), decoded.packet.data));
        }
        out
    }
    fn video_control() {
        let mut video = fvid::playback_native::NativeReader::software(
            Cursor::new(bytes("video.mkv")),
            32 << 20,
        )
        .unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());
        assert!(video.read_frame_raw().unwrap().is_none());
    }

    fn check(case: &serde_json::Value, worker: bool) {
        let n = case["file"].as_str().unwrap();
        let expected = packets(n);
        let input = source(n);
        let frames = case["frames"].as_i64().unwrap();
        assert_eq!(input.duration(), Some(input.time_of(frames)));
        assert_eq!(
            expected
                .iter()
                .flat_map(|(_, p)| p.iter().copied())
                .collect::<Vec<_>>(),
            bytes(case["expected"].as_str().unwrap())
        );
        if worker {
            capture::worker_check_with_seek(Box::new(source(n)), &expected, Some(expected[1].0));
            capture::worker_check_with_seek(
                Box::new(source(n)),
                &expected,
                Some(input.time_of(frames)),
            );
        }
    }
    #[test]
    fn healthy_root_profiles_keep_literal_pcm_and_worker_navigation() {
        video_control();
        for c in cases("controls") {
            check(&c, true);
        }
    }

    #[test]
    fn outside_lists_are_refused_before_audio_source_admission() {
        video_control();
        for c in cases("cases") {
            let n = c["file"].as_str().unwrap();
            let e = match fvid::playback_avi_audio::AviAudioReader::open(
                Cursor::new(bytes(n)),
                Default::default(),
            ) {
                Ok(_) => panic!("{n}: outside lists reached audio"),
                Err(e) => e.to_string(),
            };
            assert!(
                e.contains("RIFF") || e.contains("no header list") || e.contains("no movi list"),
                "{n}:{e}"
            );
        }
    }
    #[test]
    fn complete_first_riff_with_outside_index_keeps_native_pcm_and_worker_navigation() {
        video_control();
        for c in cases("outside") {
            check(&c, true);
        }
    }
}
