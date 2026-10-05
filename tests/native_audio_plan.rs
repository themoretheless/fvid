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
fn rounded_interior_aac_duration_exports_exact_presentation_windows() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/aac-rounded-two-tracks.m4a");
    let mut reader = fvid::container::mp4::Mp4Reader::open(std::io::BufReader::new(std::fs::File::open(&source).unwrap()),Default::default()).unwrap();
    assert!((0..reader.tracks()[1].samples.len()).any(|i| reader.tracks()[1].samples.get(i).unwrap().duration == 1016));
    let config = reader.tracks()[1].configuration.clone();
    let mut decoder = fvid::codec::aac_native::NativeAacDecoder::new(fvid::codec::config::aac_specific_config(&config).unwrap()).unwrap();
    let mut expected = Vec::new();
    let mut packet = Vec::new();
    for index in 0..48 {
        reader.read_packet(1,index,&mut packet).unwrap();
        let samples = decoder.decode(&packet).unwrap();
        assert_eq!(samples.len(),2048);
        let retained = match index { 1 => 1016, 47 => 912, _ => 1024 };
        expected.extend_from_slice(&samples[..retained*2]);
    }
    // The edit removes 1008 priming samples from this synthetic source.
    let expected = &expected[1008*2..];
    // The movie edit ends at 48008 output samples, clipping 16 final samples.
    let expected: Vec<u8> = expected[..48008*2].iter().flat_map(|sample| sample.to_le_bytes()).collect();
    let output = std::env::temp_dir().join(format!("fvid-rounded-aac-{}.f32le",std::process::id()));
    let stats = fvid::native_export::export_audio_pcm_selected(&source,&output,None,1.0,None,None,Some(1),None,None).unwrap();
    assert_eq!(stats.sample_frames,48008);
    let actual = std::fs::read(&output).unwrap();
    assert_eq!(actual.len(),expected.len());
    let maximum = actual.as_chunks::<4>().0.iter().zip(expected.as_chunks::<4>().0.iter()).map(|(a,b)| (f32::from_le_bytes(*a)-f32::from_le_bytes(*b)).abs()).fold(0.0f32,f32::max);
    assert!(maximum < 1e-6,"PCM difference {maximum}");
    std::fs::remove_file(&output).unwrap();
    fvid::native_export::export_audio_pcm_selected(&source,&output,Some((std::time::Duration::from_millis(10),std::time::Duration::from_millis(30))),1.0,None,None,Some(1),None,None).unwrap();
    let actual = std::fs::read(&output).unwrap();
    assert_eq!(actual.len(),960*8);
    let maximum = actual.as_chunks::<4>().0.iter().zip(expected[480*8..1440*8].as_chunks::<4>().0.iter()).map(|(a,b)| (f32::from_le_bytes(*a)-f32::from_le_bytes(*b)).abs()).fold(0.0f32,f32::max);
    assert!(maximum < 1e-6,"interval PCM difference {maximum}");
    std::fs::remove_file(output).unwrap();
}
