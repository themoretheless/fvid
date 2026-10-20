#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
fn check(name: &str, offset: i64) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-pcm-clock");
    let bytes = std::fs::read(base.join(format!("{name}.mkv"))).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
            .unwrap();
    for _ in 0..3 {
        assert!(video.read_frame_raw().unwrap().is_some());
    }
    assert!(video.read_frame_raw().unwrap().is_none());
    let expected = std::fs::read(base.join(format!("{name}.f32le"))).unwrap();
    let mut audio =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    let mut decoder = audio.make_decoder().unwrap();
    for pass in 0..3 {
        if pass == 1 {
            AudioStream::rewind(&mut audio);
            decoder.reset();
        }
        if pass == 2 {
            let target = 40_000_000 + offset;
            assert_eq!(audio.seek_to(target), target);
            decoder.reset();
        }
        let first = if pass == 2 { 2 } else { 0 };
        let mut actual = Vec::new();
        for index in first..3 {
            let source = audio.next_packet().unwrap().unwrap();
            assert_eq!(source.pts, index * 20_000_000 + offset);
            let decoded = decoder
                .decode_encoded(
                    &source.data,
                    source.pts.max(0) as u64,
                    source.duration as u64,
                )
                .unwrap()
                .unwrap();
            let presented = audio.present_decoded(decoded, source.pts).unwrap().unwrap();
            assert_eq!(
                presented.presentation_time(),
                Duration::from_nanos(source.pts.max(0) as u64),
                "{name} PCM packet{index}, pass{pass}"
            );
            actual.extend(presented.data);
        }
        assert!(audio.next_packet().unwrap().is_none());
        audio.validate_eof().unwrap();
        let start = if pass == 2 {
            expected.len() - 160 * 4
        } else {
            0
        };
        assert_eq!(actual, expected[start..], "{name}, pass{pass}");
    }
}
#[test]
fn single_pcm_stream_preserves_nanosecond_clock_samples_and_seek() {
    for name in ["clock", "big", "float"] {
        check(name, 0);
    }
}
#[test]
fn negative_pcm_start_clips_only_samples_before_zero() {
    check("negative", -10_000_000);
}
#[test]
fn pcm_presentation_rejects_partial_frames_and_bounds_negative_preroll() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-pcm-clock/clock.mkv");
    let audio = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(std::fs::read(path).unwrap()),
        Default::default(),
    )
    .unwrap();
    let packet = |data| fvid::audio::AudioPacket {
        data,
        pts: 0,
        timebase_num: 1,
        timebase_den: 8000,
    };
    assert!(
        audio
            .present_decoded(packet(vec![0; 3]), 0)
            .err()
            .unwrap()
            .to_string()
            .contains("invalid Matroska PCM geometry")
    );
    assert!(
        audio
            .present_decoded(packet(vec![0; 4]), i64::MIN)
            .unwrap()
            .is_none()
    );
    assert!(
        audio
            .present_decoded(packet(Vec::new()), 0)
            .unwrap()
            .is_none()
    );
    let at = audio
        .present_decoded(packet(vec![0; 4]), i64::MAX)
        .unwrap()
        .unwrap();
    assert_eq!(
        at.presentation_time(),
        Duration::from_nanos(i64::MAX as u64)
    );
}
