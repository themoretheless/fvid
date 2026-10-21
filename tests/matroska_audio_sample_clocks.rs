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
    while let Some(raw) = video.read_frame_raw().unwrap() {
        let rgb = raw.into_rgb(32 << 20).unwrap();
        assert_eq!(rgb, vec![130; 16 * 16 * 3], "{name}: owned neutral video");
        assert_eq!(video.frame_interval(), Some((frames * 20_000_000, (frames + 1) * 20_000_000, 1_000_000_000)));
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
        let mut alac_packet = 0;
        if pass != 0 {
            AudioStream::rewind(&mut audio);
            decoder.reset();
        }
        while let Some(source) = audio.next_packet().unwrap() {
            if let Some(frame) = decoder
                .decode_packet(&source.data, source.pts, source.duration.max(0) as u64)
                .unwrap()
            {
                let alac_time = if name == "alac" {
                    // The authored 240 ms / 48 kHz fixture has two full 4096-
                    // sample ALAC frames and a 3328-sample tail. Matroska rounds
                    // their block times to ms; presentation restores samples.
                    let (source, point, samples) = [
                        (0, 0, 4096),
                        (85_000_000, 85_333_333, 4096),
                        (171_000_000, 170_666_666, 3328),
                    ][alac_packet];
                    assert_eq!(frame.source_pts, source);
                    assert_eq!(frame.packet.data.len() / (4 * audio.channels() as usize), samples);
                    alac_packet += 1;
                    Some(Duration::from_nanos(point))
                } else { None };
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
                        let expected = if let Some(point) = alac_time { point }
                        else if matches!(name, "mp2" | "mp3" | "ac3") {
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
        if name == "alac" { assert_eq!(alac_packet, 3); }
        audio.validate_eof().unwrap();
    }
    AudioStream::rewind(&mut audio);
    let samples: Vec<_> = reference.iter().flat_map(|(_, data)| data.chunks_exact(4))
        .map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
    assert!(samples.iter().all(|s| s.is_finite()), "{name}: finite reference tone");
    assert!(samples.iter().filter(|s| s.abs() > 0.000_001).count() > 960, "{name}: nonzero audio coverage must not become silence");
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
