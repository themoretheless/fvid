use fvid_media::{owned_frame::GeometryFrame, owned_sab::Sab};
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
fn independent_goldens_accept_blur_smooth_guidance_and_rewind() {
    for (args, name) in [("", "blur"), ("lr=4:lpfr=2:ls=100", "smooth")] {
        let expected = std::fs::read(fixture(&format!("sab-{name}.expected.raw"))).unwrap();
        let filter = Sab::parse(args).unwrap();
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
    Sab::parse("1.5:0.7:7:0:-0.9:0")
        .unwrap()
        .apply(&mut a, 8, 0, None)
        .unwrap();
    Sab::parse("luma_radius=1.5:luma_pre_filter_radius=0.7:luma_strength=7")
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
    let source = fixture("sab-guidance-edges.y4m");
    let base = std::env::temp_dir().join(format!("fvid-sab-base-{}.mkv", std::process::id()));
    fvid::media::transcode_lossless(&source, &base, Default::default(), &CopyOptions::default())
        .unwrap();
    for input in [&source, &base] {
        for library in [false, true] {
            for step in [1usize, 2] {
                let output = std::env::temp_dir().join(format!(
                    "fvid-sab-{}-{library}-{step}.mkv",
                    std::process::id()
                ));
                let transform = LosslessTransform {
                    sab: Some(
                        if step == 2 {
                            "lr=4:lpfr=2:ls=100:enable=eq(n,2)"
                        } else {
                            "lr=4:lpfr=2:ls=100"
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
                let expected = std::fs::read(fixture("sab-smooth.expected.raw")).unwrap();
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
            "fvid-sab-clip-{}-{library}.mkv",
            std::process::id()
        ));
        let transform = LosslessTransform {
            sab: Some("enable='eq(n,0)*gte(t,.04)'".into()),
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
        let expected = std::fs::read(fixture("sab-blur.expected.raw")).unwrap();
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
fn precision_odd_and_degenerate_planes_preserve_flat_fields() {
    for args in ["", "lr=4:lpfr=2:ls=100", "lr=0.1", "ls=0.1"] {
        let filter = Sab::parse(args).unwrap();
        for depth in [8, 9, 10, 12, 14, 16, 8] {
            let max = (1u32 << depth) - 1;
            for (width, height, count) in [(5, 3, 27), (1, 1, 3), (1, 5, 11), (5, 1, 11)] {
                let values = if depth == 8 {
                    vec![max as u8; count]
                } else {
                    (0..count)
                        .flat_map(|_| (max as u16).to_le_bytes())
                        .collect()
                };
                let mut f = GeometryFrame {
                    width,
                    height,
                    subsampling: Some([2, 2]),
                    data: values,
                };
                let original = f.data.clone();
                filter.apply(&mut f, depth, 0, None).unwrap();
                assert_eq!(f.data, original, "{depth} {args} {width}x{height}");
            }
        }
    }
    // A one-pixel axis has reflection period zero; no repeated reflection loop.
    let source = fixture("sab-single-pixel.y4m");
    for library in [false, true] {
        let request = fvid::media::DecodeTransform {
            sab: Some("lr=4:lpfr=2:ls=100".into()),
            ..Default::default()
        };
        let result = if library {
            fvid_media::decode_video_transformed(&source, request)
        } else {
            fvid::media::decode_video_transformed(&source, request)
        }
        .unwrap();
        assert_eq!(result.video_frames, 4);
        assert_eq!((result.width, result.height), (1, 1));
    }
    let mut gray: Vec<u8> = (0..192).map(|i| ((i * 53) % 256) as u8).collect();
    let mut chroma = gray.clone();
    Sab::parse("lr=2:lpfr=0.7:ls=20")
        .unwrap()
        .apply_plane(&mut gray, 16, 12, 8, 0, 0, None)
        .unwrap();
    Sab::parse("cr=2:cpfr=0.7:cs=20")
        .unwrap()
        .apply_plane(&mut chroma, 16, 12, 8, 1, 0, None)
        .unwrap();
    assert_eq!(gray, chroma);
}
#[test]
fn malformed_options_and_storage_refuse_before_mutation() {
    let filter = Sab::parse("enable=0").unwrap();
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
        "lr=4.1",
        "lpfr=2.1",
        "ls=0",
        "cs=-1",
        "ls=101",
        "enable='nope'",
        "lr=NAN",
        "xx=1",
    ] {
        assert!(Sab::parse(options).is_err(), "{options}");
    }
    let mut invalid = [255u8; 2];
    assert!(
        Sab::parse("")
            .unwrap()
            .apply_plane(&mut invalid, 1, 1, 10, 0, 0, None)
            .is_err()
    );
    assert_eq!(invalid, [255; 2]);
    let mut rgb = GeometryFrame {
        width: 1,
        height: 1,
        subsampling: None,
        data: vec![1, 2, 3],
    };
    assert!(
        Sab::parse("")
            .unwrap()
            .apply(&mut rgb, 8, 0, None)
            .unwrap_err()
            .contains("planar YUV")
    );
    assert_eq!(rgb.data, [1, 2, 3]);
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
            sab: Some("ls=10:cs=30".into()),
            ..Default::default()
        };
        let stats = fvid::media::decode_video_transformed(&fixture(source), request).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
    let source = fixture("sab-guidance-edges.y4m");
    let output = std::env::temp_dir().join(format!("fvid-sab-cli-{}.y4m", std::process::id()));
    for args in [
        vec![
            "media",
            "decode",
            source.to_str().unwrap(),
            "--sab",
            "",
            "--quiet",
        ],
        vec![
            "media",
            "export-y4m",
            source.to_str().unwrap(),
            output.to_str().unwrap(),
            "--sab",
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
    let expected = std::fs::read(fixture("sab-blur.expected.raw")).unwrap();
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
    let output = std::env::temp_dir().join(format!("fvid-sab-cli-{}.mkv", std::process::id()));
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "transcode-lossless",
            source.to_str().unwrap(),
            output.to_str().unwrap(),
            "--sab",
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
