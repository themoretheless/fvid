use fvid_media::{owned_frame::GeometryFrame, owned_smartblur::SmartBlur};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
fn frame(n: usize) -> GeometryFrame {
    GeometryFrame {
        width: 16,
        height: 12,
        subsampling: Some([2, 2]),
        data: (0..288)
            .map(|i| ((i * 37 + n * 23 + 101) % 256) as u8)
            .collect(),
    }
}
#[test]
fn independent_goldens_accept_blur_sharpen_thresholds_and_rewind() {
    for (args, name) in [("", "blur"), ("lr=5:ls=-1:lt=-30", "sharpen")] {
        let expected = std::fs::read(fixture(&format!("smartblur-{name}.expected.raw"))).unwrap();
        let filter = SmartBlur::parse(args).unwrap();
        for _ in 0..2 {
            let mut result = Vec::new();
            for n in 0..4 {
                let mut f = frame(n);
                filter
                    .apply(&mut f, 8, n as u64, Some(n as f64 / 25.))
                    .unwrap();
                result.extend(f.data);
            }
            assert_eq!(result, expected);
        }
    }
    let mut a = frame(0);
    let mut b = frame(0);
    SmartBlur::parse("1.5:-0.4:7:0:-2:-31:-0.9:-2:-31")
        .unwrap()
        .apply(&mut a, 8, 0, None)
        .unwrap();
    SmartBlur::parse("luma_radius=1.5:luma_strength=-0.4:luma_threshold=7")
        .unwrap()
        .apply(&mut b, 8, 0, None)
        .unwrap();
    assert_eq!(a.data, b.data);
}
#[test]
fn root_library_exports_clip_and_select_only_after_filtering() {
    use fvid::{
        media::{CopyOptions, LosslessTransform},
        playback_native::NativeReader,
    };
    let source = fixture("smartblur-edges.y4m");
    let base = std::env::temp_dir().join(format!("fvid-smartblur-base-{}.mkv", std::process::id()));
    fvid::media::transcode_lossless(&source, &base, Default::default(), &CopyOptions::default())
        .unwrap();
    for input in [&source, &base] {
        for library in [false, true] {
            for step in [1usize, 2] {
                let output = std::env::temp_dir().join(format!(
                    "fvid-smartblur-{}-{library}-{step}.mkv",
                    std::process::id()
                ));
                let transform = LosslessTransform {
                    smartblur: Some(
                        if step == 2 {
                            "lr=5:ls=-1:lt=-30:enable=eq(n,2)"
                        } else {
                            "lr=5:ls=-1:lt=-30"
                        }
                        .into(),
                    ),
                    framestep: Some(step.to_string()),
                    ..Default::default()
                };
                let stats = if library {
                    fvid_media::transcode_lossless(
                        input,
                        &output,
                        transform,
                        &CopyOptions::default(),
                    )
                } else {
                    fvid::media::transcode_lossless(
                        input,
                        &output,
                        transform,
                        &CopyOptions::default(),
                    )
                }
                .unwrap();
                assert_eq!(stats.backend, "fvid");
                assert_eq!(stats.video_frames, 4 / step as u64);
                let expected = std::fs::read(fixture("smartblur-sharpen.expected.raw")).unwrap();
                let mut reader = NativeReader::software(
                    Cursor::new(std::fs::read(&output).unwrap()),
                    usize::MAX,
                )
                .unwrap();
                for n in (0..4).step_by(step) {
                    let f = reader.read_frame_raw().unwrap().unwrap();
                    let result = fvid::native_geometry::VideoGeometry::default()
                        .apply(&f, 16, 12)
                        .unwrap();
                    if step == 2 && n == 0 {
                        assert_eq!(result.data, frame(0).data);
                    } else {
                        assert_eq!(result.data, &expected[n * 288..(n + 1) * 288]);
                    }
                }
                assert!(reader.read_frame_raw().unwrap().is_none());
                std::fs::remove_file(output).unwrap();
            }
        }
    }
    for library in [false, true] {
        let output = std::env::temp_dir().join(format!(
            "fvid-smartblur-clip-{}-{library}.mkv",
            std::process::id()
        ));
        let transform = LosslessTransform {
            smartblur: Some("enable='eq(n,0)*gte(t,.04)'".into()),
            interval: Some((40_000, 120_000)),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::transcode_lossless(&source, &output, transform, &CopyOptions::default())
        } else {
            fvid::media::transcode_lossless(&source, &output, transform, &CopyOptions::default())
        }
        .unwrap();
        assert_eq!(stats.video_frames, 2);
        let mut reader =
            NativeReader::software(Cursor::new(std::fs::read(&output).unwrap()), usize::MAX)
                .unwrap();
        let expected = std::fs::read(fixture("smartblur-blur.expected.raw")).unwrap();
        for (n, wanted) in [(1, expected[288..576].to_vec()), (2, frame(2).data)] {
            let f = reader.read_frame_raw().unwrap().unwrap();
            let result = fvid::native_geometry::VideoGeometry::default()
                .apply(&f, 16, 12)
                .unwrap();
            assert_eq!(result.data, wanted, "source frame {n}");
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
        std::fs::remove_file(output).unwrap();
    }
    std::fs::remove_file(base).unwrap();
}
#[test]
fn precision_odd_chroma_alpha_gray_and_refusal_before_mutation() {
    for depth in [8, 9, 10, 12, 14, 16] {
        for args in ["", "ls=-1:lr=5", "lt=30", "lt=-30", "lr=0.1"] {
            let max = (1u32 << depth) - 1;
            let values = if depth == 8 {
                vec![max as u8; 27]
            } else {
                (0..27).flat_map(|_| (max as u16).to_le_bytes()).collect()
            };
            let mut f = GeometryFrame {
                width: 5,
                height: 3,
                subsampling: Some([2, 2]),
                data: values,
            };
            let original = f.data.clone();
            SmartBlur::parse(args)
                .unwrap()
                .apply(&mut f, depth, 0, None)
                .unwrap();
            assert_eq!(f.data, original, "{depth} {args}");
        }
    }
    let mut alpha: Vec<u8> = (0..192).map(|i| ((i * 53) % 256) as u8).collect();
    let mut luma = alpha.clone();
    let filter = SmartBlur::parse("ar=3:as=-1:at=-7").unwrap();
    filter
        .apply_plane(&mut alpha, 16, 12, 8, 2, 0, None)
        .unwrap();
    SmartBlur::parse("lr=3:ls=-1:lt=-7")
        .unwrap()
        .apply_plane(&mut luma, 16, 12, 8, 0, 0, None)
        .unwrap();
    assert_eq!(alpha, luma);
    let filter = SmartBlur::parse("enable=0").unwrap();
    let mut f = frame(0);
    let original = f.data.clone();
    filter.apply(&mut f, 8, 0, None).unwrap();
    assert_eq!(f.data, original);
    f.data.pop();
    let original = f.data.clone();
    assert!(filter.apply(&mut f, 8, 0, None).is_err());
    assert_eq!(f.data, original);
    for options in [
        "lr=0",
        "lr=6",
        "ls=-1.1",
        "lt=1.5",
        "ct=-32",
        "ar=-1",
        "as=-2.1",
        "enable='nope'",
        "lr=NAN",
        "xx=1",
    ] {
        assert!(SmartBlur::parse(options).is_err(), "{options}");
    }
    let mut invalid = [255u8; 2];
    assert!(
        SmartBlur::parse("")
            .unwrap()
            .apply_plane(&mut invalid, 1, 1, 10, 0, 0, None)
            .is_err()
    );
    assert_eq!(invalid, [255; 2]);
}
#[test]
fn native_sources_and_cli_decode_export_use_owned_filter() {
    use fvid::media::DecodeTransform;
    for source in [
        "../video.mp4",
        "../hevc/main10-ipb.mp4",
        "../vp9/adaptive.webm",
        "../av1/ramp.webm",
    ] {
        let request = DecodeTransform {
            smartblur: Some("lt=8:ct=-8".into()),
            ..Default::default()
        };
        let stats = fvid::media::decode_video_transformed(&fixture(source), request).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
    let source = fixture("smartblur-edges.y4m");
    let output =
        std::env::temp_dir().join(format!("fvid-smartblur-cli-{}.y4m", std::process::id()));
    for args in [
        vec![
            "media",
            "decode",
            source.to_str().unwrap(),
            "--smartblur",
            "",
            "--quiet",
        ],
        vec![
            "media",
            "export-y4m",
            source.to_str().unwrap(),
            output.to_str().unwrap(),
            "--smartblur",
            "",
        ],
    ] {
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let mut reader = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(&output).unwrap()),
        usize::MAX,
    )
    .unwrap();
    let expected = std::fs::read(fixture("smartblur-blur.expected.raw")).unwrap();
    let mut decoded = Vec::new();
    while let Some(f) = reader.read_frame_raw().unwrap() {
        decoded.extend(
            fvid::native_geometry::VideoGeometry::default()
                .apply(&f, 16, 12)
                .unwrap()
                .data,
        );
    }
    assert_eq!(decoded, expected);
    std::fs::remove_file(output).unwrap();
    let output =
        std::env::temp_dir().join(format!("fvid-smartblur-cli-{}.mkv", std::process::id()));
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "transcode-lossless",
            source.to_str().unwrap(),
            output.to_str().unwrap(),
            "--smartblur",
            "",
            "--from",
            "0.04",
            "--to",
            "0.12",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut reader = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(&output).unwrap()),
        usize::MAX,
    )
    .unwrap();
    for n in [1, 2] {
        let f = reader.read_frame_raw().unwrap().unwrap();
        let pixels = fvid::native_geometry::VideoGeometry::default()
            .apply(&f, 16, 12)
            .unwrap()
            .data;
        assert_eq!(pixels, &expected[n * 288..(n + 1) * 288]);
    }
    assert!(reader.read_frame_raw().unwrap().is_none());
    std::fs::remove_file(output).unwrap();
}
