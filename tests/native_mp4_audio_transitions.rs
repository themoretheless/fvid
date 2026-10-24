use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-geometry-normalization")
}
fn compare(actual: &[u8], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!(a.is_finite() && (a - b).abs() < 2e-6, "{a} != {b}");
    }
}
#[test]
fn native_file_export_accepts_native_rate_layout_and_edit_transitions() {
    for (name, oracle, rate) in [
        ("native-api-negative", "native-api-negative.f32le", 48000),
        ("pcm-start-third-fragmented", "pcm.f32le", 48000),
        ("aac-third-edits", "aac-edits.f32le", 48000),
        ("ssr-changes-edits", "ssr-edits.f32le", 24000),
    ] {
        let source = root().join(format!("{name}.mp4"));
        let expected = std::fs::read(root().join(oracle)).unwrap();
        // The authored witness must decode through the library first: this isolates
        // the file API's fixed-reader refusal from an unrelated invalid packet.
        let mut control = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(std::fs::read(&source).unwrap()),
            &mut control,
            None,
            &Default::default(),
        )
        .unwrap();
        compare(&control, &expected);
        for extension in ["f32le", "wav", "mka"] {
            let output = std::env::temp_dir().join(format!(
                "fvid-native-transition-{}-{name}.{extension}",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&output);
            let stats = fvid::native_export::export_audio_pcm_selected(
                &source, &output, None, 1., None, None, None, None, None,
            )
            .unwrap();
            assert_eq!(
                (stats.sample_rate, stats.channels, stats.sample_frames),
                (rate, 2, expected.len() as u64 / 8)
            );
            let actual = if extension == "f32le" {
                std::fs::read(&output).unwrap()
            } else if extension == "wav" {
                let mut f = std::fs::File::open(&output).unwrap();
                let info = fvid_media::owned_wave_inspect::inspect(&mut f, None).unwrap();
                let b = std::fs::read(&output).unwrap();
                b[info.data_offset as usize
                    ..(info.data_offset + u64::from(info.data_bytes)) as usize]
                    .to_vec()
            } else {
                let raw = output.with_extension("roundtrip.f32le");
                let _ = std::fs::remove_file(&raw);
                fvid::native_export::export_audio_pcm_selected(
                    &output, &raw, None, 1., None, None, None, None, None,
                )
                .unwrap();
                let data = std::fs::read(&raw).unwrap();
                std::fs::remove_file(raw).unwrap();
                data
            };
            compare(&actual, &expected);
            std::fs::remove_file(&output).unwrap();
        }
    }
}
#[test]
fn native_source_metadata_and_planning_use_the_first_actual_description() {
    for (name, rate) in [
        ("pcm-start-third-fragmented", 48000),
        ("aac-third-edits", 48000),
        ("ssr-changes-edits", 24000),
    ] {
        let source = root().join(format!("{name}.mp4"));
        let info = fvid::native_media::audio_source_info_selected(&source, None).unwrap();
        assert_eq!((info.sample_rate, info.channels), (rate, 2));
        fvid::native_plan::decode_audio(&source, &Default::default()).unwrap();
    }
}
#[test]
fn native_explicit_destinations_keep_pce_positions_and_output_clock() {
    let source = root().join("opaque-changing.mp4");
    for (rate, channels, oracle) in [
        (48000, 2, "opaque-stereo.f32le"),
        (32000, 2, "opaque-stereo-32k.f32le"),
        (48000, 7, "opaque-seven.f32le"),
    ] {
        let transform = fvid_media::AudioDecodeTransform {
            channels: Some(i32::from(channels)),
            sample_rate: Some(rate),
            ..Default::default()
        };
        fvid::native_plan::decode_audio(&source, &transform).unwrap();
        let output = std::env::temp_dir().join(format!(
            "fvid-native-pce-{}-{rate}-{channels}.f32le",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&output);
        let stats = fvid::native_export::export_audio_pcm_selected(
            &source,
            &output,
            None,
            1.,
            Some(channels),
            Some(rate as u32),
            Some(1),
            None,
            None,
        )
        .unwrap();
        let expected = std::fs::read(root().join(oracle)).unwrap();
        compare(&std::fs::read(&output).unwrap(), &expected);
        assert_eq!(
            (stats.sample_rate, stats.channels, stats.sample_frames),
            (
                rate as u32,
                channels,
                expected.len() as u64 / (u64::from(channels) * 4)
            )
        );
        std::fs::remove_file(output).unwrap();
    }
}
#[test]
fn native_interval_gain_and_packet_prefix_keep_the_global_output_grid() {
    let source = root().join("aac-third-edits.mp4");
    let (a, b) = (
        std::time::Duration::from_nanos(22001001),
        std::time::Duration::from_nanos(75007001),
    );
    let expected = std::fs::read(root().join("aac-edits.f32le")).unwrap();
    let first = (a.as_nanos() * 48000).div_ceil(1_000_000_000) as usize * 8;
    let last = (b.as_nanos() * 48000).div_ceil(1_000_000_000) as usize * 8;
    let scaled = expected[first..last]
        .chunks_exact(4)
        .flat_map(|s| (f32::from_le_bytes(s.try_into().unwrap()) * 0.5).to_le_bytes())
        .collect::<Vec<_>>();
    let output =
        std::env::temp_dir().join(format!("fvid-native-interval-{}.f32le", std::process::id()));
    let _ = std::fs::remove_file(&output);
    fvid::native_export::export_audio_pcm_selected(
        &source,
        &output,
        Some((a, b)),
        0.5,
        None,
        None,
        Some(1),
        None,
        None,
    )
    .unwrap();
    compare(&std::fs::read(&output).unwrap(), &scaled);
    std::fs::remove_file(&output).unwrap();
    let options = fvid_media::CopyOptions {
        max_packets: Some(2),
        ..Default::default()
    };
    let stats = fvid::native_export::export_audio_pcm_transformed_with_controls(
        &root().join("pcm-fragmented.mp4"),
        &output,
        None,
        1.,
        None,
        None,
        &options,
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 1959);
    compare(
        &std::fs::read(&output).unwrap(),
        &std::fs::read(root().join("pcm.f32le")).unwrap()[..1959 * 8],
    );
    std::fs::remove_file(output).unwrap();
}
#[test]
fn native_alac_and_adpcm_transitions_match_independent_controls() {
    let directory = root().parent().unwrap().join("mp4-geometry-lossless-adpcm");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let source = directory.join(format!("{name}.mp4"));
        let output = std::env::temp_dir().join(format!(
            "fvid-native-lossless-{}-{name}.f32le",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&output);
        let stats = fvid::native_export::export_audio_pcm_selected(
            &source, &output, None, 1., None, None, None, None, None,
        )
        .unwrap();
        assert_eq!(
            (
                u64::from(stats.sample_rate),
                u64::from(stats.channels),
                stats.sample_frames
            ),
            (
                case["rate"].as_u64().unwrap(),
                case["channels"].as_u64().unwrap(),
                case["frames"].as_u64().unwrap()
            )
        );
        compare(
            &std::fs::read(&output).unwrap(),
            &std::fs::read(directory.join(case["oracle"].as_str().unwrap())).unwrap(),
        );
        std::fs::remove_file(&output).unwrap();
    }
}
#[test]
fn native_aac_entrypoint_keeps_first_used_setup_and_no_overwrite() {
    for (name, rate) in [("aac-third-edits", 48000), ("ssr-changes-edits", 24000)] {
        let source = root().join(format!("{name}.mp4"));
        let info = fvid::native_media::aac_source_info(&source).unwrap();
        assert_eq!((info.sample_rate, info.channels), (rate, 2));
        let output = std::env::temp_dir().join(format!(
            "fvid-native-aac-{}-{name}.f32le",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&output);
        let stats = fvid::native_export::export_aac_pcm_selected(
            &source, &output, None, 1., None, None, None, None, None,
        )
        .unwrap();
        let oracle = if rate == 48000 {
            "aac-edits.f32le"
        } else {
            "ssr-edits.f32le"
        };
        compare(
            &std::fs::read(&output).unwrap(),
            &std::fs::read(root().join(oracle)).unwrap(),
        );
        assert_eq!(stats.sample_rate, rate);
        let before = std::fs::read(&output).unwrap();
        assert!(
            fvid::native_export::export_aac_pcm_selected(
                &source, &output, None, 1., None, None, None, None, None
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&output).unwrap(), before);
        std::fs::remove_file(output).unwrap();
    }
}
#[test]
fn opaque_same_geometry_does_not_require_an_invented_destination_layout() {
    let source = root().join("opaque-same-geometry.mp4");
    let mut control = Vec::new();
    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(std::fs::read(&source).unwrap()),
        &mut control,
        None,
        &Default::default(),
    )
    .unwrap();
    let expected = std::fs::read(root().join("opaque-same-geometry.f32le")).unwrap();
    compare(&control, &expected);
    let output = std::env::temp_dir().join(format!(
        "fvid-native-opaque-preserve-{}.f32le",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&output);
    let stats = fvid::native_export::export_audio_pcm_selected(
        &source, &output, None, 1., None, None, None, None, None,
    )
    .unwrap();
    assert_eq!((stats.sample_rate, stats.channels), (48000, 4));
    compare(&std::fs::read(&output).unwrap(), &expected);
    std::fs::remove_file(output).unwrap();
}
