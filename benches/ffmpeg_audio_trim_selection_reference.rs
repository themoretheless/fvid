//! Explicit mixed-track audio trim reference inputs and CLI/API checks.
use std::path::{Path, PathBuf};
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

fn main() {
    std::env::var_os("FVID_REFERENCE_FFMPEG").expect("Set FVID_REFERENCE_FFMPEG for this explicit reference benchmark");
    multiple_alac_tracks_require_explicit_selection_and_stay_native();
    selected_codec_controls_trim_in_mixed_aac_alac_container();
    println!("Mixed audio track trim selection reference suites passed");
}
