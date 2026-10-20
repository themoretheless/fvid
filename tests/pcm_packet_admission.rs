#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn playback_pcm_never_silently_drops_partial_frames_or_emits_nonfinite_samples() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/pcm-packet-admission");
    let mut admitted = Vec::new();
    for (name, reason) in [
        ("valid-int", None),
        ("valid-float", None),
        ("short-byte", Some("incomplete")),
        ("short-channel", Some("incomplete")),
        ("nan", Some("non-finite")),
        ("infinity", Some("non-finite")),
    ] {
        let media = std::fs::read(base.join(format!("{name}.mkv"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(media.clone()), 32 << 20)
                .unwrap();
        for _ in 0..3 {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut source = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(media.clone()),
            Default::default(),
        )
        .unwrap();
        assert_eq!(source.channels(), 2);
        assert_eq!(source.sample_rate(), 8000);
        let mut decoder = source.make_decoder().unwrap();
        // The reader needs a complete PCM frame to resolve coded timing;
        // malformed geometry can now be refused before the decoder runs.
        let decoded = match source.next_packet() {
            Ok(Some(encoded)) => {
                decoder.decode_packet(&encoded.data, encoded.pts, encoded.duration as u64)
            }
            Ok(None) => Ok(None),
            Err(error) => Err(error),
        };
        match (decoded, reason) {
            (Ok(Some(frame)), None) => {
                let expected = std::fs::read(base.join(format!("{name}-expected.f32le"))).unwrap();
                assert_eq!(frame.packet.data, expected);
                let source = fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(media),
                    Default::default(),
                )
                .unwrap();
                capture::worker_check(Box::new(source), &[(std::time::Duration::ZERO, expected)]);
            }
            (Err(error), Some(needle)) => {
                assert!(error.to_string().contains(needle), "{name}: {error}");
                let source = fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(media),
                    Default::default(),
                )
                .unwrap();
                capture::worker_refusal(Box::new(source), needle);
            }
            (Ok(_), Some(_)) => admitted.push(name),
            (Err(error), None) => panic!("valid control refused: {error}"),
            (Ok(None), None) => panic!("valid PCM did not produce output"),
        }
    }
    assert!(
        admitted.is_empty(),
        "malformed PCM silently admitted: {admitted:?}"
    );
}
