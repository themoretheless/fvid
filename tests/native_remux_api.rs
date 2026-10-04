use fvid::{
    media::{self, CopyOptions},
    media_control::{CancelFlag, ProgressHook},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
struct Directory(PathBuf);
impl Directory {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("fvid-remux-api-{name}-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
#[test]
fn public_remux_preserves_native_output_and_reports_published_counters() {
    let dir = Directory::new("contents");
    for (i, name) in [
        "audio/aac-mono-44k.aac",
        "avc/hlg-vui-only.mp4",
        "hevc/main10-ipb.mp4",
        "audio/aac-native-edit.m4a",
    ]
    .iter()
    .enumerate()
    {
        let source = fixture(name);
        let output = dir.0.join(format!("{i}.mp4"));
        let expected = dir.0.join(format!("{i}-expected.mp4"));
        let native = if i == 0 {
            fvid::native_export::remux_adts_aac_stats(&source, &expected, None, None)
        } else {
            fvid::native_export::remux_mp4_stats(&source, &expected, None, None)
        }
        .unwrap();
        assert!(native.done);
        assert!(native.payload_bytes > 0);
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let published = output.clone();
        let options = CopyOptions {
            progress: Some(ProgressHook::new(move |event| {
                if event.done {
                    assert!(published.exists());
                }
                captured.lock().unwrap().push(event);
            })),
            ..Default::default()
        };
        let stats = media::remux(&source, &output, &options).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.segments, 1);
        assert_eq!(stats.packets, native.packets);
        assert_eq!(stats.payload_bytes, native.payload_bytes);
        assert_eq!(
            std::fs::read(&output).unwrap(),
            std::fs::read(expected).unwrap()
        );
        let events = events.lock().unwrap();
        let last = events.last().unwrap();
        assert!(last.done);
        assert_eq!(
            (last.packets, last.payload_bytes),
            (stats.packets, stats.payload_bytes)
        );
        assert_eq!(stats.packets, if i == 0 { 7 } else { 0 });
        let original = std::fs::read(&output).unwrap();
        assert!(media::remux(&source, &output, &CopyOptions::default()).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), original);
    }
}
#[test]
fn public_remux_rejects_unimplemented_options_and_cancels_before_publication() {
    let dir = Directory::new("cancel");
    for (i, name) in ["audio/aac-mono-44k.aac", "avc/hlg-vui-only.mp4"]
        .iter()
        .enumerate()
    {
        let source = fixture(name);
        let output = dir.0.join(format!("{i}.mp4"));
        let cancel = CancelFlag::new();
        let triggered = cancel.clone();
        let options = CopyOptions {
            cancel: Some(cancel),
            progress: Some(ProgressHook::new(move |event| {
                if event.payload_bytes > 0 && !event.done {
                    triggered.cancel();
                }
            })),
            ..Default::default()
        };
        assert!(
            media::remux(&source, &output, &options)
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(!output.exists());
        for options in [
            CopyOptions {
                streams: vec![0],
                ..Default::default()
            },
            CopyOptions {
                max_rss_bytes: Some(1),
                ..Default::default()
            },
            CopyOptions {
                metadata_set: vec![("title".into(), "new".into())],
                ..Default::default()
            },
        ] {
            assert!(
                media::remux(&source, &output, &options)
                    .unwrap_err()
                    .contains("does not yet support")
            );
            assert!(!output.exists());
        }
    }
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
}

#[test]
fn initialized_fragmented_mp4_stays_native_and_preserves_every_byte() {
    let dir = Directory::new("fragmented");
    let output = dir.0.join("out.mp4");
    let stats = media::remux(&fixture("video.mp4"), &output, &CopyOptions::default()).unwrap();
    assert_eq!(stats.backend, "fvid");
    assert_eq!(stats.packets, 0);
    assert!(stats.payload_bytes > 0);
    assert_eq!(
        std::fs::read(&output).unwrap(),
        std::fs::read(fixture("video.mp4")).unwrap()
    );
}
