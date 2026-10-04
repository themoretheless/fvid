use fvid::media::{self, AudioDecodeTransform, CopyOptions, CancelFlag, ProgressHook};
use std::{path::PathBuf, sync::{Arc, Mutex}};
struct Directory(PathBuf);
impl Directory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("fvid-audio-api-{name}-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio").join(name)
}
#[test]
fn public_api_matches_owned_export_for_all_aac_containers() {
    let dir = Directory::new("containers");
    for (i, name) in ["aac-mono-44k.aac", "aac-native-edit.m4a", "aac-stereo.mka", "aac-960-48000.m4a"].iter().enumerate() {
        let source = fixture(name);
        let expected = dir.0.join(format!("{i}-expected.f32le"));
        let actual = dir.0.join(format!("{i}-actual.f32le"));
        let stats = fvid::native_export::export_aac_pcm_controlled(
            &source, &expected, None, 1.0, None, None, None, None).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let published = actual.clone();
        let options = CopyOptions { progress: Some(ProgressHook::new(move |event| {
            if event.done { assert!(published.exists()); }
            captured.lock().unwrap().push(event);
        })), ..Default::default() };
        let result = media::decode_audio(&source, &actual, &options).unwrap();
        assert_eq!(std::fs::read(actual).unwrap(), std::fs::read(expected).unwrap());
        assert_eq!(result.sample_frames, stats.sample_frames);
        assert_eq!(result.sample_rate, stats.sample_rate as i32);
        assert_eq!(result.planar_interleave_bytes, 0);
        assert_eq!(result.decode_errors, 0);
        assert!(events.lock().unwrap().last().unwrap().done);
    }
}
#[test]
fn public_transforms_intervals_cancellation_and_options_are_honored() {
    let dir = Directory::new("transform");
    let source = fixture("aac-mono-44k.aac");
    let actual = dir.0.join("actual.wav");
    let expected = dir.0.join("expected.wav");
    let interval = (std::time::Duration::from_micros(10000), std::time::Duration::from_micros(50000));
    fvid::native_export::export_aac_pcm_controlled(&source, &expected, Some(interval), 0.5, Some(2), Some(48000), None, None).unwrap();
    let stats = media::decode_audio_transformed(&source, &actual, AudioDecodeTransform {
        interval: Some((10000, 50000)), volume: Some(0.5), channels: Some(2), sample_rate: Some(48000)
    }, &CopyOptions::default()).unwrap();
    assert_eq!((stats.sample_rate, stats.channels), (48000, 2));
    assert_eq!(std::fs::read(actual).unwrap(), std::fs::read(expected).unwrap());
    let out = dir.0.join("interval.f32le");
    let stats = media::decode_audio_interval(&source, &out, Some((10000, 50000)), &CopyOptions::default()).unwrap();
    assert_eq!(stats.sample_frames, 1764);
    let cancelled = dir.0.join("cancelled.f32le");
    let cancel = CancelFlag::new(); cancel.cancel();
    assert!(media::decode_audio(&source, &cancelled, &CopyOptions { cancel: Some(cancel), ..Default::default() }).unwrap_err().contains("cancelled"));
    assert!(!cancelled.exists());
    for options in [CopyOptions { streams: vec![0, 1], ..Default::default() },
        CopyOptions { max_controlled_bytes: Some(1), ..Default::default() },
        CopyOptions { metadata_set: vec![("title".into(), "x".into())], ..Default::default() }] {
        assert!(media::decode_audio(&source, &cancelled, &options).unwrap_err().contains("does not yet support"));
        assert!(!cancelled.exists());
    }
    assert!(media::decode_audio_interval(&source, &cancelled, Some((-1, 1)), &CopyOptions::default()).is_err());
    assert!(!cancelled.exists());
}

#[test]
fn native_plan_describes_the_actual_export_pipeline_and_rejects_unsupported_options() {
    for name in ["aac-mono-44k.aac", "aac-native-edit.m4a", "aac-stereo.mka", "aac-960-48000.m4a"] {
        let source = fixture(name);
        let info = fvid::native_media::aac_source_info(&source).unwrap();
        let transform = AudioDecodeTransform { interval: Some((10000, 50000)),
            sample_rate: Some(32000), channels: Some(if info.channels == 1 { 2 } else { 1 }), volume: Some(0.5) };
        let plan = media::plan_decode_audio(&source, &transform, &CopyOptions::default()).unwrap();
        assert_eq!(plan.streams[0].index, info.stream_index);
        assert_eq!(plan.streams[0].disposition, "decode");
        assert!(plan.graph.is_none());
        assert_eq!(plan.steps.iter().map(|s| s.action.as_str()).collect::<Vec<_>>(),
            ["decode", "trim", "rematrix", "volume", "resample", "write"]);
        assert!(!format!("{plan:?}").contains("libswresample"));
        for bad in [AudioDecodeTransform { channels: Some(0), ..transform },
            AudioDecodeTransform { channels: Some(7), ..transform },
            AudioDecodeTransform { volume: Some(f64::NAN), ..transform },
            AudioDecodeTransform { sample_rate: Some(0), ..transform },
            AudioDecodeTransform { interval: Some((10, 1)), ..transform }] {
            assert!(media::plan_decode_audio(&source, &bad, &CopyOptions::default()).is_err());
        }
        assert!(media::plan_decode_audio(&source, &transform,
            &CopyOptions { max_packets: Some(1), ..Default::default() }).unwrap().notes.iter().any(|note| note.contains("packet work limit: Some(1)")));
    }
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "decode-audio"]).arg(fixture("aac-mono-44k.aac"))
        .args(["--rate", "32000", "--channels", "2", "--volume", "0.5"]).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let json: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(json["streams"][0]["disposition"], "decode");
    assert!(json["notes"][0].as_str().unwrap().contains("backend: fvid"));
}

