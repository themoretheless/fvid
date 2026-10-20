#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
#[test]
fn flac_container_offset_and_seek_preserve_source_sample_positions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let bytes = std::fs::read(root.join("flac-matroska/16-1-verbatim-0-offset.mkv")).unwrap();
    let integer = std::fs::read(root.join("flac-native/16-1-verbatim-0.s32le")).unwrap();
    let golden: Vec<u8> = integer
        .chunks_exact(4)
        .flat_map(|v| (i32::from_le_bytes(v.try_into().unwrap()) as f32 / 32768.).to_le_bytes())
        .collect();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
            .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    for target in [0, 51_000_000, 51_000_001] {
        let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        )
        .unwrap();
        reader.seek_to(target);
        let mut decoder = reader.make_decoder().unwrap();
        let p = reader.next_packet().unwrap().unwrap();
        let f = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        let out = reader
            .present_decoded(f.packet, f.source_pts)
            .unwrap()
            .unwrap();
        let first = ((target - 50_000_000).max(0) as u128 * 8000).div_ceil(1_000_000_000) as usize;
        assert_eq!(out.pts, 50_000_000 + first as u64 * 125_000);
        assert_eq!(out.data, &golden[first * 8..]);
    }
}

#[test]
fn multi_packet_flac_clock_matches_coded_samples_under_container_quantization() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/flac-timeline");
    for name in ["unit", "third"] {
        let bytes = std::fs::read(root.join(format!("{name}.mkv"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        for _ in 0..4 {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            reader.duration(),
            Some(std::time::Duration::from_nanos(256 * 1_000_000_000 / 44100)),
            "{name} before decode"
        );
        let mut decoder = reader.make_decoder().unwrap();
        for (i, level) in [4096, -4096, 8192, -8192].into_iter().enumerate() {
            let p = reader.next_packet().unwrap().unwrap();
            let f = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            let out = reader
                .present_decoded(f.packet, f.source_pts)
                .unwrap()
                .unwrap();
            assert_eq!(out.pts, i as u64 * 64 * 1_000_000_000 / 44100, "{name}/{i}");
            let expected: Vec<u8> = (0..64)
                .flat_map(|_| (level as f32 / 32768.).to_le_bytes())
                .collect();
            assert_eq!(out.data, expected);
        }
        assert!(reader.next_packet().unwrap().is_none());
        let duration = Some(std::time::Duration::from_nanos(256 * 1_000_000_000 / 44100));
        assert_eq!(reader.duration(), duration, "{name} EOF duration");
        reader.rewind();
        assert_eq!(reader.duration(), duration, "{name} rewind duration");
        reader.seek_to(2_000_001);
        assert_eq!(reader.duration(), duration, "{name} seek duration");
    }
}

#[test]
fn flac_eof_duration_excludes_partial_and_fully_discarded_last_packets() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/flac-timeline");
    for (name, tail) in [("tail", 13), ("discard-last", 64)] {
        let bytes = std::fs::read(root.join(format!("{name}.mkv"))).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        for _ in 0..4 {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            reader.duration(),
            Some(std::time::Duration::from_nanos(
                (256 - tail) as u64 * 1_000_000_000 / 44100
            )),
            "{name} initial duration"
        );
        let mut decoder = reader.make_decoder().unwrap();
        let mut samples = 0;
        while let Some(p) = reader.next_packet().unwrap() {
            let f = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                samples += out.data.len() / 4;
            }
        }
        assert_eq!(samples, 256 - tail);
        let expected = Some(std::time::Duration::from_nanos(
            (256 - tail) as u64 * 1_000_000_000 / 44100,
        ));
        assert_eq!(reader.duration(), expected, "{name}");
        reader.rewind();
        assert_eq!(reader.duration(), expected);
        reader.seek_to(1);
        assert_eq!(reader.duration(), expected);
    }
}
