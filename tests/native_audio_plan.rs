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

fn selected_audio_cli(file: &str, codec: &str) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors").join(file);
    let directory = std::env::temp_dir().join(format!("fvid-selected-{codec}-cli-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let _cleanup = Cleanup(directory.clone());
    for index in [0,1] {
        let plan = fvid::native_plan::decode_audio_selected(&source, &Default::default(), Some(index)).unwrap();
        assert_eq!(plan.streams[0].index,index);
        assert_eq!(plan.streams[0].codec,codec);
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

#[test]
fn selected_alac_cli_decodes_and_plans_without_ffmpeg_backend() {
    selected_audio_cli("alac-two-tracks.m4a", "alac");
}
#[test]
fn selected_aac_cli_decodes_and_plans_without_ffmpeg_backend() {
    selected_audio_cli("aac-two-tracks.m4a", "aac");
}

#[test]
fn rounded_interior_aac_duration_is_reproduced_before_acceptance() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/aac-rounded-two-tracks.m4a");
    let mut reader = fvid::container::mp4::Mp4Reader::open(std::io::BufReader::new(std::fs::File::open(&source).unwrap()),Default::default()).unwrap();
    assert!((0..reader.tracks()[1].samples.len()).any(|i| reader.tracks()[1].samples.get(i).unwrap().duration == 1016));
    // This is a refusal regression, not proof that irregular AAC timing is supported.
    let mut packet = Vec::new();
    reader.read_packet(1,1,&mut packet).unwrap();
    let output = std::env::temp_dir().join(format!("fvid-rounded-aac-{}.wav",std::process::id()));
    let error = fvid::native_export::export_audio_pcm_selected(&source,&output,None,1.0,None,None,Some(1),None,None).unwrap_err();
    assert!(error.to_string().contains("short interior MP4 audio packet"),"{error}");
    assert!(!output.exists());
}
