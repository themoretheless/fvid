use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avi-main-header-bounds")
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
#[test]
fn complete_main_headers_keep_their_own_stream_count() {
    for case in manifest()["controls"].as_array().unwrap() {
        let parsed = fvid::container::avi::Avi::parse(
            &bytes(case["file"].as_str().unwrap()),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(parsed.streams, 2);
        assert_eq!(parsed.audio()[0].records.len(), 3);
    }
}
#[test]
fn short_main_headers_cannot_borrow_a_stream_count_from_junk() {
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let result = fvid::container::avi::Avi::parse(&bytes(name), &Default::default());
        assert!(
            result.is_err(),
            "{name}: header borrowed dwStreams from next chunk"
        );
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("main header") && error.contains("stream count"),
            "{name}: {error}"
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
    #[test]
    fn complete_headers_keep_literal_pcm_and_worker_navigation() {
        video_control();
        for case in manifest()["controls"].as_array().unwrap() {
            let name = case["file"].as_str().unwrap();
            let expected = packets(name);
            assert_eq!(
                expected
                    .iter()
                    .flat_map(|(_, p)| p.iter().copied())
                    .collect::<Vec<_>>(),
                bytes(case["expected"].as_str().unwrap())
            );
            let input = source(name);
            let frames = case["frames"].as_i64().unwrap();
            assert_eq!(input.duration(), Some(input.time_of(frames)));
            super::capture::worker_check_with_seek(
                Box::new(source(name)),
                &expected,
                Some(expected[1].0),
            );
            super::capture::worker_check_with_seek(
                Box::new(source(name)),
                &expected,
                Some(input.time_of(frames)),
            );
        }
    }
    #[test]
    fn malformed_headers_are_refused_before_audio_source_admission() {
        video_control();
        for case in manifest()["cases"].as_array().unwrap() {
            let name = case["file"].as_str().unwrap();
            let error = match fvid::playback_avi_audio::AviAudioReader::open(
                Cursor::new(bytes(name)),
                Default::default(),
            ) {
                Ok(_) => panic!("{name}: source accepted a fabricated main stream count"),
                Err(e) => e.to_string(),
            };
            assert!(
                error.contains("main header") && error.contains("stream count"),
                "{name}: {error}"
            );
        }
    }
}
