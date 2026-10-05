use fvid_media::{
    owned_frame::GeometryFrame,
    owned_vignette::{Clock, Vignette},
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
fn data(n: usize) -> Vec<u8> {
    (0..288)
        .map(|i| ((i * 37 + n * 23 + 101) % 256) as u8)
        .collect()
}
#[test]
fn saved_pixels_persistent_dither_and_rewind() {
    let expected = std::fs::read(fixture("vignette-dither.expected.raw")).unwrap();
    let rgb = std::fs::read(fixture("vignette-fractional.rgb24")).unwrap();
    let golden = std::fs::read(fixture("vignette-fractional.expected.raw")).unwrap();
    let shading = Vignette::parse("angle=PI/3:x0=3.7:y0=5.2").unwrap();
    let mut result = Vec::new();
    for (n, data) in rgb.as_chunks::<576>().0.iter().enumerate() {
        let mut frame = GeometryFrame {
            width: 16,
            height: 12,
            subsampling: None,
            data: data.to_vec(),
        };
        shading
            .apply(&mut frame, 8, n as u64, Some(n as f64 / 25.))
            .unwrap();
        result.extend(frame.data);
    }
    assert_eq!(result[322], 72, "angle precision reproducer");
    assert_eq!(result, golden);
    let filter = Vignette::parse("").unwrap();
    for _ in 0..2 {
        let mut output = Vec::new();
        for n in 0..4 {
            let mut frame = GeometryFrame {
                width: 16,
                height: 12,
                subsampling: Some([2, 2]),
                data: data(n),
            };
            filter
                .apply(&mut frame, 8, n as u64, Some(n as f64 / 25.))
                .unwrap();
            output.extend(frame.data);
        }
        assert_eq!(output, expected);
    }
    let independent: Vec<u8> = (0..4)
        .flat_map(|n| {
            let mut frame = GeometryFrame {
                width: 16,
                height: 12,
                subsampling: Some([2, 2]),
                data: data(n),
            };
            Vignette::parse("")
                .unwrap()
                .apply(&mut frame, 8, n as u64, Some(n as f64 / 25.))
                .unwrap();
            frame.data
        })
        .collect();
    assert_ne!(
        independent, expected,
        "fixture must exercise RNG persistence, not just masking"
    );
}
#[test]
fn both_export_apis_and_cli_accept_shading_without_legacy() {
    use fvid::{
        media::{CopyOptions, LosslessTransform},
        playback_native::NativeReader,
    };
    let expected = std::fs::read(fixture("vignette-dither.expected.raw")).unwrap();
    let source = fixture("vignette-dither.y4m");
    for library in [false, true] {
        let output = std::env::temp_dir().join(format!(
            "fvid-vignette-{}-{library}.mkv",
            std::process::id()
        ));
        let transform = LosslessTransform {
            vignette: Some("".into()),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::transcode_lossless(&source, &output, transform, &CopyOptions::default())
        } else {
            fvid::media::transcode_lossless(&source, &output, transform, &CopyOptions::default())
        }
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.video_frames, 4);
        let mut reader =
            NativeReader::software(Cursor::new(std::fs::read(&output).unwrap()), usize::MAX)
                .unwrap();
        let mut pixels = Vec::new();
        while let Some(frame) = reader.read_frame_raw().unwrap() {
            pixels.extend(
                fvid::native_geometry::VideoGeometry::default()
                    .apply(&frame, 16, 12)
                    .unwrap()
                    .data,
            );
        }
        assert_eq!(pixels, expected);
        std::fs::remove_file(output).unwrap();
    }
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "decode",
            source.to_str().unwrap(),
            "--vignette",
            "angle=n/10:eval=frame",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let cli_output =
        std::env::temp_dir().join(format!("fvid-vignette-cli-{}.mkv", std::process::id()));
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "transcode-lossless",
            source.to_str().unwrap(),
            cli_output.to_str().unwrap(),
            "--vignette",
            "",
            "--framestep",
            "2",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut reader =
        NativeReader::software(Cursor::new(std::fs::read(&cli_output).unwrap()), usize::MAX)
            .unwrap();
    for n in [0, 2] {
        let frame = reader.read_frame_raw().unwrap().unwrap();
        let samples = fvid::native_geometry::VideoGeometry::default()
            .apply(&frame, 16, 12)
            .unwrap();
        assert_eq!(samples.data, &expected[n * 288..(n + 1) * 288]);
    }
    assert!(reader.read_frame_raw().unwrap().is_none());
    std::fs::remove_file(cli_output).unwrap();
    let base = std::env::temp_dir().join(format!(
        "fvid-vignette-clock-base-{}.mkv",
        std::process::id()
    ));
    fvid_media::transcode_lossless(
        &source,
        &base,
        LosslessTransform::default(),
        &CopyOptions::default(),
    )
    .unwrap();
    // Matroska's 1 ms clock differs from Y4M's 1/25 clock. pts*tb
    // must keep the same seconds. This export has no declared nominal rate.
    let clocked = fvid_media::decode_video_transformed(
        &base,
        fvid::media::DecodeTransform {
            vignette: Some("angle=pts*tb+n/10:eval=frame".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(clocked.video_frames, 4);
    let source_metadata = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(&base).unwrap()),
        Default::default(),
    )
    .unwrap();
    assert_eq!(
        source_metadata
            .tracks
            .iter()
            .find(|t| t.kind == 1)
            .unwrap()
            .default_duration_ns,
        0
    );
    let refused = fvid_media::decode_video_transformed(
        &base,
        fvid::media::DecodeTransform {
            vignette: Some("angle=r:eval=frame".into()),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(refused.contains("finite source metadata"), "{refused}");
    let accepted = fvid_media::decode_video_transformed(
        &source,
        fvid::media::DecodeTransform {
            vignette: Some("angle=r*tb:eval=frame".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(accepted.video_frames, 4);

    std::fs::remove_file(base).unwrap();
    let stats = fvid_media::decode_video_transformed(
        &source,
        fvid::media::DecodeTransform {
            vignette: Some("mode=backward:dither=0".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.video_frames, 4);
}
#[test]
fn dimensions_clock_enable_and_extended_precision() {
    let filter = Vignette::parse("angle=n/10").unwrap();
    let explicit = Vignette::parse("angle=n/10:eval=frame").unwrap();
    for n in 0..3 {
        let mut a = GeometryFrame {
            width: 16,
            height: 12,
            subsampling: Some([2, 2]),
            data: data(n),
        };
        let mut b = GeometryFrame {
            width: 16,
            height: 12,
            subsampling: Some([2, 2]),
            data: data(n),
        };
        filter.apply(&mut a, 8, n as u64, None).unwrap();
        explicit.apply(&mut b, 8, n as u64, None).unwrap();
        assert_eq!(a.data, b.data);
    }
    for depth in [8, 9, 10, 12, 14, 16] {
        let maximum = (1u32 << depth) - 1;
        let bytes = if depth == 8 {
            vec![maximum as u8; 27]
        } else {
            (0..27)
                .flat_map(|_| (maximum as u16).to_le_bytes())
                .collect()
        };
        let mut frame = GeometryFrame {
            width: 5,
            height: 3,
            subsampling: Some([2, 2]),
            data: bytes,
        };
        let original = frame.data.clone();
        Vignette::parse("angle=0:dither=0")
            .unwrap()
            .apply(&mut frame, depth, 0, None)
            .unwrap();
        assert_eq!(frame.data, original);
        Vignette::parse("mode=backward")
            .unwrap()
            .apply(&mut frame, depth, 0, None)
            .unwrap();
        assert_eq!(frame.data, original);
    }
    let filter = Vignette::parse("angle=r*tb/2:eval=frame:x0=pts:y0=t").unwrap();
    let mut frame = GeometryFrame {
        width: 16,
        height: 12,
        subsampling: Some([2, 2]),
        data: data(0),
    };
    let before = frame.data.clone();
    assert!(filter.apply(&mut frame, 8, 0, None).is_err());
    assert_eq!(frame.data, before);
    filter
        .apply_clock(
            &mut frame,
            8,
            Clock {
                n: 0,
                t: Some(0.),
                pts: Some(0.),
                rate: Some(25.),
                time_base: Some(0.04),
                sample_aspect: 2.,
            },
        )
        .unwrap();
    let disabled = Vignette::parse("enable=0").unwrap();
    let mut frame = GeometryFrame {
        width: 16,
        height: 12,
        subsampling: Some([2, 2]),
        data: data(0),
    };
    let before = frame.data.clone();
    disabled.apply(&mut frame, 8, 0, None).unwrap();
    assert_eq!(frame.data, before);
    frame.data.pop();
    assert!(disabled.apply(&mut frame, 8, 0, None).is_err());
    for args in [
        "mode=no",
        "eval=no",
        "angle=unknown",
        "aspect=0",
        "aspect=NaN",
        "dither=2",
        "no=1",
        "x0='1",
    ] {
        assert!(Vignette::parse(args).is_err(), "{args}");
    }
}
