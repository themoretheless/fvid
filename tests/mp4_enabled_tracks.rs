use std::{
    fs,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-enabled-tracks")
}
#[test]
fn enabled_movie_track_is_preferred_for_native_playback() {
    let mut reader = fvid::playback_native::NativeReader::software(
        std::io::Cursor::new(fs::read(root().join("disabled-first.mp4")).unwrap()),
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
}
#[test]
fn enabled_movie_track_is_preferred_for_owned_decode() {
    let stats = fvid_media::decode_video(&root().join("disabled-first.mp4")).unwrap();
    assert_eq!((stats.width, stats.height, stats.video_frames), (64, 64, 3));
}
#[test]
fn legacy_zero_and_both_enabled_controls_keep_container_order() {
    for name in ["legacy-zero.mp4", "both-enabled.mp4"] {
        let stats = fvid_media::decode_video(&root().join(name)).unwrap();
        assert_eq!((stats.width, stats.height, stats.video_frames), (16, 16, 3));
    }
}
#[test]
fn owned_audio_automatic_selection_prefers_enabled_movie_stream() {
    let mut output = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        std::io::Cursor::new(fs::read(root().join("audiovisual.mp4")).unwrap()),
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
fn all_flag_versions_and_layouts_keep_exact_playback_seek_and_eof() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let side = case["side"].as_u64().unwrap() as usize;
        let source = root().join(case["file"].as_str().unwrap());
        let stats = fvid_media::decode_video_transformed(
            &source,
            fvid_media::DecodeTransform {
                horizontal_flip: true,
                ..Default::default()
            },
        )
        .unwrap();
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
                let rgb = reader
                    .read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap();
                assert_eq!(rgb, vec![130; side * side * 3], "{}", case["file"]);
                assert_eq!(reader.frame_interval().unwrap(), (index, index + 1, 50));
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        for index in [2, 0, 1, 2] {
            assert_eq!(
                reader
                    .seek_raw(std::time::Duration::from_millis(index * 20))
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                vec![130; side * side * 3]
            );
            assert_eq!(
                reader.frame_interval().unwrap(),
                (index as u128, index as u128 + 1, 50)
            );
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
    }
}
#[test]
fn root_and_owned_readers_keep_all_flags_tracks_and_packet_access() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let data = fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let mut root_reader =
            fvid::container::mp4::Mp4Reader::open(std::io::Cursor::new(&data), Default::default())
                .unwrap();
        let mut owned =
            fvid_media::owned_mp4::Mp4Reader::open(std::io::Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(root_reader.tracks().len(), 2);
        assert_eq!(owned.tracks().len(), 2);
        assert_eq!(root_reader.track_header_flags(999), None);
        assert_eq!(owned.track_header_flags(999), None);
        for index in 0..2 {
            let flags = case["flags"][index].as_u64().unwrap() as u32;
            assert_eq!(root_reader.track_header_flags(index), Some(flags));
            assert_eq!(owned.track_header_flags(index), Some(flags));
            for sample in 0..3 {
                let mut a = Vec::new();
                let mut b = Vec::new();
                root_reader.read_packet(index, sample, &mut a).unwrap();
                owned.read_packet(index, sample, &mut b).unwrap();
                assert_eq!(a, b);
                assert!(!a.is_empty());
            }
        }
        let expected = if case["side"] == 64 {
            vec![1]
        } else {
            vec![0, 1]
        };
        assert_eq!(root_reader.automatic_track_indices(&[*b"vide"]), expected);
        assert_eq!(owned.automatic_track_indices(&[*b"vide"]), expected);
    }
}
#[test]
fn explicit_disabled_audio_inspection_still_works() {
    let mut output = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        std::io::Cursor::new(fs::read(root().join("audiovisual.mp4")).unwrap()),
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
fn player_default_audio_and_explicit_subtitle_access_keep_track_identity() {
    use fvid::audio::AudioStream;
    let source = fs::read(root().join("audiovisual.mp4")).unwrap();
    let mut audio = fvid::playback_mp4_audio::Mp4AudioReader::open(
        std::io::Cursor::new(source.clone()),
        Default::default(),
    )
    .unwrap();
    assert_eq!(audio.track().id, 3);
    assert_eq!(audio.audio_tracks().len(), 2);
    let explicit = fvid::playback_mp4_audio::Mp4AudioReader::open_at(
        std::io::Cursor::new(source.clone()),
        Default::default(),
        0,
    )
    .unwrap();
    assert_eq!(explicit.track().id, 2);
    let mut decoder = audio.make_decoder().unwrap();
    let mut pcm = Vec::new();
    for index in 0..3 {
        let packet = audio.read_packet().unwrap().unwrap();
        assert_eq!((packet.pts, packet.duration), (index * 160, 160));
        pcm.extend(
            decoder
                .decode_encoded(&packet.data, packet.pts as u64, packet.duration as u64)
                .unwrap()
                .unwrap()
                .data,
        );
    }
    assert!(audio.read_packet().unwrap().is_none());
    assert_eq!(pcm, fs::read(root().join("expected-audio.f32le")).unwrap());
    audio.rewind();
    assert_eq!(audio.read_packet().unwrap().unwrap().pts, 0);
    audio.seek(320);
    assert_eq!(audio.read_packet().unwrap().unwrap().pts, 320);
    let mut subtitles = fvid::playback_mp4_subtitles::Mp4SubtitleReader::open(
        std::io::Cursor::new(source),
        Default::default(),
    )
    .unwrap();
    let tracks = subtitles.tracks();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[1].name, "enabled");
    let cues = subtitles.cues(1).unwrap();
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
fn owned_audio_file_geometry_and_interval_follow_enabled_stream() {
    let source = root().join("audiovisual.mp4");
    let expected = fs::read(root().join("expected-audio.f32le")).unwrap();
    for (index, interval) in [None, Some((20_000, 40_000))].into_iter().enumerate() {
        let output = std::env::temp_dir().join(format!(
            "fvid-mp4-enabled-audio-{}-{index}.wav",
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
fn all_stored_tracks_still_consume_sample_and_byte_budgets() {
    let data = fs::read(root().join("disabled-first.mp4")).unwrap();
    assert!(
        fvid::container::mp4::Mp4Reader::open(
            std::io::Cursor::new(&data),
            fvid::container::mp4::Limits {
                samples: 5,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(
        fvid_media::owned_mp4::Mp4Reader::open(
            std::io::Cursor::new(&data),
            fvid_media::owned_mp4::Limits {
                samples: 5,
                ..Default::default()
            }
        )
        .is_err()
    );
    let mut reader = fvid_media::owned_mp4::Mp4Reader::open(
        std::io::Cursor::new(&data),
        fvid_media::owned_mp4::Limits {
            packet_bytes: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(reader.read_packet(1, 0, &mut Vec::new()).is_err());
    let mut output = Vec::new();
    let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        std::io::Cursor::new(fs::read(root().join("audiovisual.mp4")).unwrap()),
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
fn explicit_video_index_can_still_inspect_the_disabled_track() {
    let reader = fvid::container::mp4::Mp4Reader::open(
        std::io::Cursor::new(fs::read(root().join("disabled-first.mp4")).unwrap()),
        Default::default(),
    )
    .unwrap();
    let mut video = fvid::playback_mp4::Mp4VideoReader::from_demuxer(reader, 0, 32 << 20).unwrap();
    assert_eq!((video.track().width, video.track().height), (16, 16));
    assert!(video.read_frame().unwrap().is_some());
}
