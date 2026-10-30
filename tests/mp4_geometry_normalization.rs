use std::{io::Cursor, path::PathBuf, time::Duration};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-geometry-normalization")
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn f32s(bytes: &[u8]) -> Vec<f64> {
    bytes
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()) as f64)
        .collect()
}
fn compare(actual: &[f64], expected: &[f64], tolerance: f64, name: &str) {
    assert_eq!(actual.len(), expected.len(), "{name}: length");
    let mut peak = 0f64;
    for (i, (&a, &b)) in actual.iter().zip(expected).enumerate() {
        peak = peak.max(b.abs());
        assert!(
            a.is_finite() && (a - b).abs() <= tolerance,
            "{name}/{i}: {a} != {b}"
        );
    }
    assert!(peak > 1e-4, "nonzero oracle required");
}
fn interval_oracle(pcm: &[u8], interval: Option<(Duration, Duration)>, rate: u64) -> Vec<f64> {
    let range = if let Some((a, b)) = interval {
        let at = |d: Duration| {
            ((d.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as usize * 8).min(pcm.len())
        };
        at(a)..at(b)
    } else {
        0..pcm.len()
    };
    f32s(&pcm[range])
}
#[test]
fn pcm_native_rate_and_layout_changes_follow_one_exact_output_grid() {
    for case in cases().into_iter().filter(|c| c["precise"] == true) {
        let name = case["name"].as_str().unwrap();
        let input = bytes(&format!("{name}.mp4"));
        let oracle = bytes(case["oracle"].as_str().unwrap());
        for interval in [
            None,
            Some((Duration::from_micros(22001), Duration::from_micros(75007))),
            Some((Duration::from_micros(43333), Duration::from_millis(999))),
        ] {
            let mut output = Vec::new();
            let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&input),
                &mut output,
                interval,
                &Default::default(),
            )
            .unwrap();
            let expected = interval_oracle(&oracle, interval, case["rate"].as_u64().unwrap());
            compare(&f32s(&output), &expected, 1e-6, name);
            assert_eq!(
                (stats.sample_rate, stats.channels, stats.sample_frames),
                (
                    case["rate"].as_u64().unwrap() as u32,
                    2,
                    expected.len() as u64 / 2
                )
            );
            output.clear();
            let stats64 = fvid_media::owned_mp4_audio::decode_mp4_pcm_f64(
                Cursor::new(&input),
                &mut output,
                interval,
                &Default::default(),
            )
            .unwrap();
            let actual = output
                .chunks_exact(8)
                .map(|v| f64::from_le_bytes(v.try_into().unwrap()))
                .collect::<Vec<_>>();
            let precise_expected = if let Some(oracle64) = case["oracle64"].as_str() {
                let data = bytes(oracle64);
                let values = data
                    .chunks_exact(8)
                    .map(|v| f64::from_le_bytes(v.try_into().unwrap()))
                    .collect::<Vec<_>>();
                if let Some((a, b)) = interval {
                    let first = ((a.as_nanos() * 48000).div_ceil(1_000_000_000) as usize * 2)
                        .min(values.len());
                    let last = ((b.as_nanos() * 48000).div_ceil(1_000_000_000) as usize * 2)
                        .min(values.len());
                    values[first..last].to_vec()
                } else {
                    values
                }
            } else {
                expected.clone()
            };
            compare(&actual, &precise_expected, 1e-12, name);
            assert_eq!(stats, stats64);
        }
    }
}
#[test]
fn aac_profile_rate_and_layout_changes_match_independent_native_and_filter_oracles() {
    for case in cases().into_iter().filter(|c| c["precise"] == false) {
        let name = case["name"].as_str().unwrap();
        let input = bytes(&format!("{name}.mp4"));
        let oracle = bytes(case["oracle"].as_str().unwrap());
        for interval in [
            None,
            Some((Duration::from_micros(22001), Duration::from_micros(75007))),
            Some((Duration::from_micros(93333), Duration::from_millis(247))),
        ] {
            let mut output = Vec::new();
            let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&input),
                &mut output,
                interval,
                &Default::default(),
            )
            .unwrap_or_else(|error| panic!("{name} interval {interval:?}: {error}"));
            let expected = interval_oracle(&oracle, interval, case["rate"].as_u64().unwrap());
            compare(&f32s(&output), &expected, 2e-6, name);
            assert_eq!(
                (stats.sample_rate, stats.channels, stats.sample_frames),
                (
                    case["rate"].as_u64().unwrap() as u32,
                    2,
                    expected.len() as u64 / 2
                )
            );
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn root_aac_export_keeps_the_same_normalized_waveform() {
    for name in [
        "aac-third",
        "aac-third-edits",
        "ssr-third",
        "ssr-third-edits",
    ] {
        let mut output = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes(&format!("{name}.mp4")), &mut output)
            .unwrap();
        compare(
            &f32s(&output),
            &f32s(&bytes(
                match (name.starts_with("ssr-"), name.ends_with("-edits")) {
                    (false, false) => "aac.f32le",
                    (false, true) => "aac-edits.f32le",
                    (true, false) => "ssr.f32le",
                    (true, true) => "ssr-edits.f32le",
                },
            )),
            2e-6,
            name,
        );
    }
}
#[test]
fn file_planning_and_wave_publication_accept_normalized_geometry() {
    for case in cases().into_iter().filter(|c| {
        matches!(
            c["name"].as_str().unwrap(),
            "pcm-start-third-edits"
                | "pcm-fragmented"
                | "pcm64-edits"
                | "aac-third-edits"
                | "aac-fragmented"
                | "ssr-fragmented"
        )
    }) {
        let name = case["name"].as_str().unwrap();
        let path = root().join(format!("{name}.mp4"));
        fvid_media::plan_decode_audio(&path, &Default::default(), &Default::default()).unwrap();
        let dest =
            std::env::temp_dir().join(format!("fvid-geometry-{}-{name}.wav", std::process::id()));
        let stats = fvid_media::decode_audio(&path, &dest, &Default::default()).unwrap();
        let wave = std::fs::read(&dest).unwrap();
        std::fs::remove_file(dest).unwrap();
        let header =
            fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&wave), None).unwrap();
        assert_eq!(
            (header.sample_rate, header.channels),
            (case["rate"].as_u64().unwrap() as u32, 2)
        );
        assert_eq!(stats.sample_frames, case["frames"].as_u64().unwrap());
        let payload = &wave[header.data_offset as usize
            ..(header.data_offset + u64::from(header.data_bytes)) as usize];
        let actual = if header.bits_per_sample == 64 {
            payload
                .chunks_exact(8)
                .map(|v| f64::from_le_bytes(v.try_into().unwrap()))
                .collect()
        } else {
            f32s(payload)
        };
        if let Some(oracle64) = case["oracle64"].as_str() {
            assert_eq!(header.bits_per_sample, 64);
            let expected = bytes(oracle64)
                .chunks_exact(8)
                .map(|v| f64::from_le_bytes(v.try_into().unwrap()))
                .collect::<Vec<_>>();
            compare(&actual, &expected, 1e-12, name);
        } else {
            compare(
                &actual,
                &f32s(&bytes(case["oracle"].as_str().unwrap())),
                2e-6,
                name,
            );
        }
    }
}
#[test]
fn normalization_honours_budget_and_packet_prefix_controls() {
    let input = bytes("pcm.mp4");
    let mut output = Vec::new();
    let tiny = fvid_media::CopyOptions {
        max_controlled_bytes: Some(1),
        ..Default::default()
    };
    let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(&input),
        &mut output,
        None,
        &tiny,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("controlled memory budget exceeded")
    );
    assert!(output.is_empty());
    let admitted = fvid_media::CopyOptions {
        max_controlled_bytes: Some(64 * 1024 * 1024),
        ..Default::default()
    };
    for (name, oracle) in [
        ("pcm64-edits", "pcm64-edits.f32le"),
        ("aac-third", "aac.f32le"),
        ("ssr-third", "ssr.f32le"),
    ] {
        let mut normalized = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(&format!("{name}.mp4"))),
            &mut normalized,
            None,
            &admitted,
        )
        .unwrap();
        compare(&f32s(&normalized), &f32s(&bytes(oracle)), 2e-6, name);
    }
    let prefix = fvid_media::CopyOptions {
        max_packets: Some(2),
        ..Default::default()
    };
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(&input),
        &mut output,
        None,
        &prefix,
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 1959);
    compare(
        &f32s(&output),
        &f32s(&bytes("pcm.f32le"))[..1959 * 2],
        1e-6,
        "packet prefix",
    );
}

