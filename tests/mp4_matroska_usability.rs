use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-matroska-usability")
}
fn remux(data: &[u8], order: &[usize]) -> Vec<u8> {
    let mut reader =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(data), Default::default()).unwrap();
    let mut output = Cursor::new(Vec::new());
    fvid_media::owned_mp4_matroska::write_selected(
        &mut reader,
        &mut output,
        None,
        None,
        None,
        order,
    )
    .unwrap();
    output.into_inner()
}
fn frames(data: Vec<u8>) -> Vec<(Vec<u8>, u128, u128)> {
    let mut reader =
        fvid::playback_native::NativeReader::software(Cursor::new(data), 32 << 20).unwrap();
    let mut result = Vec::new();
    while let Some(raw) = reader.read_frame_raw().unwrap() {
        let (a, b, s) = reader.frame_interval().unwrap();
        result.push((
            raw.into_rgb(32 << 20).unwrap(),
            a * 1_000_000_000 / u128::from(s),
            b * 1_000_000_000 / u128::from(s),
        ));
    }
    result
}
#[test]
fn copied_disabled_track_keeps_its_effective_usability() {
    let source = fs::read(root().join("disabled-first.mp4")).unwrap();
    let output = remux(&source, &[]);
    let reader =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(output), Default::default()).unwrap();
    assert!(!reader.track_enabled(1));
    assert!(reader.track_enabled(2));
}
#[test]
fn remux_preserves_selected_video_pixels_and_presentation_intervals() {
    let source = fs::read(root().join("disabled-first.mp4")).unwrap();
    assert_eq!(frames(remux(&source, &[])), frames(source));
}
#[test]
fn equal_priority_and_legacy_fallback_controls_remain_playable() {
    for name in ["legacy-zero.mp4", "both-enabled.mp4"] {
        let source = fs::read(root().join(name)).unwrap();
        assert_eq!(frames(remux(&source, &[])), frames(source));
    }
}
fn remux_root(data: &[u8], order: &[usize]) -> Vec<u8> {
    let mut reader =
        fvid::container::mp4::Mp4Reader::open(Cursor::new(data), Default::default()).unwrap();
    let mut output = Cursor::new(Vec::new());
    fvid::container::mp4_matroska::write_selected(
        &mut reader,
        &mut output,
        None,
        None,
        None,
        order,
    )
    .unwrap();
    output.into_inner()
}
#[test]
fn both_remux_paths_map_source_flags_through_output_reordering() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let data = fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        for order in [vec![0, 1], vec![1, 0]] {
            let output = remux(&data, &order);
            assert_eq!(output, remux_root(&data, &order));
            let reader = fvid_media::owned_webm::WebmReader::open(
                Cursor::new(output.clone()),
                Default::default(),
            )
            .unwrap();
            for (index, &source) in order.iter().enumerate() {
                assert_eq!(
                    reader.track_enabled(index as u64 + 1),
                    case["effective_enabled"][source].as_bool().unwrap()
                );
            }
            if case["file"] == "disabled-first.mp4" {
                assert_eq!(frames(output), frames(data.clone()));
            } else {
                assert_eq!(frames(output).len(), 3);
            }
        }
    }
}
#[test]
fn every_copied_packet_keeps_its_source_bytes_and_clock() {
    let data = fs::read(root().join("disabled-first.mp4")).unwrap();
    let source =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
    for order in [vec![0, 1], vec![1, 0], vec![0], vec![1]] {
        let output = remux(&data, &order);
        let mut reader =
            fvid_media::owned_webm::WebmReader::open(Cursor::new(&output), Default::default())
                .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks.len(), order.len());
        assert_eq!(reader.packets.len(), order.len() * 3);
        let mut original =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
        for (index, &selected) in order.iter().enumerate() {
            let packets: Vec<_> = reader
                .packets
                .iter()
                .enumerate()
                .filter(|(_, p)| p.track == index as u64 + 1)
                .map(|(i, p)| (i, p.clone()))
                .collect();
            assert_eq!(reader.track_enabled(index as u64 + 1), selected == 1);
            for (sample, (stored, p)) in packets.iter().enumerate() {
                let metadata = source.tracks()[selected].samples.get(sample).unwrap();
                assert_eq!(
                    p.pts_ns,
                    metadata.pts * 1_000_000_000 / i64::from(source.tracks()[selected].timescale)
                );
                assert_eq!(
                    p.duration_ns,
                    Some(
                        u64::from(metadata.duration) * 1_000_000_000
                            / u64::from(source.tracks()[selected].timescale)
                    )
                );
                let mut bytes = Vec::new();
                original.read_packet(selected, sample, &mut bytes).unwrap();
                assert_eq!(reader.read_packet(*stored).unwrap(), bytes);
            }
        }
        if order == [0] {
            assert!(
                fvid::playback_native::NativeReader::software(Cursor::new(output), 32 << 20)
                    .is_err()
            );
        } else {
            assert_eq!(frames(output), frames(data.clone()));
        }
    }
}
#[test]
fn file_remux_preserves_usability_atomically() {
    let source = root().join("disabled-first.mp4");
    let output =
        std::env::temp_dir().join(format!("fvid-copy-usability-{}.mkv", std::process::id()));
    fvid_media::owned_mp4_remux::remux(&source, &output, &Default::default()).unwrap();
    assert_eq!(
        frames(fs::read(&output).unwrap()),
        frames(fs::read(&source).unwrap())
    );
    fs::remove_file(&output).unwrap();
    let cancel = fvid_control::CancelFlag::new();
    cancel.cancel();
    assert!(
        fvid_media::owned_mp4_remux::remux(
            &source,
            &output,
            &fvid_control::CopyOptions {
                cancel: Some(cancel),
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(!output.exists());
    assert!(
        fvid_media::owned_mp4_remux::remux(
            &source,
            &output,
            &fvid_control::CopyOptions {
                max_packet_bytes: 1,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(!output.exists());
}
#[test]
fn filtered_video_with_copied_aac_retains_disabled_audio_and_exact_packets() {
    let source = root().join("audiovisual.mp4");
    let mut baseline = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(fs::read(&source).unwrap()),
        &mut baseline,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        (stats.sample_rate, stats.channels, stats.sample_frames),
        (48000, 1, 3072)
    );
    assert_eq!(baseline.len(), 3072 * 4);
    assert!(
        baseline
            .chunks_exact(4)
            .all(|b| f32::from_le_bytes(b.try_into().unwrap()) == 0.0)
    );
    let output = std::env::temp_dir().join(format!(
        "fvid-usability-filtered-{}.mkv",
        std::process::id()
    ));
    fvid_media::transcode_lossless(
        &source,
        &output,
        fvid_media::LosslessTransform {
            horizontal_flip: true,
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap();
    let data = fs::read(&output).unwrap();
    let mut reader =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(&data), Default::default()).unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks.len(), 3);
    assert!(reader.track_enabled(1));
    assert!(!reader.track_enabled(2));
    assert!(reader.track_enabled(3));
    assert_eq!(reader.tracks[0].codec, "V_FFV1");
    let mut input = fvid_media::owned_mp4::Mp4Reader::open(
        Cursor::new(fs::read(&source).unwrap()),
        Default::default(),
    )
    .unwrap();
    for track in [1, 2] {
        let packets: Vec<_> = reader
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == track as u64 + 1)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(packets.len(), 3);
        for (sample, &stored) in packets.iter().enumerate() {
            let mut packet = Vec::new();
            input.read_packet(track, sample, &mut packet).unwrap();
            assert_eq!(reader.read_packet(stored).unwrap(), packet);
        }
    }
    let mut audio = Vec::new();
    let stats = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
        Cursor::new(&data),
        &mut audio,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 3072);
    assert_eq!(audio.len(), baseline.len());
    assert!(
        audio
            .chunks_exact(4)
            .all(|b| f32::from_le_bytes(b.try_into().unwrap()) == 0.0)
    );
    let mut expected = frames(fs::read(&source).unwrap());
    for (rgb, _, _) in &mut expected {
        for row in rgb.chunks_exact_mut(16 * 3) {
            let pixels: Vec<_> = row
                .chunks_exact(3)
                .map(|p| [p[0], p[1], p[2]])
                .rev()
                .collect();
            for (dst, pixel) in row.chunks_exact_mut(3).zip(pixels) {
                dst.copy_from_slice(&pixel);
            }
        }
    }
    let actual = frames(data);
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!((actual.1, actual.2), (expected.1, expected.2));
        assert!(
            actual
                .0
                .iter()
                .zip(expected.0)
                .all(|(&a, b)| a.abs_diff(b) <= 1)
        );
    }
    fs::remove_file(output).unwrap();
}
#[test]
fn remuxed_selected_avc_keeps_precision_navigation_and_eof() {
    let source = fs::read(root().join("disabled-first.mp4")).unwrap();
    let expected = frames(source.clone());
    for order in [vec![0, 1], vec![1, 0]] {
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(remux(&source, &order)),
            32 << 20,
        )
        .unwrap();
        assert_eq!(
            reader.duration().unwrap(),
            std::time::Duration::from_millis(60)
        );
        for _ in 0..2 {
            for reference in &expected {
                let raw = reader.read_frame_raw().unwrap().unwrap();
                let fvid::playback_native::RawFrame::Avc { picture, .. } = &raw else {
                    panic!("precision narrowed")
                };
                assert_eq!(picture.component_depths(), [14, 8, 8]);
                let (a, b, s) = reader.frame_interval().unwrap();
                assert_eq!(
                    (
                        a * 1_000_000_000 / u128::from(s),
                        b * 1_000_000_000 / u128::from(s)
                    ),
                    (reference.1, reference.2)
                );
                assert_eq!(raw.into_rgb(32 << 20).unwrap(), reference.0);
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        for index in [2, 0, 1, 2] {
            let raw = reader
                .seek_raw(std::time::Duration::from_millis(index as u64 * 20))
                .unwrap()
                .unwrap();
            assert_eq!(raw.into_rgb(32 << 20).unwrap(), expected[index].0);
            let (a, b, s) = reader.frame_interval().unwrap();
            assert_eq!(
                (
                    a * 1_000_000_000 / u128::from(s),
                    b * 1_000_000_000 / u128::from(s)
                ),
                (expected[index].1, expected[index].2)
            );
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
    }
}
