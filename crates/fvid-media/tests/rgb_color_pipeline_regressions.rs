use fvid_media::owned_webm::{Limits, WebmReader};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/rgb-color-pipeline")
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&fs::read(root().join("manifest.json")).unwrap())
        .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Frame {
    color: Vec<u8>,
    alpha: Option<Vec<u8>>,
    depth: u8,
    width: usize,
    height: usize,
    subsampling: Option<[usize; 2]>,
    pts: i64,
    duration: u64,
}
fn decode(path: &Path) -> Vec<Frame> {
    let mut reader =
        WebmReader::open(Cursor::new(fs::read(path).unwrap()), Limits::default()).unwrap();
    reader.scan_all().unwrap();
    let track = reader.tracks[0].clone();
    let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
        track.width as usize,
        track.height as usize,
        &track.codec_private,
        32 << 20,
    )
    .unwrap();
    (0..reader.packets.len())
        .map(|i| {
            let p = reader.packets[i].clone();
            let decoded = decoder.decode(&reader.read_packet(i).unwrap()).unwrap();
            Frame {
                color: decoded.frame.data,
                alpha: decoded.alpha,
                depth: decoded.depth,
                width: decoded.frame.width,
                height: decoded.frame.height,
                subsampling: decoded.frame.subsampling,
                pts: p.pts_ns,
                duration: p.duration_ns.unwrap_or(track.default_duration_ns),
            }
        })
        .collect()
}
#[test]
fn all_authored_controls_decode_exact_color_alpha_and_clocks() {
    for case in cases() {
        let frames = decode(&root().join(case["file"].as_str().unwrap()));
        assert_eq!(frames.len(), 3);
        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(
                frame.color,
                fs::read(root().join(case["frames"][i]["color"].as_str().unwrap())).unwrap(),
                "{} frame {i}",
                case["file"]
            );
            let alpha = case["frames"][i]["alpha"]
                .as_str()
                .map(|name| fs::read(root().join(name)).unwrap());
            assert_eq!(frame.alpha, alpha);
            assert_eq!(frame.depth, case["depth"].as_u64().unwrap() as u8);
            assert_eq!((frame.width, frame.height), (4, 2));
            assert_eq!(
                frame.subsampling,
                if case["layout"] == "rgb" {
                    None
                } else {
                    Some([1, 1])
                }
            );
            assert_eq!(
                (frame.pts, frame.duration),
                (i as i64 * 20_000_000, 20_000_000)
            );
        }
    }
}
#[test]
fn unsupported_rgb_colorhold_precision_is_a_refusal_not_decoder_failure() {
    let source = root().join("rgb-10.mkv");
    assert_eq!(decode(&source).len(), 3);
    let output =
        std::env::temp_dir().join(format!("fvid-rgb-color-before-{}.mkv", std::process::id()));
    let error = fvid_media::transcode_lossless(
        &source,
        &output,
        fvid_media::LosslessTransform {
            colorhold: Some("color=red".into()),
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap_err();
    assert!(!error.to_string().is_empty());
    assert!(!output.exists());
    eprintln!("declared native RGB refusal: {error}");
}
#[test]
fn native_rgb_exposure_acceptance_preserves_alpha_and_has_known_doubled_pixels() {
    for case in cases().into_iter().filter(|c| c["layout"] == "rgb") {
        let source = root().join(case["file"].as_str().unwrap());
        let mut expected = decode(&source);
        for frame in &mut expected {
            let max = (1u32 << frame.depth) - 1;
            if frame.depth == 8 {
                for v in &mut frame.color {
                    *v = (u32::from(*v) * 2).min(max) as u8;
                }
            } else {
                for v in frame.color.chunks_exact_mut(2) {
                    let value = (u32::from(u16::from_le_bytes([v[0], v[1]])) * 2).min(max) as u16;
                    v.copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        let output = std::env::temp_dir().join(format!(
            "fvid-rgb-exposure-{}-{}-{}.mkv",
            std::process::id(),
            case["depth"],
            case["alpha"]
        ));
        fvid_media::transcode_lossless(
            &source,
            &output,
            fvid_media::LosslessTransform {
                exposure: Some("1".into()),
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        assert_eq!(decode(&output), expected);
        fs::remove_file(output).unwrap();
    }
}
fn flipped(data: &[u8], width: usize, channels: usize, bytes: usize) -> Vec<u8> {
    data.chunks_exact(width * channels * bytes)
        .flat_map(|row| {
            row.chunks_exact(channels * bytes)
                .rev()
                .flat_map(|pixel| pixel.iter().copied())
        })
        .collect()
}
#[test]
fn existing_rgb_geometry_export_is_an_end_to_end_codec_control() {
    for case in cases().into_iter().filter(|c| c["layout"] == "rgb") {
        let source = root().join(case["file"].as_str().unwrap());
        let mut expected = decode(&source);
        for frame in &mut expected {
            let bytes = if frame.depth == 8 { 1 } else { 2 };
            frame.color = flipped(&frame.color, frame.width, 3, bytes);
            frame.alpha = frame
                .alpha
                .as_ref()
                .map(|a| flipped(a, frame.width, 1, bytes));
        }
        let output = std::env::temp_dir().join(format!(
            "fvid-rgb-geometry-control-{}-{}-{}.mkv",
            std::process::id(),
            case["depth"],
            case["alpha"]
        ));
        fvid_media::transcode_lossless(
            &source,
            &output,
            fvid_media::LosslessTransform {
                horizontal_flip: true,
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        assert_eq!(decode(&output), expected);
        fs::remove_file(output).unwrap();
    }
}
#[test]
fn native_yuva_exposure_acceptance_keeps_opacity_untouched() {
    for case in cases().into_iter().filter(|c| c["layout"] == "yuv444") {
        let source = root().join(case["file"].as_str().unwrap());
        let mut expected = decode(&source);
        for frame in &mut expected {
            let mut geometry = fvid_media::owned_frame::GeometryFrame {
                width: frame.width,
                height: frame.height,
                subsampling: frame.subsampling,
                data: frame.color.clone(),
            };
            fvid_media::owned_exposure::Exposure::parse("1")
                .unwrap()
                .apply_yuv(
                    &mut geometry,
                    frame.depth,
                    false,
                    fvid_media::owned_yuv_rgb::Matrix::Bt601,
                )
                .unwrap();
            frame.color = geometry.data;
        }
        let output = std::env::temp_dir().join(format!(
            "fvid-yuva-exposure-{}-{}.mkv",
            std::process::id(),
            case["depth"]
        ));
        fvid_media::transcode_lossless(
            &source,
            &output,
            fvid_media::LosslessTransform {
                exposure: Some("1".into()),
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        assert_eq!(decode(&output), expected);
        fs::remove_file(output).unwrap();
    }
}

#[test]
fn rgb_color_operators_and_order_match_qualified_kernels() {
    for case in cases().into_iter().filter(|c| c["layout"] == "rgb") {
        for mode in 0..6 {
            let source = root().join(case["file"].as_str().unwrap());
            let mut expected = decode(&source);
            let hold = matches!(expected[0].depth, 8 | 16);
            if mode == 4 && !hold {
                continue;
            }
            let mut transform = fvid_media::LosslessTransform::default();
            if mode == 0 || mode == 5 {
                transform.grayworld = Some("enable='eq(n,1)'".into());
            }
            if mode == 1 || mode == 5 {
                transform.vibrance = Some("intensity=0.5".into());
            }
            if mode == 2 || mode == 5 {
                transform.exposure = Some("0.5".into());
            }
            if mode == 3 || mode == 5 {
                transform.colorcontrast = Some("rc=0.5:gm=-0.2:by=0.1".into());
            }
            if mode == 4 || (mode == 5 && hold) {
                transform.colorhold = Some("red:0.2:0.5".into());
            }
            for (n, frame) in expected.iter_mut().enumerate() {
                let d = &mut frame.color;
                if transform.grayworld.is_some() && n == 1 {
                    fvid_media::owned_grayworld::GrayWorld::default()
                        .apply_rgb(d, frame.width, frame.height, frame.depth, 3)
                        .unwrap();
                }
                if let Some(a) = &transform.vibrance {
                    fvid_media::owned_vibrance::Vibrance::parse(a)
                        .unwrap()
                        .apply_rgb(d, frame.depth, 3)
                        .unwrap();
                }
                if let Some(a) = &transform.exposure {
                    fvid_media::owned_exposure::Exposure::parse(a)
                        .unwrap()
                        .apply_rgb(d, frame.depth, 3)
                        .unwrap();
                }
                if let Some(a) = &transform.colorcontrast {
                    fvid_media::owned_colorcontrast::ColorContrast::parse(a)
                        .unwrap()
                        .apply_rgb(d, frame.depth, 3)
                        .unwrap();
                }
                if let Some(a) = &transform.colorhold {
                    fvid_media::owned_colorhold::ColorHold::parse(a)
                        .unwrap()
                        .apply_rgb(d, frame.depth, 3)
                        .unwrap();
                }
            }
            let output = std::env::temp_dir().join(format!(
                "fvid-rgb-operators-{}-{}-{}-{mode}.mkv",
                std::process::id(),
                case["depth"],
                case["alpha"]
            ));
            fvid_media::transcode_lossless(&source, &output, transform, &Default::default())
                .unwrap();
            assert_eq!(decode(&output), expected, "case {case}, mode {mode}");
            fs::remove_file(output).unwrap();
        }
    }
}

#[test]
fn neutral_rgb_exposure_is_bit_exact_at_every_precision() {
    for case in cases().into_iter().filter(|c| c["layout"] == "rgb") {
        let source = root().join(case["file"].as_str().unwrap());
        let output = std::env::temp_dir().join(format!(
            "fvid-rgb-neutral-{}-{}-{}.mkv",
            std::process::id(),
            case["depth"],
            case["alpha"]
        ));
        fvid_media::transcode_lossless(
            &source,
            &output,
            fvid_media::LosslessTransform {
                exposure: Some("0".into()),
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        assert_eq!(decode(&output), decode(&source));
        fs::remove_file(output).unwrap();
    }
}

#[test]
fn invalid_color_arguments_do_not_publish_output() {
    for (index, transform) in [
        fvid_media::LosslessTransform {
            grayworld: Some("enable='unknown_variable'".into()),
            ..Default::default()
        },
        fvid_media::LosslessTransform {
            vibrance: Some("intensity=nan".into()),
            ..Default::default()
        },
        fvid_media::LosslessTransform {
            exposure: Some("not-a-number".into()),
            ..Default::default()
        },
        fvid_media::LosslessTransform {
            colorcontrast: Some("unknown=1".into()),
            ..Default::default()
        },
        fvid_media::LosslessTransform {
            colorhold: Some("not-a-color".into()),
            ..Default::default()
        },
    ]
    .into_iter()
    .enumerate()
    {
        let output = std::env::temp_dir().join(format!(
            "fvid-rgb-invalid-{}-{index}.mkv",
            std::process::id()
        ));
        assert!(
            fvid_media::transcode_lossless(
                &root().join("rgb-8.mkv"),
                &output,
                transform,
                &Default::default()
            )
            .is_err()
        );
        assert!(!output.exists());
    }
}

#[test]
fn native_cas_sharpens_rgb_and_selected_opacity_planes() {
    for case in cases().into_iter().filter(|c| c["layout"] == "rgb") {
        for mask in [0, 1, 2, 4, 7, 8, 15] {
            let source = root().join(case["file"].as_str().unwrap());
            let original = decode(&source);
            let mut expected = original.clone();
            let args = format!("strength=0.8:planes={mask}");
            for frame in &mut expected {
                let filter = fvid_media::owned_cas::Cas::parse(&args).unwrap();
                if let Some(alpha) = &frame.alpha {
                    let bytes = if frame.depth == 8 { 1 } else { 2 };
                    let mut packed = Vec::new();
                    for (rgb, a) in frame
                        .color
                        .chunks_exact(3 * bytes)
                        .zip(alpha.chunks_exact(bytes))
                    {
                        packed.extend_from_slice(rgb);
                        packed.extend_from_slice(a);
                    }
                    filter
                        .apply_rgb(&mut packed, frame.width, frame.height, frame.depth, 4)
                        .unwrap();
                    frame.color.clear();
                    let mut opacity = Vec::new();
                    for pixel in packed.chunks_exact(4 * bytes) {
                        frame.color.extend_from_slice(&pixel[..3 * bytes]);
                        opacity.extend_from_slice(&pixel[3 * bytes..]);
                    }
                    frame.alpha = Some(opacity);
                } else {
                    filter
                        .apply_rgb(&mut frame.color, frame.width, frame.height, frame.depth, 3)
                        .unwrap();
                }
            }
            if mask == 0 {
                assert_eq!(expected, original);
            }
            if mask == 8
                && case["alpha"] == true
                && case["file"].as_str().unwrap().contains("smooth")
            {
                assert_ne!(expected, original, "selected opacity must change");
            }
            if mask == 7 {
                assert_ne!(expected, original, "sharpening witness must change pixels");
            }
            let output = std::env::temp_dir().join(format!(
                "fvid-cas-{}-{}-{}-{mask}.mkv",
                std::process::id(),
                case["depth"],
                case["alpha"]
            ));
            fvid_media::transcode_lossless(
                &source,
                &output,
                fvid_media::LosslessTransform {
                    cas: Some(args),
                    ..Default::default()
                },
                &Default::default(),
            )
            .unwrap();
            assert_eq!(decode(&output), expected, "{case}, mask {mask}");
            fs::remove_file(output).unwrap();
        }
    }
}

#[test]
fn cas_yuva_mask_filters_selected_opacity_without_changing_color() {
    for case in cases().into_iter().filter(|c| c["layout"] == "yuv444") {
        let source = root().join(case["file"].as_str().unwrap());
        let mut expected = decode(&source);
        for frame in &mut expected {
            let bytes = if frame.depth == 8 { 1 } else { 2 };
            let mut packed = Vec::new();
            for a in frame.alpha.as_ref().unwrap().chunks_exact(bytes) {
                packed.extend_from_slice(&vec![0; 3 * bytes]);
                packed.extend_from_slice(a);
            }
            fvid_media::owned_cas::Cas::parse("strength=0.8:planes=8")
                .unwrap()
                .apply_rgb(&mut packed, frame.width, frame.height, frame.depth, 4)
                .unwrap();
            frame.alpha = Some(
                packed
                    .chunks_exact(4 * bytes)
                    .flat_map(|p| p[3 * bytes..].iter().copied())
                    .collect(),
            );
        }
        let output = std::env::temp_dir().join(format!(
            "fvid-cas-yuva-{}-{}.mkv",
            std::process::id(),
            case["depth"]
        ));
        fvid_media::transcode_lossless(
            &source,
            &output,
            fvid_media::LosslessTransform {
                cas: Some("strength=0.8:planes=8".into()),
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        assert_eq!(decode(&output), expected);
        fs::remove_file(output).unwrap();
    }
}

#[test]
fn cas_invalid_options_and_resource_limits_do_not_publish_partial_output() {
    for (index, args, options) in [
        (0, "strength=2", fvid_control::CopyOptions::default()),
        (1, "strength=nan", fvid_control::CopyOptions::default()),
        (2, "planes=16", fvid_control::CopyOptions::default()),
        (
            3,
            "strength=0.8",
            fvid_control::CopyOptions {
                max_packets: Some(1),
                ..Default::default()
            },
        ),
        (
            4,
            "strength=0.8",
            fvid_control::CopyOptions {
                max_packet_bytes: 1,
                ..Default::default()
            },
        ),
        (
            5,
            "strength=0.8",
            fvid_control::CopyOptions {
                max_controlled_bytes: Some(1),
                ..Default::default()
            },
        ),
    ] {
        let output = std::env::temp_dir().join(format!(
            "fvid-cas-limits-{}-{index}.mkv",
            std::process::id()
        ));
        assert!(
            fvid_media::transcode_lossless(
                &root().join("rgb-8-alpha-smooth.mkv"),
                &output,
                fvid_media::LosslessTransform {
                    cas: Some(args.into()),
                    ..Default::default()
                },
                &options
            )
            .is_err()
        );
        assert!(!output.exists());
    }
}