#[test]
fn normalized_aac_can_fold_to_mono_without_applying_source_layout_twice() {
    for (name, oracle) in [
        ("aac-third-edits", "aac-edits.f32le"),
        ("ssr-third", "ssr.f32le"),
    ] {
        let path = root().join(format!("{name}.mp4"));
        let transform = fvid_media::AudioDecodeTransform {
            channels: Some(1),
            ..Default::default()
        };
        fvid_media::plan_decode_audio(&path, &transform, &Default::default()).unwrap();
        let destination = std::env::temp_dir().join(format!(
            "fvid-geometry-mono-{}-{name}.wav",
            std::process::id()
        ));
        fvid_media::decode_audio_transformed(&path, &destination, transform, &Default::default())
            .unwrap();
        let wave = std::fs::read(&destination).unwrap();
        std::fs::remove_file(destination).unwrap();
        let header =
            fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&wave), None).unwrap();
        assert_eq!(header.channels, 1);
        let expected: Vec<f64> = f32s(&bytes(oracle))
            .chunks_exact(2)
            .map(|v| (v[0] + v[1]) * 0.5)
            .collect();
        let payload = &wave[header.data_offset as usize
            ..(header.data_offset + u64::from(header.data_bytes)) as usize];
        compare(&f32s(payload), &expected, 2e-6, name);
    }
}
#[cfg(feature = "player")]
#[test]
fn public_media_api_and_cli_publish_the_normalized_waveform() {
    for (name, oracle) in [
        ("pcm-negative", "pcm-negative.f32le"),
        ("aac-third-edits", "aac-edits.f32le"),
        ("ssr-third-edits", "ssr-edits.f32le"),
    ] {
        let path = root().join(format!("{name}.mp4"));
        fvid::media::plan_decode_audio(&path, &Default::default(), &Default::default()).unwrap();
        let dest = std::env::temp_dir().join(format!(
            "fvid-public-geometry-{}-{name}.f32le",
            std::process::id()
        ));
        fvid::media::decode_audio(&path, &dest, &Default::default()).unwrap();
        let pcm = std::fs::read(&dest).unwrap();
        std::fs::remove_file(&dest).unwrap();
        compare(&f32s(&pcm), &f32s(&bytes(oracle)), 2e-6, name);
        let cli = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "decode-audio"])
            .arg(&path)
            .arg(&dest)
            .output()
            .unwrap();
        assert!(
            cli.status.success(),
            "{}",
            String::from_utf8_lossy(&cli.stderr)
        );
        let pcm = std::fs::read(&dest).unwrap();
        std::fs::remove_file(dest).unwrap();
        compare(&f32s(&pcm), &f32s(&bytes(oracle)), 2e-6, name);
    }
}
#[test]
fn opaque_initial_aac_layout_requires_a_destination_instead_of_silent_rematrixing() {
    let mut control = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(bytes("opaque-control.mp4")),
        &mut control,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        (stats.sample_rate, stats.channels, stats.sample_frames),
        (48000, 4, 3072)
    );
    compare(
        &f32s(&control),
        &f32s(&bytes("opaque-control.f32le")),
        1e-6,
        "valid four-front PCE control",
    );
    let mut output = Vec::new();
    let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(bytes("opaque-changing.mp4")),
        &mut output,
        None,
        &Default::default(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("explicit destination speaker layout"),
        "{error}"
    );
    assert!(output.is_empty());
}
#[test]
fn explicit_destinations_can_normalize_an_opaque_initial_aac_layout() {
    let path = root().join("opaque-changing.mp4");
    for (rate, channels, oracle) in [
        (48000u32, 2u16, "opaque-stereo.f32le"),
        (32000, 2, "opaque-stereo-32k.f32le"),
        (48000, 7, "opaque-seven.f32le"),
    ] {
        let transform = fvid_media::AudioDecodeTransform {
            channels: Some(i32::from(channels)),
            sample_rate: Some(rate as i32),
            ..Default::default()
        };
        fvid_media::plan_decode_audio(&path, &transform, &Default::default()).unwrap();
        let dest = std::env::temp_dir().join(format!(
            "fvid-opaque-normalize-{}-{rate}-{channels}.wav",
            std::process::id()
        ));
        fvid_media::decode_audio_transformed(&path, &dest, transform, &Default::default()).unwrap();
        let wave = std::fs::read(&dest).unwrap();
        std::fs::remove_file(dest).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&wave), None).unwrap();
        assert_eq!((info.sample_rate, info.channels), (rate, channels));
        let expected = f32s(&bytes(oracle));
        compare(
            &f32s(
                &wave[info.data_offset as usize
                    ..(info.data_offset + u64::from(info.data_bytes)) as usize],
            ),
            &expected,
            2e-6,
            "opaque explicit destination",
        );
    }
}
