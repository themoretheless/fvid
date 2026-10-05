use fvid::{
    native_export::export_aac_pcm_selected,
    native_plan::{AudioDecodeTransform, decode_audio_selected},
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio")
        .join(name)
}
#[test]
fn chosen_mp4_and_matroska_tracks_match_independent_pcm_references() {
    let dir =
        Directory(std::env::temp_dir().join(format!("fvid-aac-selection-{}", std::process::id())));
    std::fs::create_dir(&dir.0).unwrap();
    let mut mp4_pcm = Vec::new();
    for (name, stream, reference) in [
        (
            "two-audio.mp4",
            1,
            include_bytes!("fixtures/audio/two-audio-stream1-reference.f32le").as_slice(),
        ),
        (
            "two-audio.mp4",
            2,
            include_bytes!("fixtures/audio/two-audio-stream2-reference.f32le").as_slice(),
        ),
        (
            "two-audio.mka",
            0,
            include_bytes!("fixtures/audio/two-audio-mka0-reference.f32le").as_slice(),
        ),
        (
            "two-audio.mka",
            1,
            include_bytes!("fixtures/audio/two-audio-mka1-reference.f32le").as_slice(),
        ),
    ] {
        let source = fixture(name);
        let output = dir.0.join(format!("{name}-{stream}.f32le"));
        let stats = export_aac_pcm_selected(
            &source,
            &output,
            Some((Duration::ZERO, Duration::from_millis(40))),
            1.0,
            None,
            None,
            Some(stream),
            None,
            None,
        )
        .unwrap();
        let second = if name.ends_with("mp4") { stream == 2 } else { stream == 1 };
        assert_eq!(stats.sample_rate, if second { 32000 } else { 48000 });
        assert_eq!(stats.sample_frames, if second { 1280 } else { 1920 });
        let actual = std::fs::read(&output).unwrap();
        assert_eq!(actual.len(), reference.len());
        if name.ends_with("mp4") {
            mp4_pcm.push(actual.clone());
        } else {
            assert_eq!(actual, mp4_pcm[stream]);
        }
        let mut peak = 0.0f64;
        let mut squared = 0.0;
        for (a, b) in actual.as_chunks::<4>().0.iter().zip(reference.as_chunks::<4>().0.iter()) {
            let delta = f64::from(f32::from_le_bytes(*a))
                - f64::from(f32::from_le_bytes(*b));
            peak = peak.max(delta.abs());
            squared += delta * delta;
        }
        assert!(
            peak < 5e-4 && (squared / (actual.len() / 4) as f64).sqrt() < 1e-4,
            "{name}:{stream} peak={peak}"
        );
        let transform = AudioDecodeTransform {
            interval: Some((0, 40000)),
            ..Default::default()
        };
        assert_eq!(
            decode_audio_selected(&source, &transform, Some(stream))
                .unwrap()
                .streams[0]
                .index,
            stream
        );
        let plan_run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "decode-audio"])
            .arg(&source)
            .args(["--streams", &stream.to_string()])
            .output()
            .unwrap();
        assert!(
            plan_run.status.success(),
            "{}",
            String::from_utf8_lossy(&plan_run.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&plan_run.stdout).unwrap();
        assert_eq!(json["streams"][0]["index"], stream);
        let cli = dir.0.join(format!("cli-{name}-{stream}.f32le"));
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "decode-audio"])
            .arg(&source)
            .arg(&cli)
            .args([
                "--streams",
                &stream.to_string(),
                "--from",
                "0",
                "--to",
                "0.04",
            ])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(std::fs::read(cli).unwrap(), actual);
        {
            let public = dir.0.join(format!("public-{name}-{stream}.f32le"));
            let options = fvid::media::CopyOptions {
                streams: vec![stream],
                ..Default::default()
            };
            fvid::media::decode_audio_transformed(&source, &public, transform, &options).unwrap();
            assert_eq!(std::fs::read(public).unwrap(), actual);
            assert_eq!(
                fvid::media::plan_decode_audio(&source, &transform, &options)
                    .unwrap()
                    .streams[0]
                    .index,
                stream
            );
        }
    }
    let missing = dir.0.join("invalid.f32le");
    for (name, selected) in [
        ("two-audio.mp4", None),
        ("two-audio.mp4", Some(0)),
        ("two-audio.mp4", Some(3)),
        ("two-audio.mka", None),
        ("two-audio.mka", Some(2)),
        ("aac-mono-44k.aac", Some(1)),
        ("ac3-51.mka", Some(0)),
    ] {
        assert!(
            export_aac_pcm_selected(
                &fixture(name),
                &missing,
                None,
                1.0,
                None,
                None,
                selected,
                None,
                None
            )
            .is_err()
        );
        assert!(!missing.exists());
    }
}
