use fvid_media::{
    owned_draw::{Draw, Kind},
    owned_frame::GeometryFrame,
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
const ARGS: &str = "x=1:y=2:w=9:h=7:c=red@.4:t=2";
const ASPECT_ARGS: &str = "x=sar*3:y=dar:w=iw/2:h=ih/2:c=red@.4:t=2";
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
fn frame(n: usize) -> GeometryFrame {
    GeometryFrame {
        width: 17,
        height: 13,
        subsampling: Some([2, 2]),
        data: (0..347)
            .map(|i| ((i * 37 + n * 23 + 101) % 256) as u8)
            .collect(),
    }
}
fn expected(name: &str, aspect: bool) -> Vec<u8> {
    std::fs::read(fixture(&format!(
        "{name}{}.expected.raw",
        if aspect { "-aspect" } else { "" }
    )))
    .unwrap()
}
fn pixels(path: &Path) -> Vec<u8> {
    let mut reader = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(path).unwrap()),
        usize::MAX,
    )
    .unwrap();
    let mut data = Vec::new();
    while let Some(raw) = reader.read_frame_raw().unwrap() {
        let [w, h] = reader.dimensions();
        data.extend(
            fvid::native_geometry::VideoGeometry::default()
                .apply(&raw, w, h)
                .unwrap()
                .data,
        );
    }
    data
}
#[test]
fn boxes_grids_subsampled_blends_aspect_and_rewind_match_independent_pixels() {
    for (kind, name) in [(Kind::Box, "drawbox"), (Kind::Grid, "drawgrid")] {
        for aspect in [false, true] {
            let filter = Draw::parse(kind, if aspect { ASPECT_ARGS } else { ARGS }).unwrap();
            for _ in 0..2 {
                let mut out = Vec::new();
                for n in 0..4 {
                    let mut f = frame(n);
                    filter
                        .apply_with_aspect(
                            &mut f,
                            8,
                            if aspect { 4. / 3. } else { 1. },
                            n as u64,
                            Some(n as f64 / 25.),
                        )
                        .unwrap();
                    out.extend(f.data);
                }
                assert_eq!(out, expected(name, aspect));
            }
            assert_ne!(
                expected(name, aspect),
                (0..4).flat_map(|n| frame(n).data).collect::<Vec<_>>()
            );
        }
    }
}
#[test]
fn explicit_alpha_replacement_inversion_and_high_precision_are_owned() {
    for kind in [Kind::Box, Kind::Grid] {
        let filter = Draw::parse(kind, "c=red@.5:t=fill:replace=1").unwrap();
        let mut rgba = vec![7; 4 * 3 * 3];
        filter.apply_rgba(&mut rgba, 3, 3, 0, None).unwrap();
        assert_eq!(rgba, [255, 0, 0, 127].repeat(9));
        let filter = Draw::parse(kind, "c=invert:t=fill:replace=1").unwrap();
        filter.apply_rgba(&mut rgba, 3, 3, 0, None).unwrap();
        assert_eq!(rgba, [0, 255, 255, 127].repeat(9));
        let mut yuva = vec![10; 4 * 3 * 3];
        Draw::parse(kind, "c=red@.5:t=fill:replace=1")
            .unwrap()
            .apply_yuva(&mut yuva, 3, 3, [1, 1], 8, 1., 0, None)
            .unwrap();
        assert_eq!(
            yuva,
            [vec![81; 9], vec![90; 9], vec![240; 9], vec![127; 9]].concat()
        );
        Draw::parse(kind, "c=invert:t=fill")
            .unwrap()
            .apply_yuva(&mut yuva, 3, 3, [1, 1], 8, 1., 0, None)
            .unwrap();
        assert_eq!(&yuva[..9], &[174; 9]);
        assert_eq!(&yuva[27..], &[127; 9]);
        for depth in 8..=16 {
            for channels in [3, 4] {
                let mut data = vec![0; 9 * channels * if depth == 8 { 1 } else { 2 }];
                Draw::parse(kind, "c=white:t=fill:replace=1")
                    .unwrap()
                    .apply_rgb(&mut data, 3, 3, depth, channels, 1., 0, None)
                    .unwrap();
                let want = ((1u32 << depth) - 1) as u16;
                if depth == 8 {
                    assert_eq!(data, vec![255; 9 * channels]);
                } else {
                    assert_eq!(data, want.to_le_bytes().repeat(9 * channels));
                }
            }
        }
    }
}
#[test]
fn invalid_parameters_geometry_and_nonfinite_coordinates_refuse_before_mutation() {
    for args in [
        "x=unknown",
        "c=not_a_color",
        "replace=2",
        "box_source=side_data_detection_bboxes",
        "t=invalid(1)",
        "x=if(1,2,unknown)",
        "1:2:3:4:5:6:7:8",
    ] {
        assert!(Draw::box_filter(args).is_err(), "{args}");
    }
    for args in ["x=1/0", "x=2147483648", "w=w", "y=NAN"] {
        let filter = Draw::box_filter(args).unwrap();
        let mut f = frame(0);
        let old = f.data.clone();
        assert!(filter.apply(&mut f, 8, 0, None).is_err());
        assert_eq!(f.data, old);
    }
    let filter = Draw::box_filter("").unwrap();
    let mut bad = vec![1; 3];
    assert!(
        filter
            .apply_yuva(&mut bad, 1, 1, [2, 2], 8, 1., 0, None)
            .is_err()
    );
    assert_eq!(bad, [1; 3]);
    assert!(filter.apply_rgba(&mut bad, usize::MAX, 2, 0, None).is_err());
    assert_eq!(bad, [1; 3]);
}
#[test]
fn both_exports_keep_non_square_aspect_and_qualified_pixels() {
    for (name, grid) in [("drawbox", false), ("drawgrid", true)] {
        for aspect in [false, true] {
            for library in [false, true] {
                let source = fixture(if aspect {
                    "drawing-aspect.y4m"
                } else {
                    "drawing.y4m"
                });
                let out = std::env::temp_dir().join(format!(
                    "fvid-draw-{}-{name}-{aspect}-{library}.mkv",
                    std::process::id()
                ));
                let mut request = fvid::media::LosslessTransform::default();
                if grid {
                    request.drawgrid = Some(if aspect { ASPECT_ARGS } else { ARGS }.into());
                } else {
                    request.drawbox = Some(if aspect { ASPECT_ARGS } else { ARGS }.into());
                }
                let stats = if library {
                    fvid_media::transcode_lossless(&source, &out, request, &Default::default())
                } else {
                    fvid::media::transcode_lossless(&source, &out, request, &Default::default())
                }
                .unwrap();
                assert_eq!(stats.backend, "fvid");
                assert_eq!(stats.video_frames, 4);
                assert_eq!(pixels(&out), expected(name, aspect));
                let reader = fvid::playback_native::NativeReader::software(
                    Cursor::new(std::fs::read(&out).unwrap()),
                    usize::MAX,
                )
                .unwrap();
                assert_eq!(reader.pixel_aspect(), if aspect { (4, 3) } else { (1, 1) });
                std::fs::remove_file(out).unwrap();
            }
        }
    }
}
#[test]
fn cli_decode_export_and_lossless_use_the_owned_drawing_pipeline() {
    for (name, option) in [("drawbox", "--drawbox"), ("drawgrid", "--drawgrid")] {
        for operation in ["decode", "export-y4m", "transcode-lossless"] {
            let out = std::env::temp_dir().join(format!(
                "fvid-draw-cli-{}-{name}.{extension}",
                std::process::id(),
                extension = if operation == "export-y4m" {
                    "y4m"
                } else {
                    "mkv"
                }
            ));
            let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"));
            command
                .args(["media", operation])
                .arg(fixture("drawing.y4m"));
            if operation != "decode" {
                command.arg(&out);
            }
            let result = command.args([option, ARGS]).output().unwrap();
            assert!(
                result.status.success(),
                "{operation}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            if operation == "decode" {
                let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
                assert_eq!(stats["backend"], "fvid");
            } else {
                assert_eq!(pixels(&out), expected(name, false));
                std::fs::remove_file(out).unwrap();
            }
        }
    }
}
#[test]
fn timeline_selection_small_frames_and_native_codecs_keep_owned_routing() {
    for grid in [false, true] {
        let name = if grid { "drawgrid" } else { "drawbox" };
        for library in [false, true] {
            let out = std::env::temp_dir().join(format!(
                "fvid-draw-time-{}-{grid}-{library}.mkv",
                std::process::id()
            ));
            let mut request = fvid::media::LosslessTransform {
                framestep: Some("2".into()),
                ..Default::default()
            };
            let args = format!("{ARGS}:enable='eq(n,2)*gte(t,0.08)'");
            if grid {
                request.drawgrid = Some(args);
            } else {
                request.drawbox = Some(args);
            }
            let stats = if library {
                fvid_media::transcode_lossless(
                    &fixture("drawing.y4m"),
                    &out,
                    request,
                    &Default::default(),
                )
            } else {
                fvid::media::transcode_lossless(
                    &fixture("drawing.y4m"),
                    &out,
                    request,
                    &Default::default(),
                )
            }
            .unwrap();
            assert_eq!(stats.video_frames, 2);
            let mut want = frame(0).data;
            want.extend_from_slice(&expected(name, false)[2 * 347..3 * 347]);
            assert_eq!(pixels(&out), want);
            std::fs::remove_file(out).unwrap();
        }
        for source in [
            "drawing-small.y4m",
            "../video.mp4",
            "../hevc/main10-ipb.mp4",
            "../vp9/adaptive.webm",
            "../av1/ramp.webm",
        ] {
            let mut request = fvid::media::DecodeTransform::default();
            if grid {
                request.drawgrid = Some(ARGS.into());
            } else {
                request.drawbox = Some(ARGS.into());
            }
            let stats = fvid::media::decode_video_transformed(&fixture(source), request).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert!(stats.video_frames > 0);
        }
    }
}

#[test]
fn box_then_grid_order_is_preserved_through_both_exports() {
    let expected = std::fs::read(fixture("drawing-combined.expected.raw")).unwrap();
    for library in [false, true] {
        let out = std::env::temp_dir().join(format!(
            "fvid-draw-combined-{}-{library}.mkv",
            std::process::id()
        ));
        let request = fvid::media::LosslessTransform {
            drawbox: Some(ARGS.into()),
            drawgrid: Some("w=5:h=4:c=blue@.3:t=1".into()),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::transcode_lossless(
                &fixture("drawing.y4m"),
                &out,
                request,
                &Default::default(),
            )
        } else {
            fvid::media::transcode_lossless(
                &fixture("drawing.y4m"),
                &out,
                request,
                &Default::default(),
            )
        }
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(pixels(&out), expected);
        std::fs::remove_file(out).unwrap();
    }
}
