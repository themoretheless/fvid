use std::{io::Cursor, time::Duration};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/mp4-audio-export-transitions/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn compare(actual: &[u8], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len());
    let mut peak = 0f32;
    for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        peak = peak.max(b.abs());
        assert!((a - b).abs() <= 1e-6, "{a} != {b}");
    }
    assert!(peak > 1e-4);
}
#[test]
fn aac_configuration_changes_export_the_independent_waveform_and_interval() {
    for stem in ["changes", "fragmented", "third", "equivalent"] {
        for edited in [false, true] {
            let input = bytes(&format!("{stem}{}.mp4", if edited { "-edits" } else { "" }));
            let expected = bytes(if edited {
                "edited.f32le"
            } else {
                "expected.f32le"
            });
            for interval in [
                None,
                Some((Duration::from_millis(30), Duration::from_millis(150))),
            ] {
                let mut output = Vec::new();
                let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                    Cursor::new(&input),
                    &mut output,
                    interval,
                    &Default::default(),
                )
                .unwrap();
                assert_eq!((stats.sample_rate, stats.channels), (48000, 2));
                let wanted = if interval.is_some() {
                    &expected[1440 * 8..7200 * 8]
                } else {
                    &expected
                };
                compare(&output, wanted);
                assert_eq!(stats.sample_frames as usize * 8, wanted.len());
            }
        }
    }
}
#[test]
fn delayed_ssr_is_drained_before_reconfiguration_and_at_eof() {
    let expected = bytes("ssr-expected.f32le");
    for stem in ["changes", "fragmented", "third"] {
        let input = bytes(&format!("ssr-{stem}.mp4"));
        let mut output = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(input),
            &mut output,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            (stats.sample_rate, stats.channels, stats.sample_frames),
            (24000, 2, 12288)
        );
        compare(&output, &expected);
    }
}
#[test]
fn owned_file_plan_and_wave_publication_accept_configuration_changes() {
    for stem in [
        "changes",
        "fragmented",
        "third",
        "changes-edits",
        "ssr-changes",
    ] {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-audio-export-transitions")
            .join(format!("{stem}.mp4"));
        let plan =
            fvid_media::plan_decode_audio(&path, &Default::default(), &Default::default()).unwrap();
        assert_eq!(plan.streams[0].codec, "aac");
        let destination = std::env::temp_dir().join(format!(
            "fvid-config-export-{}-{stem}.wav",
            std::process::id()
        ));
        fvid_media::decode_audio(&path, &destination, &Default::default()).unwrap();
        let wave = std::fs::read(&destination).unwrap();
        std::fs::remove_file(&destination).unwrap();
        let header =
            fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&wave), None).unwrap();
        let expected = bytes(if stem.starts_with("ssr-") {
            "ssr-expected.f32le"
        } else if stem.ends_with("-edits") {
            "edited.f32le"
        } else {
            "expected.f32le"
        });
        compare(
            &wave[header.data_offset as usize
                ..(header.data_offset + u64::from(header.data_bytes)) as usize],
            &expected,
        );
    }
}
#[test]
fn memory_limit_and_output_geometry_change_refuse_before_pcm() {
    let mut output = Vec::new();
    let options = fvid_media::CopyOptions {
        max_controlled_bytes: Some(1),
        ..Default::default()
    };
    let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(bytes("third.mp4")),
        &mut output,
        None,
        &options,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("controlled memory budget exceeded"),
        "{error}"
    );
    assert!(output.is_empty());
    let input = std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/mp4-aac-transitions/changes.mp4",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(input),
        &mut output,
        None,
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("output geometry normalization"),
        "{error}"
    );
    assert!(output.is_empty());
}
#[test]
fn pcm_depth_and_coding_changes_preserve_float32_and_float64_samples() {
    let expected = bytes("pcm-expected.f32le");
    let expected64: Vec<u8> = expected
        .chunks_exact(4)
        .flat_map(|v| (f32::from_le_bytes(v.try_into().unwrap()) as f64).to_le_bytes())
        .collect();
    for stem in [
        "pcm",
        "pcm-fragmented",
        "pcm-start-third",
        "pcm-start-third-fragmented",
    ] {
        let input = bytes(&format!("{stem}.mp4"));
        for interval in [
            None,
            Some((Duration::from_millis(5), Duration::from_millis(100))),
        ] {
            let mut output = Vec::new();
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&input),
                &mut output,
                interval,
                &Default::default(),
            )
            .unwrap();
            let wanted = if interval.is_some() {
                &expected[240 * 8..4800 * 8]
            } else {
                &expected
            };
            compare(&output, wanted);
            output.clear();
            fvid_media::owned_mp4_audio::decode_mp4_pcm_f64(
                Cursor::new(&input),
                &mut output,
                interval,
                &Default::default(),
            )
            .unwrap();
            let wanted = if interval.is_some() {
                &expected64[240 * 16..4800 * 16]
            } else {
                &expected64
            };
            assert_eq!(&output, wanted);
        }
    }
}
#[test]
fn repeated_edits_restore_the_active_decoder_configuration_and_delayed_windows() {
    for (stem, oracle) in [
        ("later-edits", "later-edited.f32le"),
        ("ssr-edits", "ssr-edited.f32le"),
    ] {
        let input = bytes(&format!("{stem}.mp4"));
        let mut output = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(input),
            &mut output,
            None,
            &Default::default(),
        )
        .unwrap();
        compare(&output, &bytes(oracle));
    }
}
#[cfg(feature = "player")]
#[test]
fn root_aac_export_uses_the_same_configuration_aware_timeline() {
    for (stem, oracle) in [
        ("third", "expected.f32le"),
        ("later-edits", "later-edited.f32le"),
        ("ssr-edits", "ssr-edited.f32le"),
    ] {
        let mut output = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes(&format!("{stem}.mp4")), &mut output)
            .unwrap();
        compare(&output, &bytes(oracle));
    }
}
#[test]
fn changing_aac_profiles_with_identical_speaker_routing_can_fold_to_mono() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-audio-export-transitions/third.mp4");
    let transform = fvid_media::AudioDecodeTransform {
        channels: Some(1),
        ..Default::default()
    };
    fvid_media::plan_decode_audio(&path, &transform, &Default::default()).unwrap();
    let destination =
        std::env::temp_dir().join(format!("fvid-config-mono-{}.wav", std::process::id()));
    fvid_media::decode_audio_transformed(&path, &destination, transform, &Default::default())
        .unwrap();
    let wave = std::fs::read(&destination).unwrap();
    std::fs::remove_file(&destination).unwrap();
    let header = fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&wave), None).unwrap();
    assert_eq!(header.channels, 1);
    let expected: Vec<u8> = bytes("expected.f32le")
        .chunks_exact(8)
        .flat_map(|v| {
            let a = f32::from_le_bytes(v[..4].try_into().unwrap());
            let b = f32::from_le_bytes(v[4..].try_into().unwrap());
            ((a + b) * 0.5).to_le_bytes()
        })
        .collect();
    compare(
        &wave[header.data_offset as usize
            ..(header.data_offset + u64::from(header.data_bytes)) as usize],
        &expected,
    );
}
