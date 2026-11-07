use fvid_control::CopyOptions;
use fvid_media_info::{DecodeTransform, LosslessTransform, OverlaySpec};
use std::path::{Path, PathBuf};

fn unique_temp_dir(label: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("fvid-{label}-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}

#[test]
fn y4m_probe_and_supports_transformed_reuse_opened_header() {
    let source = fixtures_root().join("framestep-six-frames.y4m");
    let overlay_path = fixtures_root().join("overlay-secondary-clock.y4m");
    let info = fvid_media::owned_y4m_probe::try_y4m(&source)
        .expect("try_y4m ok")
        .expect("recognized Y4M");
    assert_eq!(info.format, "yuv4mpegpipe");
    assert_eq!(info.streams.len(), 1);
    assert_eq!(info.streams[0].width, 4);
    assert_eq!(info.streams[0].height, 4);

    let transform = DecodeTransform {
        framestep: Some("2".into()),
        overlay: Some(OverlaySpec {
            path: overlay_path,
            x: 2,
            y: 2,
        }),
        ..Default::default()
    };
    let stats = fvid_media::decode_video_transformed(&source, transform).expect("decode transformed");
    assert_eq!(stats.video_frames, 3);
}

#[test]
fn webm_probe_pre_reserves_stream_and_chapter_tables() {
    let source = fixtures_root().join("webm-probe-clear.mkv");
    let info = fvid_media::owned_webm_probe::probe_webm(&source).expect("probe webm");
    assert_eq!(info.format, "matroska,webm");
    assert_eq!(info.streams.len(), 1);
    assert_eq!(info.streams[0].codec, "ffv1");
}

#[test]
fn transcode_lossless_rejects_existing_destination_before_transform_validation() {
    let dir = unique_temp_dir("lossless-dest-exists");
    let source = fixtures_root().join("framestep-six-frames.y4m");
    let dest = dir.join("already-exists.mkv");
    std::fs::write(&dest, b"sentinel").expect("write sentinel");

    // Even with an invalid/unsupported transform or tiny packet budget, existing
    // destination must fail immediately with "output already exists" and leave
    // the destination untouched.
    let bad_transform = LosslessTransform {
        gblur: Some("sigma=-999".into()),
        ..Default::default()
    };
    let tight_options = CopyOptions {
        max_packet_bytes: 1,
        ..Default::default()
    };
    let err = fvid_media::transcode_lossless(&source, &dest, bad_transform, &tight_options)
        .expect_err("must reject existing destination first");
    assert!(
        err.contains("output already exists"),
        "expected 'output already exists', got: {err}"
    );
    assert_eq!(std::fs::read(&dest).expect("read sentinel"), b"sentinel");
    let _ = std::fs::remove_dir_all(&dir);
}
