use fvid::native_plan::{decode_audio, AudioDecodeTransform};
use std::path::{Path, PathBuf};
fn fixture(name: &str) -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio").join(name) }
#[test]
fn owned_plans_and_cli_are_available_without_media_feature() {
    for name in ["aac-mono-44k.aac", "aac-native-edit.m4a", "aac-stereo.mka", "aac-960-48000.m4a"] {
        let source = fixture(name);
        let transform = AudioDecodeTransform { interval: Some((10000,50000)), sample_rate: Some(32000), channels: Some(2), volume: Some(0.5) };
        let plan = decode_audio(&source, &transform).unwrap();
        assert_eq!(plan.command, "decode-audio");
        assert_eq!(plan.streams[0].codec, "aac");
        assert_eq!(plan.steps.first().unwrap().action, "decode");
        assert_eq!(plan.steps.last().unwrap().action, "write");
        assert!(plan.graph.is_none());
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media","plan","decode-audio"]).arg(&source)
            .args(["--from","0.01","--to","0.05","--rate","32000","--channels","2","--volume","0.5"])
            .output().unwrap();
        assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&run.stdout).unwrap(), serde_json::to_value(&plan).unwrap());
        #[cfg(feature="media")]
        {
            let compatible: fvid::media::MediaPlan = plan.clone();
            assert_eq!(compatible, fvid::media::plan_decode_audio(&source, &transform, &Default::default()).unwrap());
        }
    }
}
#[test]
fn native_plan_rejects_invalid_requests_before_execution() {
    let source = fixture("aac-mono-44k.aac");
    for options in [vec!["--from","0.1"], vec!["--volume","NaN"], vec!["--channels","7"],
        vec!["--rate","0"], vec!["--rate","32000","--sample-rate","48000"],
        vec!["--from","2","--to","1"], vec!["--streams","1"], vec!["--volume"]] {
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media","plan","decode-audio"]).arg(&source).args(options).output().unwrap();
        assert!(!run.status.success()); assert!(run.stdout.is_empty()); assert!(!run.stderr.is_empty());
    }
}

#[test]
fn selected_alac_cli_decodes_and_plans_without_ffmpeg_backend() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/alac-two-tracks.m4a");
    let directory = std::env::temp_dir().join(format!("fvid-selected-alac-cli-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let _cleanup = Cleanup(directory.clone());
    for index in [0,1] {
        let plan = fvid::native_plan::decode_audio_selected(&source, &Default::default(), Some(index)).unwrap();
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media","plan","decode-audio"]).arg(&source).args(["--streams",&index.to_string()]).output().unwrap();
        assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&run.stdout).unwrap(), serde_json::to_value(plan).unwrap());
        let output = directory.join(format!("selected-{index}.wav"));
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media","decode-audio"]).arg(&source).arg(&output).args(["--streams",&index.to_string()]).output().unwrap();
        assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
        let expected = directory.join(format!("expected-{index}.wav"));
        fvid::native_export::export_audio_pcm_selected(&source,&expected,None,1.0,None,None,Some(index),None,None).unwrap();
        assert_eq!(std::fs::read(output).unwrap(),std::fs::read(expected).unwrap());
    }
}
