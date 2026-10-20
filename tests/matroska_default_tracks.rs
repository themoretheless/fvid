use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-default-tracks")
}
#[test]
fn default_second_video_is_selected_automatically() {
    let mut reader = fvid::playback_native::NativeReader::software(
        Cursor::new(fs::read(root().join("prefer-second.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert_eq!(
        reader
            .read_frame_raw()
            .unwrap()
            .unwrap()
            .into_rgb(32 << 20)
            .unwrap()
            .len(),
        64 * 64 * 3
    );
    let stats = fvid_media::decode_video(&root().join("prefer-second.mkv")).unwrap();
    assert_eq!((stats.width, stats.height), (64, 64));
}
#[test]
fn unique_default_audio_decodes_without_an_explicit_index() {
    let mut output = Vec::new();
    let stats = fvid_media::owned_matroska_pcm::decode_matroska_pcm(
        Cursor::new(fs::read(root().join("audiovisual.mkv")).unwrap()),
        &mut output,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 480);
    assert_eq!(
        output,
        fs::read(root().join("expected-audio.f32le")).unwrap()
    );
}
#[test]
fn equal_priority_and_first_default_controls_keep_file_order() {
    for name in [
        "prefer-first.mkv",
        "both-default.mkv",
        "no-default.mkv",
        "missing-first.mkv",
    ] {
        let stats = fvid_media::decode_video(&root().join(name)).unwrap();
        assert_eq!((stats.width, stats.height, stats.video_frames), (16, 16, 3));
    }
}
#[test]
fn empty_default_flag_uses_its_schema_default() {
    let reader = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(fs::read(root().join("empty-first.mkv")).unwrap()),
        Default::default(),
    )
    .unwrap();
    assert_ne!(reader.track_dispositions[&1] & 1, 0);
}
#[test]
fn default_candidates_preserve_pixels_clocks_seek_and_eof() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let source = root().join(case["file"].as_str().unwrap());
        let Some(side) = case["side"].as_u64() else {
            assert!(
                fvid::playback_native::NativeReader::software(
                    std::io::Cursor::new(fs::read(&source).unwrap()),
                    32 << 20
                )
                .is_err()
            );
            assert!(fvid_media::decode_video(&source).is_err());
            continue;
        };
        let stats = fvid_media::decode_video(&source).unwrap();
        assert_eq!(
            (stats.width, stats.height, stats.video_frames),
            (side as u32, side as u32, 3)
        );
        let mut reader = fvid::playback_native::NativeReader::software(
            std::io::Cursor::new(fs::read(&source).unwrap()),
            32 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            for index in 0..3 {
                let raw = reader.read_frame_raw().unwrap().unwrap();
                let rgb = raw.into_rgb(32 << 20).unwrap();
                assert_eq!(
                    rgb,
                    vec![130; side as usize * side as usize * 3],
                    "{}",
                    case["file"]
                );
                assert_eq!(
                    reader.frame_interval().unwrap(),
                    (
                        index * 20_000_000,
                        index * 20_000_000 + 20_000_000,
                        1_000_000_000
                    )
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        for index in [2, 0, 1, 2] {
            let raw = reader
                .seek_raw(std::time::Duration::from_millis(index * 20))
                .unwrap()
                .unwrap();
            assert_eq!(
                raw.into_rgb(32 << 20).unwrap(),
                vec![130; side as usize * side as usize * 3]
            );
            assert_eq!(
                reader.frame_interval().unwrap(),
                (
                    index as u128 * 20_000_000,
                    index as u128 * 20_000_000 + 20_000_000,
                    1_000_000_000
                )
            );
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
    }
}
#[test]
fn owned_audio_file_geometry_and_interval_follow_enabled_stream() {
    let source = root().join("audiovisual.mkv");
    let expected = fs::read(root().join("expected-audio.f32le")).unwrap();
    for (index, interval) in [None, Some((20_000, 40_000))].into_iter().enumerate() {
        let output = std::env::temp_dir().join(format!(
            "fvid-default-audio-{}-{index}.wav",
            std::process::id()
        ));
        let stats = fvid_media::decode_audio_transformed(
            &source,
            &output,
            fvid_media::AudioDecodeTransform {
                interval,
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        assert_eq!((stats.sample_rate, stats.channels), (8000, 1));
        assert_eq!(
            stats.sample_frames,
            if interval.is_some() { 160 } else { 480 }
        );
        let bytes = fs::read(&output).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        let mut at = 12;
        let mut bits = 0;
        let mut decoded = Vec::new();
        while at + 8 <= bytes.len() {
            let length = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
            let payload = &bytes[at + 8..at + 8 + length];
            match &bytes[at..at + 4] {
                b"fmt " => {
                    assert_eq!(u32::from_le_bytes(payload[4..8].try_into().unwrap()), 8000);
                    bits = u16::from_le_bytes(payload[14..16].try_into().unwrap());
                }
                b"data" => {
                    assert_eq!(bits, 64);
                    for sample in payload.chunks_exact(8) {
                        decoded.extend(
                            (f64::from_le_bytes(sample.try_into().unwrap()) as f32).to_le_bytes(),
                        );
                    }
                }
                _ => {}
            }
            at += 8 + length + (length % 2);
        }
        assert_eq!(
            decoded,
            if interval.is_some() {
                expected[160 * 4..320 * 4].to_vec()
            } else {
                expected.clone()
            }
        );
        fs::remove_file(output).unwrap();
    }
}
#[test]
fn preference_metadata_is_defaulted_and_explicit_access_remains_available() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let mut reader = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            Default::default(),
        )
        .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks.len(), 2);
        assert_eq!(reader.packets.len(), 6);
        assert!(reader.track_enabled(1));
        assert!(reader.track_enabled(2));
        let flags: Vec<_> = case["defaults"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_bool().unwrap())
            .collect();
        for (i, &value) in flags.iter().enumerate() {
            assert_eq!(reader.track_dispositions[&(i as u64 + 1)] & 1 != 0, value);
        }
        let expected: Vec<_> = (0..2)
            .filter(|&i| !flags.iter().any(|&v| v) || flags[i])
            .map(|i| i as u64 + 1)
            .collect();
        assert_eq!(reader.preferred_track_numbers(&[999, 1, 1, 2]), expected);
        assert!(reader.preferred_track_numbers(&[999]).is_empty());
        for index in 0..6 {
            assert!(!reader.read_packet(index).unwrap().is_empty());
        }
    }
    let mut output = Vec::new();
    let stats = fvid_media::owned_matroska_pcm::decode_matroska_pcm(
        Cursor::new(fs::read(root().join("audiovisual.mkv")).unwrap()),
        &mut output,
        None,
        &fvid_control::CopyOptions {
            streams: vec![1],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 480);
    assert_eq!(output, vec![0; 480 * 4]);
}
#[test]
fn malformed_default_flags_refuse_the_specific_attribute() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["invalid"].as_array().unwrap() {
        let error = match fvid_media::owned_webm::WebmReader::open(
            Cursor::new(fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            Default::default(),
        ) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("invalid default admitted"),
        };
        assert!(error.contains(case["error"].as_str().unwrap()), "{error}");
    }
}
#[cfg(feature = "player")]
#[test]
fn player_audio_preference_preserves_manual_audio_and_subtitle_lists() {
    use fvid::audio::AudioStream;
    let source = fs::read(root().join("audiovisual.mkv")).unwrap();
    let mut audio = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(source.clone()),
        Default::default(),
    )
    .unwrap();
    assert_eq!(audio.track().number, 3);
    assert_eq!(audio.audio_tracks().len(), 2);
    let explicit = fvid::playback_webm_audio::WebmAudioReader::open_at(
        Cursor::new(source.clone()),
        Default::default(),
        0,
    )
    .unwrap();
    assert_eq!(explicit.track().number, 2);
    let mut decoder = audio.make_decoder().unwrap();
    let mut pcm = Vec::new();
    for index in 0..3 {
        let packet = audio.read_packet().unwrap().unwrap();
        assert_eq!(packet.pts_ns, index * 20_000_000);
        pcm.extend(
            decoder
                .decode_encoded(&packet.data, index as u64 * 160, 160)
                .unwrap()
                .unwrap()
                .data,
        );
    }
    assert!(audio.read_packet().unwrap().is_none());
    assert_eq!(pcm, fs::read(root().join("expected-audio.f32le")).unwrap());
    let mut subtitles = fvid::playback_webm_subtitles::WebmSubtitleReader::open(
        Cursor::new(source),
        Default::default(),
    )
    .unwrap();
    assert_eq!(subtitles.tracks().len(), 2);
    assert_eq!(subtitles.cues(0).unwrap()[0].text, "disabled text");
    assert_eq!(subtitles.cues(1).unwrap()[0].text, "enabled text0");
}
#[test]
fn movie_preference_is_preserved_as_matroska_default_priority() {
    let source = fs::read(root().join("movie-priority.mp4")).unwrap();
    let mut input =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&source), Default::default()).unwrap();
    let mut output = Cursor::new(Vec::new());
    fvid_media::owned_mp4_matroska::write(&mut input, &mut output, None, None).unwrap();
    let bytes = output.into_inner();
    let reader =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(&bytes), Default::default()).unwrap();
    assert!(reader.track_enabled(1));
    assert!(reader.track_enabled(2));
    assert_eq!(reader.track_dispositions[&1] & 1, 0);
    assert_eq!(reader.track_dispositions[&2] & 1, 1);
    fn pixels(data: Vec<u8>) -> Vec<Vec<u8>> {
        let mut reader =
            fvid::playback_native::NativeReader::software(Cursor::new(data), 32 << 20).unwrap();
        let mut result = Vec::new();
        while let Some(frame) = reader.read_frame_raw().unwrap() {
            result.push(frame.into_rgb(32 << 20).unwrap());
        }
        result
    }
    assert_eq!(pixels(bytes), pixels(source));
}
