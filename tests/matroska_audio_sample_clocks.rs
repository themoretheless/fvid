#![cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod audio_capture;
use audio_capture::worker_check;
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
fn check(name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-audio-sample-clocks")
        .join(format!("{name}.mkv"));
    let bytes = std::fs::read(path).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
            .unwrap();
    let mut frames = 0;
    while video.read_frame_raw().unwrap().is_some() {
        frames += 1;
    }
    assert_eq!(frames, 12);
    let mut audio =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    let mut decoder = audio.make_decoder().unwrap();
    let mut checked = 0;
    let mut reference = Vec::new();
    for pass in 0..2 {
        if pass != 0 {
            AudioStream::rewind(&mut audio);
            decoder.reset();
        }
        while let Some(source) = audio.next_packet().unwrap() {
            if let Some(frame) = decoder
                .decode_packet(&source.data, source.pts, source.duration.max(0) as u64)
                .unwrap()
            {
                if let Some(packet) = audio
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    assert!(!packet.data.is_empty());
                    if pass == 0 {
                        reference.push((packet.presentation_time(), packet.data.clone()));
                    }
                    if frame.source_pts >= 0 {
                        // MPEG/AC3 restore codec sample coordinates beneath
                        // integer-ns metadata. Source rounding may be 1 ns above
                        // the exact sample; compare the rational-floor point,
                        // without widening the clock tolerance.
                        let expected = if matches!(name, "mp2" | "mp3" | "ac3") {
                            let rate = u128::from(audio.sample_rate());
                            assert_eq!(rate, 48_000);
                            let sample =
                                (frame.source_pts as u128 * rate + 500_000_000) / 1_000_000_000;
                            Duration::from_nanos((sample * 1_000_000_000 / rate) as u64)
                        } else {
                            Duration::from_nanos(frame.source_pts as u64)
                        };
                        assert_eq!(packet.timebase_num, 1);
                        assert_eq!(packet.timebase_den, 1_000_000_000);
                        assert_eq!(
                            packet.presentation_time(),
                            expected,
                            "{name}: source {}",
                            frame.source_pts
                        );
                        checked += 1;
                    }
                }
            }
        }
        audio.validate_eof().unwrap();
    }
    AudioStream::rewind(&mut audio);
    worker_check(Box::new(audio), &reference);
    assert!(
        checked >= 4,
        "{name}: insufficient timed decoded packets {checked}"
    );
}
#[test]
fn flac_container_sample_clock() {
    check("flac");
}
#[test]
fn alac_container_sample_clock() {
    check("alac");
}
#[test]
fn mp3_container_sample_clock() {
    check("mp3");
}
#[test]
fn mp2_container_sample_clock() {
    check("mp2");
}
#[test]
fn vorbis_container_sample_clock() {
    check("vorbis");
}
#[test]
fn ac3_container_sample_clock() {
    check("ac3");
}
