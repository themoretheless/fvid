//! Cancellation after staging PCM must not publish or leak private audio.
use std::path::PathBuf;
#[test]
fn cancelled_complex_window_removes_private_pcm_and_final_output() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let source = root.join("hevc-complex-audio-8-gap.mkv");
    let output =
        std::env::temp_dir().join(format!("fvid-pcm-window-abort-{}.mkv", std::process::id()));
    assert!(!output.exists());
    let cancel = fvid_control::CancelFlag::new();
    let trigger = cancel.clone();
    let options = fvid_control::CopyOptions {
        cancel: Some(cancel),
        progress: Some(fvid_control::ProgressHook::new(move |event| {
            assert!(
                !event.done,
                "cancelled operation must not report publication"
            );
            if event.packets > 0 {
                trigger.cancel();
            }
        })),
        ..Default::default()
    };
    let error = fvid_media::transcode_lossless(
        &source,
        &output,
        fvid_media::LosslessTransform {
            horizontal_flip: true,
            interval: Some((67_123, 199_789)),
            ..Default::default()
        },
        &options,
    )
    .unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert!(!output.exists());
    let prefix = format!("fvid-video-companions-{}-", std::process::id());
    assert!(
        !std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e.file_name().to_string_lossy().starts_with(&prefix)),
        "cancelled operation leaked PCM staging files"
    );
}
