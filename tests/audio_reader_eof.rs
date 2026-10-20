#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(format: &str, bytes: Vec<u8>) -> Box<dyn AudioStream> {
    match format {
        "wav" => Box::new(
            fvid::playback_wav::WavAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        "aiff" => Box::new(
            fvid::playback_aiff::AiffAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        "au" => Box::new(
            fvid::playback_au::AuAudioReader::open(Cursor::new(bytes), Default::default()).unwrap(),
        ),
        _ => unreachable!(),
    }
}
#[test]
fn pcm_packet_reader_seek_at_or_beyond_eof_is_exhausted_instead_of_replaying_audio() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-reader-eof");
    let video = std::fs::read(base.join("video.mkv")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(video), 32 << 20).unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let expected = std::fs::read(base.join("expected.f32le")).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut wrong = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let format = case["format"].as_str().unwrap();
        let bytes = std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let mut source = open(format, bytes.clone());
        let mut decoder = source.make_decoder().unwrap();
        let mut pcm = Vec::new();
        let mut packets = Vec::new();
        while let Some(p) = source.next_packet().unwrap() {
            let frame = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            pcm.extend_from_slice(&frame.packet.data);
            packets.push((frame.packet.presentation_time(), frame.packet.data));
        }
        assert_eq!(pcm, expected, "{name} exact native PCM control");
        assert_eq!(source.duration(), Some(Duration::from_millis(20)));
        assert_eq!(source.seek_to(i64::MIN), 0);
        assert!(source.next_packet().unwrap().is_some());
        let landed = source.seek_to(1);
        assert!((0..=1).contains(&landed));
        assert!(source.next_packet().unwrap().is_some());
        let mut correct = true;
        for target in [160, 161, i64::MAX] {
            let at = source.seek_to(target);
            let replayed = source.next_packet().unwrap().is_some();
            if at != 160 || replayed {
                correct = false;
                wrong.push(format!(
                    "{name} target{target}: landed{at}, replayed={replayed}"
                ));
            }
        }
        if correct {
            capture::worker_check_with_seek(
                open(format, bytes),
                &packets,
                Some(Duration::from_millis(20)),
            );
        }
    }
    assert!(
        wrong.is_empty(),
        "EOF seek replayed valid native PCM: {wrong:?}"
    );
}
