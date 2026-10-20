use std::{
    fs,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-enabled-tracks")
}
#[test]
fn disabled_video_is_excluded_from_automatic_playback_selection() {
    let mut reader = fvid::playback_native::NativeReader::software(
        std::io::Cursor::new(fs::read(root().join("disabled.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    let raw = reader.read_frame_raw().unwrap().unwrap();
    let rgb = raw.into_rgb(32 << 20).unwrap();
    assert_eq!(rgb.len(), 64 * 64 * 3);
}
#[test]
fn disabled_video_is_excluded_from_owned_automatic_decode() {
    let stats = fvid_media::decode_video(&root().join("disabled.mkv")).unwrap();
    assert_eq!((stats.width, stats.height), (64, 64));
    assert_eq!(stats.video_frames, 3);
}
#[test]
fn omitted_empty_and_enabled_flag_controls_stay_usable() {
    for name in ["omitted.mkv", "empty.mkv", "enabled.mkv"] {
        let stats = fvid_media::decode_video(&root().join(name)).unwrap();
        assert_eq!((stats.width, stats.height, stats.video_frames), (16, 16, 3));
    }
}
#[test]
fn only_enabled_audio_is_automatically_selected_for_pcm() {
    let mut output = Vec::new();
    let stats = fvid_media::owned_matroska_pcm::decode_matroska_pcm(
        std::io::Cursor::new(fs::read(root().join("audiovisual.mkv")).unwrap()),
        &mut output,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        (stats.sample_rate, stats.channels, stats.sample_frames),
        (8000, 1, 480)
    );
    assert_eq!(
        output,
        fs::read(root().join("expected-audio.f32le")).unwrap()
    );
}
#[test]
fn usable_video_variants_preserve_pixels_clocks_seek_and_eof() {
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
fn disabled_tracks_remain_indexed_and_byte_exact_copies_preserve_flags() {
    use fvid_media::owned_webm::{Limits, WebmReader};
    for name in ["disabled.mkv", "late-disabled.mkv", "audiovisual.mkv"] {
        let input = fs::read(root().join(name)).unwrap();
        let mut reader = WebmReader::open(std::io::Cursor::new(&input), Limits::default()).unwrap();
        reader.scan_all().unwrap();
        let count = if name == "audiovisual.mkv" { 5 } else { 2 };
        assert_eq!(reader.tracks.len(), count);
        assert_eq!(reader.packets.len(), count * 3);
        assert!(!reader.track_enabled(999));
        for index in 0..reader.packets.len() {
            assert!(!reader.read_packet(index).unwrap().is_empty());
        }
        assert!(!reader.metadata_complete);
        let mut output = Vec::new();
        fvid_media::owned_matroska_copy::copy(
            &mut std::io::Cursor::new(&input),
            &mut output,
            false,
            None,
            None,
        )
        .unwrap();
        assert_eq!(output, input);
    }
}
#[test]
fn explicit_audio_inspection_can_still_select_disabled_stored_stream() {
    let mut output = Vec::new();
    let stats = fvid_media::owned_matroska_pcm::decode_matroska_pcm(
        std::io::Cursor::new(fs::read(root().join("audiovisual.mkv")).unwrap()),
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
#[cfg(feature = "player")]
#[test]
fn player_audio_and_subtitle_selection_skips_disabled_tracks() {
    use fvid::audio::AudioStream;
    let source = fs::read(root().join("audiovisual.mkv")).unwrap();
    let mut audio = fvid::playback_webm_audio::WebmAudioReader::open(
        std::io::Cursor::new(source.clone()),
        Default::default(),
    )
    .unwrap();
    assert_eq!(audio.track().number, 3);
    let mut decoder = audio.make_decoder().unwrap();
    let mut pcm = Vec::new();
    for index in 0..3 {
        let packet = audio.read_packet().unwrap().unwrap();
        assert_eq!(packet.pts_ns, index * 20_000_000);
        assert_eq!(packet.track, 3);
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
    audio.rewind();
    assert_eq!(audio.read_packet().unwrap().unwrap().track, 3);
    audio.seek(40_000_000);
    assert_eq!(audio.read_packet().unwrap().unwrap().pts_ns, 40_000_000);
    let mut subtitles = fvid::playback_webm_subtitles::WebmSubtitleReader::open(
        std::io::Cursor::new(source),
        Default::default(),
    )
    .unwrap();
    let tracks = subtitles.tracks();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].name, "enabled");
    let cues = subtitles.cues(0).unwrap();
    assert_eq!(cues.len(), 3);
    for (index, cue) in cues.iter().enumerate() {
        assert_eq!(cue.text, format!("enabled text{index}"));
        assert_eq!(
            cue.start,
            std::time::Duration::from_millis(index as u64 * 20)
        );
        assert_eq!(
            cue.end,
            std::time::Duration::from_millis(index as u64 * 20 + 20)
        );
    }
}
#[test]
fn invalid_usability_flags_are_specific_parse_refusals() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["invalid"].as_array().unwrap() {
        let error = match fvid_media::owned_webm::WebmReader::open(
            std::io::Cursor::new(fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            Default::default(),
        ) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("invalid flag accepted"),
        };
        assert!(error.contains(case["error"].as_str().unwrap()), "{error}");
    }
}
#[test]
fn disabled_packets_still_consume_index_and_stored_byte_budgets() {
    use fvid_media::owned_webm::{Limits, WebmReader};
    let data = fs::read(root().join("disabled.mkv")).unwrap();
    for limits in [
        Limits {
            packet_bytes: 1,
            ..Default::default()
        },
        Limits {
            packets: 1,
            ..Default::default()
        },
        Limits {
            elements: 1,
            ..Default::default()
        },
    ] {
        assert!(WebmReader::open(std::io::Cursor::new(&data), limits).is_err());
    }
    let mut output = Vec::new();
    let error = fvid_media::owned_matroska_pcm::decode_matroska_pcm(
        std::io::Cursor::new(fs::read(root().join("audiovisual.mkv")).unwrap()),
        &mut output,
        None,
        &fvid_control::CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("controlled memory budget"));
    assert!(output.is_empty());
}
#[test]
fn owned_audio_file_geometry_and_interval_follow_enabled_stream() {
    let source = root().join("audiovisual.mkv");
    let expected = fs::read(root().join("expected-audio.f32le")).unwrap();
    for (index, interval) in [None, Some((20_000, 40_000))].into_iter().enumerate() {
        let output = std::env::temp_dir().join(format!(
            "fvid-enabled-audio-{}-{index}.wav",
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
fn unsupported_remux_does_not_silently_enable_a_disabled_track() {
    let output =
        std::env::temp_dir().join(format!("fvid-disabled-remux-{}.mkv", std::process::id()));
    let error = fvid_media::transcode_lossless(
        &root().join("disabled.mkv"),
        &output,
        fvid_media::LosslessTransform {
            horizontal_flip: true,
            ..Default::default()
        },
        &Default::default(),
    );
    assert!(error.is_err());
    assert!(!output.exists());
}
