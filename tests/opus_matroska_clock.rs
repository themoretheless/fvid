#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
#[test]
fn opus_one_tick_gaps_and_positive_origins_are_not_rounding_noise() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/opus-clock");
    for (name, times) in [("gap", [0, 21, 41]), ("offset", [1, 21, 41])] {
        let bytes = std::fs::read(root.join(format!("{name}.mkv"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        for _ in 0..3 {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )
        .unwrap();
        let mut decoder = reader.make_decoder().unwrap();
        for stamp in times {
            let p = reader.next_packet().unwrap().unwrap();
            let f = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            let out = reader
                .present_decoded(f.packet, f.source_pts)
                .unwrap()
                .unwrap();
            assert_eq!(out.pts, stamp * 1_000_000, "{name}");
            assert_eq!(out.data.len(), 960 * 4);
        }
        assert!(reader.next_packet().unwrap().is_none());
        assert_eq!(
            reader.duration(),
            Some(std::time::Duration::from_millis(61)),
            "{name} duration"
        );
    }
}

#[test]
fn short_opus_packets_recover_fractional_container_grid() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/opus-clock");
    for (name, count) in [
        ("short-third", 4),
        ("short-shift-third", 4),
        ("short-lace-third", 2),
    ] {
        let bytes = std::fs::read(root.join(format!("{name}.mkv"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        for _ in 0..count {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        assert!(video.read_frame_raw().unwrap().is_none());
        let control_reader = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        )
        .unwrap();
        let mut control = control_reader.make_decoder().unwrap();
        let mut expected = Vec::new();
        for i in 0..4 {
            let f = control
                .decode_packet(&[0x80, 0xff, 0xfe], i * 2_500_000, 2_500_000)
                .unwrap()
                .unwrap();
            assert_eq!(f.packet.data.len(), 120 * 4);
            expected.extend(f.packet.data);
        }
        for target in [0, 3_000_000, 3_000_001, 7_500_001] {
            let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap();
            assert_eq!(reader.seek_to(target), target);
            let mut decoder = reader.make_decoder().unwrap();
            let mut actual = Vec::new();
            let mut stamps = Vec::new();
            while let Some(p) = reader.next_packet().unwrap() {
                let f = decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                    .unwrap();
                assert_eq!(f.packet.data.len(), 120 * 4);
                if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                    stamps.push(out.pts);
                    actual.extend(out.data);
                }
            }
            let first = (target as u128 * 48000).div_ceil(1_000_000_000) as usize;
            let times: Vec<_> = (0..4)
                .filter_map(|i| {
                    let start = (i * 120).max(first);
                    (start < (i + 1) * 120).then_some(start as u64 * 1_000_000_000 / 48000)
                })
                .collect();
            assert_eq!(stamps, times, "{name}/{target}");
            assert_eq!(actual, &expected[first * 4..], "{name}/{target}");
            assert_eq!(
                reader.duration(),
                Some(std::time::Duration::from_millis(10)),
                "{name}"
            );
        }
    }
}

#[test]
fn opus_rounded_delay_and_signed_padding_keep_exact_seek_samples() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/opus-clock");
    for (name, head) in [("short-delay-tail", 0), ("short-delay-head-tail", 9)] {
        let bytes = std::fs::read(root.join(format!("{name}.mkv"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        for _ in 0..4 {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        assert!(video.read_frame_raw().unwrap().is_none());
        let control_reader = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        )
        .unwrap();
        let mut control = control_reader.make_decoder().unwrap();
        let mut source = Vec::new();
        for i in 0..4 {
            let f = control
                .decode_packet(&[0x80, 0xff, 0xfe], i * 2_500_000, 2_500_000)
                .unwrap()
                .unwrap();
            assert_eq!(f.packet.data.len(), 120 * 4);
            source.extend(f.packet.data);
        }
        for target in [0, 1_000_000, 1_000_001, 5_000_001] {
            let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap();
            reader.seek_to(target);
            let mut decoder = reader.make_decoder().unwrap();
            let mut times = Vec::new();
            let mut actual = Vec::new();
            let mut frames = 0;
            while let Some(p) = reader.next_packet().unwrap() {
                let f = decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                    .unwrap();
                if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                    times.push(out.pts);
                    frames += out.data.len() / 4;
                    actual.extend(out.data);
                }
            }
            let floor = (target as u128 * 48000).div_ceil(1_000_000_000) as i64;
            let mut wanted = Vec::new();
            let mut count = 0;
            let mut expected = Vec::new();
            for i in 0..4 {
                let start = i * 120 - 17;
                let first = (floor - start)
                    .clamp(0, 120)
                    .max(if i == 2 { head } else { 0 });
                let last = if i == 3 { 107 } else { 120 };
                if first < last {
                    wanted.push((start + first) as u64 * 1_000_000_000 / 48000);
                    count += last - first;
                    expected.extend_from_slice(
                        &source[(i * 120 + first) as usize * 4..(i * 120 + last) as usize * 4],
                    );
                }
            }
            assert_eq!(times, wanted, "{name}/{target}");
            assert_eq!(frames, count as usize, "{name}/{target}");
            assert_eq!(actual, expected, "{name}/{target} PCM");
            assert_eq!(
                reader.duration(),
                Some(std::time::Duration::from_micros(9375))
            );
        }
    }
}
