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
            (us as u64 * u64::from(info.sample_rate)).div_ceil(1000000) as usize
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