#[test]
fn explicit_alac_track_selection_uses_owned_plan_and_decoder() {
    let dir = Directory::new("selected-alac");
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/alac-two-tracks.m4a");
    assert!(media::plan_decode_audio(&source, &Default::default(), &Default::default()).unwrap_err().contains("explicit"));
    let absent = dir.0.join("ambiguous.wav");
    assert!(media::decode_audio(&source, &absent, &Default::default()).unwrap_err().contains("explicit"));
    assert!(!absent.exists());
    for index in [0, 1] {
        let options = CopyOptions { streams: vec![index], ..Default::default() };
        let plan = media::plan_decode_audio(&source, &Default::default(), &options).unwrap();
        assert_eq!(plan.streams[0].index, index);
        assert_eq!(plan.streams[0].codec, "alac");
        let expected = dir.0.join(format!("expected-{index}.wav"));
        let actual = dir.0.join(format!("actual-{index}.wav"));
        fvid::native_export::export_audio_pcm_selected(&source, &expected, None, 1.0, None, None, Some(index), None, None).unwrap();
        let decoded = media::decode_audio(&source, &actual, &options).unwrap();
        assert!(decoded.sample_frames > 0);
        assert_eq!(std::fs::read(actual).unwrap(), std::fs::read(expected).unwrap());
    }
}

#[test]
fn public_packet_controls_limit_owned_pcm_and_fail_atomically() {
    let dir = Directory::new("packet-controls");
    let source = fixture("aac-mono-44k.aac");
    let full = dir.0.join("full.f32le");
    let limited = dir.0.join("limited.f32le");
    let transform = AudioDecodeTransform { volume: Some(0.5), ..Default::default() };
    let full_stats = media::decode_audio_transformed(&source, &full, transform, &Default::default()).unwrap();
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let saved = events.clone();
    let stats = media::decode_audio_transformed(&source, &limited, transform, &CopyOptions {
        max_packets: Some(1),
        progress: Some(ProgressHook::new(move |event| saved.lock().unwrap().push(event))),
        ..Default::default()
    }).unwrap();
    assert_eq!(stats.decoded_frames, 1);
    assert!(stats.sample_frames > 0 && stats.sample_frames < full_stats.sample_frames);
    let bytes = std::fs::read(&limited).unwrap();
    assert_eq!(bytes, std::fs::read(full).unwrap()[..bytes.len()]);
    let events = events.lock().unwrap();
    assert_eq!(events.iter().filter(|event| event.done).count(), 1);
    assert_eq!(events.last().unwrap().packets, 1);
    for options in [
        CopyOptions { max_packet_bytes: 1, ..Default::default() },
        CopyOptions { max_rss_bytes: Some(1), ..Default::default() },
    ] {
        let output = dir.0.join("rejected.wav");
        assert!(media::decode_audio_transformed(&source, &output, transform, &options).is_err());
        assert!(!output.exists());
    }
}

#[test]
fn wave_packet_work_and_payload_limits_are_enforced() {
    let dir = Directory::new("wave-controls");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-packet-controls.wav");
    let output = dir.0.join("limited.f32le");
    let stats = media::decode_audio(&source, &output, &CopyOptions {
        max_packet_bytes: 128, max_packets: Some(2), ..Default::default()
    }).unwrap();
    assert_eq!(stats.decoded_frames, 2);
    assert_eq!(stats.sample_frames, 64);
    let expected: Vec<u8> = (0..64).flat_map(|index| (index as f32 / 1000.0).to_le_bytes()).collect();
    assert_eq!(std::fs::read(output).unwrap(), expected);
    let rejected = dir.0.join("rejected.wav");
    assert!(media::decode_audio(&source, &rejected, &CopyOptions {
        max_packet_bytes: 1, ..Default::default()
    }).is_err());
    assert!(!rejected.exists());
}
