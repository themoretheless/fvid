use std::path::{Path, PathBuf};
#[test]
fn alac_and_matroska_pcm_trim_use_owned_sample_exact_wave_export() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = std::env::temp_dir().join(format!("fvid-audio-trim-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    for (n, name) in [
        "alac/stereo-24.m4a",
        "alac/mono-16.m4a",
        "audio/pcm-int.mkv",
        "audio/pcm-float.mkv",
    ]
    .iter()
    .enumerate()
    {
        let input = root.join("tests/fixtures").join(name);
        let info = fvid::native_media::audio_source_info_selected(&input, None).unwrap();
        let full = dir.join(format!("{n}.f32le"));
        fvid::native_export::export_audio_pcm_selected(
            &input, &full, None, 1.0, None, None, None, None, None,
        )
        .unwrap();
        let full = std::fs::read(full).unwrap();
        let (from, to) = (1000i64, 5000i64);
        let boundary = |us: i64| {
            ((us as u64 * u64::from(info.sample_rate) + 999999) / 1000000) as usize
                * usize::from(info.channels)
                * 4
        };
        let expected = &full[boundary(from)..boundary(to).min(full.len())];
        assert!(!expected.is_empty());
        let output = dir.join(format!("{n}.wav"));
        let result =
            fvid::native_export::trim_audio_wave(&input, &output, from, to, None, None, None)
                .unwrap();
        assert_eq!(
            result.sample_frames as usize * usize::from(info.channels) * 4,
            expected.len()
        );
        let decoded = dir.join(format!("{n}-trim.f32le"));
        fvid::native_export::export_audio_pcm_selected(
            &output, &decoded, None, 1.0, None, None, None, None, None,
        )
        .unwrap();
        assert_eq!(std::fs::read(decoded).unwrap(), expected);
        let plan = fvid::native_plan::trim_audio(&input, from, to, None).unwrap();
        assert_eq!(plan.command, "trim");
        assert!(plan.notes.iter().any(|s| s.contains("backend: fvid")));
        let cli = dir.join(format!("{n}-cli.wav"));
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "trim"])
            .arg(&input)
            .arg(&cli)
            .args(["--from", "0.001", "--to", "0.005"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            std::fs::read(&cli).unwrap(),
            std::fs::read(&output).unwrap()
        );
        assert!(
            fvid::native_export::trim_audio_wave(&input, &output, from, to, None, None, None)
                .is_err()
        );
        let bad = dir.join(format!("{n}-bad.wav"));
        assert!(
            fvid::native_export::trim_audio_wave(&input, &bad, -1, to, None, None, None).is_err()
        );
        assert!(!bad.exists());
        assert!(
            fvid::native_export::trim_audio_wave(&input, &bad, from, to, Some(999), None, None)
                .is_err()
        );
        let cancelled = fvid::media_control::CancelFlag::new();
        cancelled.cancel();
        assert!(
            fvid::native_export::trim_audio_wave(
                &input,
                &bad,
                from,
                to,
                None,
                Some(&cancelled),
                None
            )
            .is_err()
        );
        assert!(!bad.exists());
        #[cfg(feature = "media")]
        {
            let api = dir.join(format!("{n}-api.wav"));
            let stats = fvid::media::trim(&input, &api, from, to, &Default::default()).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(std::fs::read(api).unwrap(), std::fs::read(&output).unwrap());
            assert_eq!(
                fvid::media::plan_trim(&input, from, to, &Default::default())
                    .unwrap()
                    .command,
                "trim"
            );
        }
    }
}

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG to mux two ALAC tracks"]
fn multiple_alac_tracks_require_explicit_selection_and_stay_native() {
    let binary = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let input = root.join("tests/fixtures/alac/stereo-24.m4a");
    let dir = std::env::temp_dir().join(format!("fvid-multi-alac-trim-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    for extension in ["m4a", "mka"] {
        let source = dir.join(format!("two.{extension}"));
        let result = std::process::Command::new(&binary)
            .args(["-v", "error", "-i"])
            .arg(&input)
            .args(["-map", "0:a:0", "-map", "0:a:0", "-c", "copy"])
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(fvid::native_media::is_owned_audio_trim_source(&source).unwrap());
        assert!(
            fvid::native_plan::trim_audio(&source, 1000, 5000, None)
                .unwrap_err()
                .contains("explicit")
        );
        #[cfg(feature = "media")]
        {
            let output = dir.join(format!("unselected-{extension}.wav"));
            assert!(
                fvid::media::trim(&source, &output, 1000, 5000, &Default::default())
                    .unwrap_err()
                    .contains("explicit")
            );
            assert!(!output.exists());
        }
        for index in [0, 1] {
            let output = dir.join(format!("{extension}-{index}.wav"));
            let cli = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
                .args(["media", "trim"])
                .arg(&source)
                .arg(&output)
                .args([
                    "--from",
                    "0.001",
                    "--to",
                    "0.005",
                    "--streams",
                    &index.to_string(),
                ])
                .output()
                .unwrap();
            assert!(
                cli.status.success(),
                "{}",
                String::from_utf8_lossy(&cli.stderr)
            );
            let stats: serde_json::Value = serde_json::from_slice(&cli.stdout).unwrap();
            assert_eq!(stats["backend"], "fvid");
            #[cfg(feature = "media")]
            {
                let api = dir.join(format!("api-{extension}-{index}.wav"));
                let options = fvid::media::CopyOptions {
                    streams: vec![index],
                    ..Default::default()
                };
                assert_eq!(
                    fvid::media::trim(&source, &api, 1000, 5000, &options)
                        .unwrap()
                        .backend,
                    "fvid"
                );
                assert_eq!(std::fs::read(api).unwrap(), std::fs::read(&output).unwrap());
            }
        }
        assert_eq!(
            std::fs::read(dir.join(format!("{extension}-0.wav"))).unwrap(),
            std::fs::read(dir.join(format!("{extension}-1.wav"))).unwrap()
        );
    }
}

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG to mux mixed audio tracks"]
fn selected_codec_controls_trim_in_mixed_aac_alac_container() {
    let binary = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let dir = std::env::temp_dir().join(format!("fvid-mixed-trim-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    for extension in ["m4a", "mka"] {
        let source = dir.join(format!("mixed.{extension}"));
        let result = std::process::Command::new(&binary)
            .args(["-v", "error", "-i"])
            .arg(root.join("alac/stereo-24.m4a"))
            .arg("-i")
            .arg(root.join("audio/aac-native-edit.m4a"))
            .args(["-map", "0:a:0", "-map", "1:a:0", "-c", "copy"])
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        for (index, codec) in [(0, "alac"), (1, "aac")] {
            let expected = dir.join(format!("{extension}-{index}-expected.wav"));
            fvid::native_export::export_audio_pcm_selected(
                &source,
                &expected,
                Some((
                    std::time::Duration::from_micros(1000),
                    std::time::Duration::from_micros(5000),
                )),
                1.0,
                None,
                None,
                Some(index),
                None,
                None,
            )
            .unwrap();
            let output = dir.join(format!("{extension}-{index}.wav"));
            fvid::native_export::trim_audio_wave(
                &source,
                &output,
                1000,
                5000,
                Some(index),
                None,
                None,
            )
            .unwrap();
            assert_eq!(
                std::fs::read(&output).unwrap(),
                std::fs::read(&expected).unwrap()
            );
            let plan = fvid::native_plan::trim_audio(&source, 1000, 5000, Some(index)).unwrap();
            assert_eq!(plan.streams[0].codec, codec);
            let cli = dir.join(format!("cli-{extension}-{index}.wav"));
            let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
                .args(["media", "trim"])
                .arg(&source)
                .arg(&cli)
                .args([
                    "--from",
                    "0.001",
                    "--to",
                    "0.005",
                    "--streams",
                    &index.to_string(),
                ])
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(
                std::fs::read(cli).unwrap(),
                std::fs::read(&expected).unwrap()
            );
            #[cfg(feature = "media")]
            {
                let api = dir.join(format!("api-{extension}-{index}.wav"));
                let options = fvid::media::CopyOptions {
                    streams: vec![index],
                    ..Default::default()
                };
                assert_eq!(
                    fvid::media::trim(&source, &api, 1000, 5000, &options)
                        .unwrap()
                        .backend,
                    "fvid"
                );
                assert_eq!(
                    std::fs::read(api).unwrap(),
                    std::fs::read(&expected).unwrap()
                );
            }
        }
    }
}
