#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
fn check_codec(clock: u32, negative: bool, codec: &str) {
    let offset = if negative {
        -(i64::from(clock) / 100)
    } else {
        0
    };
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-audio-clocks");
    let bytes = std::fs::read(base.join(format!(
        "{codec}-{clock}{}.mp4",
        if negative { "-negative" } else { "" }
    )))
    .unwrap();
    let expected = std::fs::read(base.join("expected.f32le")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
            .unwrap();
    for _ in 0..3 {
        assert!(video.read_frame_raw().unwrap().is_some());
    }
    assert!(video.read_frame_raw().unwrap().is_none());
    let mut audio =
        fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    assert_eq!(audio.sample_rate(), 8000);
    assert_eq!(audio.timescale(), clock);
    let mut decoder = audio.make_decoder().unwrap();
    for pass in 0..3 {
        if pass == 1 {
            AudioStream::rewind(&mut audio);
            decoder.reset();
        }
        if pass == 2 {
            let target = i64::from(clock) * 2 / 50 + offset;
            assert_eq!(audio.seek_to(target), target);
            decoder.reset();
        }
        let first = if pass == 2 { 2 } else { 0 };
        let mut samples = Vec::new();
        for index in first..3 {
            let source = audio.next_packet().unwrap().unwrap();
            assert_eq!(source.pts, index * i64::from(clock) / 50 + offset);
            let frame = decoder
                .decode_packet(&source.data, source.pts, source.duration as u64)
                .unwrap()
                .unwrap();
            let packet = audio
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
                .unwrap();
            assert_eq!(
                packet.presentation_time(),
                Duration::from_millis((index * 20 - if negative { 10 } else { 0 }).max(0) as u64),
                "media clock{clock}, pass{pass}"
            );
            samples.extend(packet.data);
        }
        assert!(audio.next_packet().unwrap().is_none());
        audio.validate_eof().unwrap();
        let start = if pass == 2 {
            320 * 4
        } else if negative {
            80 * 4
        } else {
            0
        };
        assert_eq!(samples, expected[start..]);
    }
}
fn check_offset(clock: u32, negative: bool) {
    for codec in ["pcm", "alac"] {
        check_codec(clock, negative, codec);
    }
}
fn check(clock: u32) {
    check_offset(clock, false);
}
#[test]
fn negative_composition_audio_start() {
    check_offset(1000, true);
}
#[test]
fn sample_clock_control() {
    check(8000);
}
#[test]
fn millisecond_media_clock() {
    check(1000);
}
#[test]
fn unrelated_audio_rate_clock() {
    check(44100);
}
#[test]
fn standard_audio_clock() {
    check(48000);
}
#[test]
fn video_rate_media_clock() {
    check(90000);
}
#[test]
fn decoded_geometry_and_fractional_negative_preroll_are_bounded() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-audio-clocks/pcm-44100.mp4");
    let audio = fvid::playback_mp4_audio::Mp4AudioReader::open(
        Cursor::new(std::fs::read(base).unwrap()),
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
            .contains("invalid MP4 decoded audio geometry")
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
    let data: Vec<u8> = (0..16).collect();
    let clipped = audio
        .present_decoded(packet(data.clone()), -1)
        .unwrap()
        .unwrap();
    assert_eq!(clipped.data, data[4..]);
    assert_eq!(
        clipped.presentation_time(),
        Duration::from_nanos((44100 - 8000) * 1_000_000_000 / (44100 * 8000))
    );
}

#[test]
fn seek_inside_pcm_and_alac_packets_trims_to_first_sample_at_or_after_target() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-audio-clocks");
    let expected = std::fs::read(base.join("expected.f32le")).unwrap();
    for codec in ["pcm", "alac"] {
        for clock in [1000u32, 8000, 44100, 48000, 90000] {
            let name = if clock == 1000 {
                format!("{codec}-seek-inside.mp4")
            } else {
                format!("{codec}-{clock}.mp4")
            };
            let bytes = std::fs::read(base.join(name)).unwrap();
            let mut video =
                fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                    .unwrap();
            assert!(video.read_frame_raw().unwrap().is_some());
            let mut audio = fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(bytes),
                Default::default(),
            )
            .unwrap();
            for target in [
                1,
                i64::from(clock) / 100 + 1,
                i64::from(clock) * 3 / 100 + 1,
            ] {
                let first = (target as u128 * 8000).div_ceil(u128::from(clock)) as usize;
                audio.seek_to(target);
                let mut decoder = audio.make_decoder().unwrap();
                let mut actual = Vec::new();
                while let Some(p) = audio.next_packet().unwrap() {
                    let f = decoder
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap()
                        .unwrap();
                    if let Some(out) = audio.present_decoded(f.packet, f.source_pts).unwrap() {
                        if actual.is_empty() {
                            assert_eq!(
                                out.presentation_time(),
                                Duration::from_nanos(first as u64 * 1_000_000_000 / 8000),
                                "{codec}/{clock}/{target}"
                            );
                        }
                        actual.extend(out.data);
                    }
                }
                assert_eq!(actual, expected[first * 4..], "{codec}/{clock}/{target}");
            }
            AudioStream::rewind(&mut audio);
            let p = audio.next_packet().unwrap().unwrap();
            let mut decoder = audio.make_decoder().unwrap();
            let f = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            assert_eq!(
                audio
                    .present_decoded(f.packet, f.source_pts)
                    .unwrap()
                    .unwrap()
                    .data,
                expected[..160 * 4]
            );
        }
    }
}
