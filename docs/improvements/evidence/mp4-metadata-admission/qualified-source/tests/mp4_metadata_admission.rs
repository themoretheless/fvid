use fvid::{
    container::mp4::{Limits, Mp4Reader},
    media::CopyOptions,
};
use std::{fs::File, io::BufReader, path::PathBuf};
fn source(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-metadata-admission")
        .join(name)
}
#[test]
fn parser_metadata_limit_rejects_padding_before_loading_it() {
    let error = match Mp4Reader::open(
        BufReader::new(File::open(source("padded.mp4")).unwrap()),
        Limits {
            metadata_bytes: 64 << 10,
            ..Default::default()
        },
    ) {
        Ok(_) => panic!("metadata parser must reject oversized moov"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("metadata exceeds budget"),
        "{error}"
    );
}
#[test]
fn controlled_remux_plan_does_not_bypass_parser_metadata_limit() {
    let options = CopyOptions {
        max_controlled_bytes: Some(64 << 10),
        ..Default::default()
    };
    let error =
        fvid_media::owned_mp4_remux::plan_remux(&source("padded.mp4"), &options).unwrap_err();
    assert!(error.contains("metadata exceeds budget"), "{error}");
}
#[test]
fn small_metadata_and_ample_budget_controls_are_valid() {
    for (name, budget) in [("small.mp4", 64 << 10), ("padded.mp4", 2 << 20)] {
        let options = CopyOptions {
            max_controlled_bytes: Some(budget),
            ..Default::default()
        };
        assert!(fvid_media::owned_mp4_remux::plan_remux(&source(name), &options).is_ok());
    }
}
#[test]
fn route_inspection_and_concat_propagate_metadata_admission() {
    let padded = source("padded.mp4");
    let options = CopyOptions {
        max_controlled_bytes: Some(64 << 10),
        ..Default::default()
    };
    let error = fvid::native_export::is_native_mp4_matroska_with_memory_limit(
        &padded,
        options.max_controlled_bytes,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("metadata exceeds budget"),
        "{error}"
    );
    let error = fvid::media::plan_remux(&padded, &options).unwrap_err();
    assert!(error.contains("metadata exceeds budget"), "{error}");
    let inputs = [padded.clone(), padded];
    let result = fvid::container::mp4_concat::open_with_options(&inputs, &options);
    assert!(matches!(result,Err(e) if e.to_string().contains("metadata exceeds budget")));
    let error = fvid::media::plan_concat(&inputs, &options).unwrap_err();
    assert!(error.contains("metadata exceeds budget"), "{error}");
}
#[test]
fn remux_budget_errors_remain_specific_and_preserve_destination() {
    let dir = std::env::temp_dir().join(format!("fvid-metadata-admission-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let out = dir.join("untouched.mkv");
    std::fs::write(&out, b"preserve existing destination").unwrap();
    let options = CopyOptions {
        max_controlled_bytes: Some(64 << 10),
        ..Default::default()
    };
    for route in 0..3 {
        let result = match route {
            0 => fvid_media::owned_mp4_remux::remux(&source("padded.mp4"), &out, &options),
            1 => fvid::media::remux(&source("padded.mp4"), &out, &options),
            _ => fvid_media::remux(&source("padded.mp4"), &out, &options),
        };
        let error = result.unwrap_err();
        assert!(error.contains("metadata exceeds budget"), "{error}");
        assert_eq!(
            std::fs::read(&out).unwrap(),
            b"preserve existing destination"
        );
    }
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "refusal must not leave temporary output"
    );
}
#[cfg(feature = "player")]
#[test]
fn ample_budget_remux_retains_pixels_timestamps_and_seek() {
    use fvid::playback_native::NativeReader;
    use std::time::Duration;
    let padded = source("padded.mp4");
    let mut original =
        NativeReader::software(BufReader::new(File::open(&padded).unwrap()), 32 << 20).unwrap();
    let expected = original
        .read_frame_raw()
        .unwrap()
        .unwrap()
        .into_rgb(32 << 20)
        .unwrap();
    let dir = std::env::temp_dir().join(format!("fvid-metadata-ample-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let output = dir.join("out.mkv");
    let options = CopyOptions {
        max_controlled_bytes: Some(2 << 20),
        ..Default::default()
    };
    assert_eq!(
        fvid::media::remux(&padded, &output, &options)
            .unwrap()
            .packets,
        1
    );
    let mut result =
        NativeReader::software(BufReader::new(File::open(output).unwrap()), 32 << 20).unwrap();
    assert_eq!(result.duration(), Some(Duration::from_millis(40)));
    assert_eq!(
        result
            .read_frame_raw()
            .unwrap()
            .unwrap()
            .into_rgb(32 << 20)
            .unwrap(),
        expected
    );
    assert!(result.read_frame_raw().unwrap().is_none());
    assert!(result.read_frame_raw().unwrap().is_none());
    result.rewind().unwrap();
    assert_eq!(
        result
            .read_frame_raw()
            .unwrap()
            .unwrap()
            .into_rgb(32 << 20)
            .unwrap(),
        expected
    );
    for ms in [0, 20, 39, 40, 100] {
        assert_eq!(
            result
                .seek_raw(Duration::from_millis(ms))
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
            expected
        );
    }
}
#[test]
fn unsupported_sample_entry_keeps_its_existing_refusal_route() {
    let input = source("unsupported-entry.mp4");
    let error = match fvid_media::owned_mp4::Mp4Reader::open(
        BufReader::new(File::open(&input).unwrap()),
        Default::default(),
    ) {
        Ok(_) => panic!("unknown entry must remain unsupported"),
        Err(error) => error,
    };
    assert!(error.is_unsupported(), "{error}");
    let dir =
        std::env::temp_dir().join(format!("fvid-metadata-unsupported-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let error =
        fvid_media::owned_mp4_remux::remux(&input, &dir.join("out.mkv"), &CopyOptions::default())
            .unwrap_err();
    assert_eq!(error, "request is not an owned MP4 Matroska remux");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}
