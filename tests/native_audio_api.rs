#![cfg(feature = "media")]
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
    for options in [CopyOptions { streams: vec![0], ..Default::default() },
        CopyOptions { max_packets: Some(1), ..Default::default() },
        CopyOptions { metadata_set: vec![("title".into(), "x".into())], ..Default::default() }] {
        assert!(media::decode_audio(&source, &cancelled, &options).unwrap_err().contains("does not yet support"));
        assert!(!cancelled.exists());
    }
    assert!(media::decode_audio_interval(&source, &cancelled, Some((-1, 1)), &CopyOptions::default()).is_err());
    assert!(!cancelled.exists());
}
