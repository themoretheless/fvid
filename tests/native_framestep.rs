use fvid::media::{DecodeTransform, decode_video_transformed};
use std::path::{Path, PathBuf};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

#[test]
fn owned_framestep_selects_native_codec_outputs_after_interval() {
    for name in ["playback-errors/ffv1-level-one-source.mp4", "hevc/main10-ipb.mp4",
        "vp9/adaptive.webm", "av1/ramp.webm", "playback-errors/shuffleplanes-444-8.mkv", "playback-errors/framestep-six-frames.y4m"] {
        let source = fixture(name);
        let mut reader = fvid::playback_native::NativeReader::software(
            std::io::BufReader::new(std::fs::File::open(&source).unwrap()), usize::MAX,
        ).unwrap();
        reader.read_frame_raw().unwrap().unwrap();
        let (start, _, scale) = reader.frame_interval().unwrap();
        let end = i64::try_from(start * 1_000_000 / u128::from(scale)).unwrap() + 1_000_000;
        for interval in [None, Some((0, end))] {
            let full = decode_video_transformed(&source, DecodeTransform {
                interval, ..Default::default()
            }).unwrap_or_else(|error| panic!("{name} {interval:?}: {error}"));
            for step in [1u64, 2, 3, 100] {
                let actual = decode_video_transformed(&source, DecodeTransform {
                    interval, framestep: Some(format!("step={step}")), ..Default::default()
                }).unwrap();
                assert_eq!(actual.backend, "fvid");
                assert_eq!(actual.video_frames, full.video_frames.div_ceil(step), "{name}");
                assert_eq!((actual.width, actual.height), (full.width, full.height));
                assert_eq!(actual.pixel_format, full.pixel_format);
            }
        }
    }
    let source = fixture("playback-errors/framestep-six-frames.y4m");
    let actual = decode_video_transformed(&source, DecodeTransform {
        interval: Some((250_000, 1_000_000)), framestep: Some("2".into()), ..Default::default()
    }).unwrap();
    assert_eq!(actual.video_frames, 2);
}

#[test]
fn dropped_frames_still_validate_storage_and_invalid_options_are_refused() {
    let source = fixture("playback-errors/framestep-discarded-truncated.y4m");
    let error = decode_video_transformed(&source, DecodeTransform {
        framestep: Some("2".into()), ..Default::default()
    }).unwrap_err();
    assert!(error.to_lowercase().contains("truncated") || error.contains("failed to fill whole buffer"), "{error}");
    for invalid in ["0", "-1", "2147483648", "step=n"] {
        assert!(decode_video_transformed(&fixture("playback-errors/ffv1-level-one-source.mp4"), DecodeTransform {
            framestep: Some(invalid.into()), ..Default::default()
        }).is_err());
    }
}

#[test]
fn framestep_cli_decodes_avc_without_legacy_feature() {
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode"]).arg(fixture("playback-errors/ffv1-level-one-source.mp4"))
        .args(["--framestep", "3"]).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let result: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(result["backend"], "fvid");
    assert_eq!(result["video_frames"], 4);
}
