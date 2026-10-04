use fvid_media::{owned_frame::GeometryFrame, owned_removegrain::RemoveGrain};
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
        width: 17,
        height: 13,
        subsampling: Some([2, 2]),
        data: (0..347)
            .map(|i| ((i * i * 13 + i * 37 + n * 23 + 101) % 256) as u8)
            .collect(),
    }
}
fn expected(mode: u8) -> Vec<u8> {
    std::fs::read(fixture(&format!("removegrain-{mode:02}.expected.raw"))).unwrap()
}
#[test]
fn every_mode_matches_independent_pixels_preserves_borders_and_rewinds() {
    for mode in 0..=24 {
        let args = format!("{mode}:{mode}:{mode}:{mode}");
        let filter = RemoveGrain::parse(&args).unwrap();
        let gold = expected(mode);
        for _ in 0..2 {
            let mut out = Vec::new();
            for n in 0..4 {
                let original = frame(n);
                let mut f = frame(n);
                filter
                    .apply(&mut f, 8, n as u64, Some(n as f64 / 25.))
                    .unwrap();
                let mut offset = 0;
                for (w, h) in [(17, 13), (9, 7), (9, 7)] {
                    for y in 0..h {
                        for x in 0..w {
                            if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                                assert_eq!(
                                    f.data[offset + y * w + x],
                                    original.data[offset + y * w + x]
                                );
                            }
                        }
                    }
                    offset += w * h;
                }
                out.extend(f.data);
            }
            assert_eq!(out, gold, "mode {mode}");
        }
    }
    assert_ne!(
        expected(19),
        expected(0),
        "fixture must exercise the filter"
    );
}
#[test]
fn high_precision_impulse_and_transactional_refusals() {
    for depth in 8..=16 {
        let c = ((1u32 << depth) - 1) as u16;
        let a = c / 10;
        for mode in 0..=24 {
            let mut values = [a; 9];
            values[4] = c;
            let mut data: Vec<_> = if depth == 8 {
                values.map(|v| v as u8).to_vec()
            } else {
                values.into_iter().flat_map(u16::to_le_bytes).collect()
            };
            let old = data.clone();
            let filter = RemoveGrain::parse(&format!("m0={mode}")).unwrap();
            filter
                .apply_plane(&mut data, 3, 3, depth, 0, 0, None)
                .unwrap();
            let actual = if depth == 8 {
                data[4] as u16
            } else {
                u16::from_le_bytes([data[8], data[9]])
            };
            let want = match mode {
                0 | 13 | 15 | 23 | 24 => c,
                11 | 12 => ((4 * c as u32 + 12 * a as u32 + 8) / 16) as u16,
                20 => ((c as u32 + 8 * a as u32 + 4) / 9) as u16,
                _ => a,
            };
            assert_eq!(actual, want, "depth {depth} mode {mode}");
            for i in 0..9 {
                if i != 4 {
                    let bytes = if depth == 8 { 1 } else { 2 };
                    assert_eq!(
                        &data[i * bytes..(i + 1) * bytes],
                        &old[i * bytes..(i + 1) * bytes]
                    );
                }
            }
        }
    }
    for args in [
        "m0=-1",
        "m0=25",
        "m4=1",
        "m0=1.5",
        "m0=NaN",
        "1:2:3:4:5",
        "enable='eq(unknown,1)'",
    ] {
        assert!(RemoveGrain::parse(args).is_err(), "{args}");
    }
    let filter = RemoveGrain::parse("m0=19").unwrap();
    let mut invalid = vec![255, 255];
    let old = invalid.clone();
    assert!(
        filter
            .apply_plane(&mut invalid, 1, 1, 10, 0, 0, None)
            .is_err()
    );
    assert_eq!(old, invalid);
    for (w, h, depth, p) in [
        (0, 1, 8, 0),
        (1, 1, 7, 0),
        (1, 1, 8, 4),
        (usize::MAX, 2, 8, 0),
    ] {
        let mut data = vec![1];
        assert!(
            filter
                .apply_plane(&mut data, w, h, depth, p, 0, None)
                .is_err()
        );
        assert_eq!(data, [1]);
    }
}
#[test]
fn parity_is_shared_by_present_planes_and_small_frames_are_safe() {
    let mut both = frame(0);
    RemoveGrain::parse("m0=13:m1=14:m2=19")
        .unwrap()
        .apply(&mut both, 8, 0, None)
        .unwrap();
    assert_eq!(both.data, frame(0).data);
    let mut a = frame(0);
    let mut b = frame(0);
    RemoveGrain::parse("m0=19:m3=13")
        .unwrap()
        .apply(&mut a, 8, 0, None)
        .unwrap();
    RemoveGrain::parse("m0=19")
        .unwrap()
        .apply(&mut b, 8, 0, None)
        .unwrap();
    assert_eq!(a.data, b.data, "absent alpha must not select parity");
    for mode in 0..=24 {
        for (w, h) in [(1, 1), (2, 8), (8, 2)] {
            let mut data = vec![77; w * h];
            RemoveGrain::parse(&format!("m0={mode}"))
                .unwrap()
                .apply_plane(&mut data, w, h, 8, 0, 0, None)
                .unwrap();
            assert_eq!(data, vec![77; w * h]);
        }
    }
    for library in [false, true] {
        let request = fvid::media::DecodeTransform {
            removegrain: Some("m0=24:m1=24:m2=24".into()),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::decode_video_transformed(&fixture("removegrain-small.y4m"), request)
        } else {
            fvid::media::decode_video_transformed(&fixture("removegrain-small.y4m"), request)
        }
        .unwrap();
        assert_eq!(stats.video_frames, 4);
    }
}
fn read_pixels(path: &Path) -> Vec<u8> {
    let mut reader = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(path).unwrap()),
        usize::MAX,
    )
    .unwrap();
    let mut pixels = Vec::new();
    while let Some(raw) = reader.read_frame_raw().unwrap() {
        let [w, h] = reader.dimensions();
        pixels.extend(
            fvid::native_geometry::VideoGeometry::default()
                .apply(&raw, w, h)
                .unwrap()
                .data,
        );
    }
    pixels
}
#[test]
fn both_owned_exports_accept_every_mode_and_keep_all_frames() {
    for library in [false, true] {
        for mode in 0..=24 {
            let out = std::env::temp_dir().join(format!(
                "fvid-removegrain-{}-{library}-{mode}.mkv",
                std::process::id()
            ));
            let request = fvid::media::LosslessTransform {
                removegrain: Some(format!("{mode}:{mode}:{mode}:{mode}")),
                ..Default::default()
            };
            let stats = if library {
                fvid_media::transcode_lossless(
                    &fixture("removegrain.y4m"),
                    &out,
                    request,
                    &Default::default(),
                )
            } else {
                fvid::media::transcode_lossless(
                    &fixture("removegrain.y4m"),
                    &out,
                    request,
                    &Default::default(),
                )
            }
            .unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(stats.video_frames, 4);
            assert_eq!(read_pixels(&out), expected(mode));
            std::fs::remove_file(out).unwrap();
        }
    }
}
#[test]
fn cli_decode_export_and_lossless_use_owned_filter() {
    let input = fixture("removegrain.y4m");
    let out = std::env::temp_dir().join(format!("fvid-removegrain-cli-{}.mkv", std::process::id()));
    for operation in ["decode", "transcode-lossless"] {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"));
        command.args(["media", operation]).arg(&input);
        if operation != "decode" {
            command.arg(&out);
        }
        let result = command
            .args(["--removegrain", "19:19:19"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(stats["backend"], "fvid");
        if operation != "decode" {
            assert_eq!(read_pixels(&out), expected(19));
            std::fs::remove_file(&out).unwrap();
        }
    }
    let out = out.with_extension("y4m");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "export-y4m"])
        .arg(&input)
        .arg(&out)
        .args(["--removegrain", "19:19:19"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(read_pixels(&out), expected(19));
    std::fs::remove_file(out).unwrap();
}
#[test]
fn enable_selection_uses_filter_input_clock_before_framestep() {
    let source = fixture("removegrain.y4m");
    let gold = expected(19);
    for library in [false, true] {
        let out = std::env::temp_dir().join(format!(
            "fvid-removegrain-enable-{}-{library}.mkv",
            std::process::id()
        ));
        let request = fvid::media::LosslessTransform {
            removegrain: Some("19:19:19:enable='eq(n,2)*gte(t,0.08)'".into()),
            framestep: Some("2".into()),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::transcode_lossless(&source, &out, request, &Default::default())
        } else {
            fvid::media::transcode_lossless(&source, &out, request, &Default::default())
        }
        .unwrap();
        assert_eq!(stats.video_frames, 2);
        let mut want = frame(0).data;
        want.extend_from_slice(&gold[2 * 347..3 * 347]);
        assert_eq!(read_pixels(&out), want);
        std::fs::remove_file(out).unwrap();
    }
}

#[test]
fn chain_order_and_compressed_sources_keep_owned_routing() {
    let source = fixture("removegrain.y4m");
    let expected = std::fs::read(fixture("removegrain-chain.expected.raw")).unwrap();
    let base = std::env::temp_dir().join(format!(
        "fvid-removegrain-source-{}.mkv",
        std::process::id()
    ));
    fvid::media::transcode_lossless(&source, &base, Default::default(), &Default::default())
        .unwrap();
    for library in [false, true] {
        let request = fvid::media::DecodeTransform {
            removegrain: Some("20:19:10".into()),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::decode_video_transformed(&base, request)
        } else {
            fvid::media::decode_video_transformed(&base, request)
        }
        .unwrap();
        assert_eq!(stats.video_frames, 4);
        assert_eq!(
            stats.backend,
            if library {
                "owned Matroska FFV1 decode"
            } else {
                "fvid"
            }
        );
        let out = base.with_file_name(format!(
            "fvid-removegrain-chain-{}-{library}.mkv",
            std::process::id()
        ));
        let request = fvid::media::LosslessTransform {
            pixelize: Some("3:3:avg:7".into()),
            removegrain: Some("20:19:10".into()),
            yaepblur: Some("r=2:p=7:s=1024".into()),
            ..Default::default()
        };
        let stats = if library {
            fvid_media::transcode_lossless(&base, &out, request, &Default::default())
        } else {
            fvid::media::transcode_lossless(&base, &out, request, &Default::default())
        }
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(read_pixels(&out), expected);
        std::fs::remove_file(out).unwrap();
    }
    std::fs::remove_file(base).unwrap();
    for source in [
        "../video.mp4",
        "../hevc/main10-ipb.mp4",
        "../vp9/adaptive.webm",
        "../av1/ramp.webm",
    ] {
        let stats = fvid::media::decode_video_transformed(
            &fixture(source),
            fvid::media::DecodeTransform {
                removegrain: Some("20:19:10".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
}

#[test]
fn equal_cost_axes_and_nearest_neighbors_keep_the_qualified_tie_order() {
    for (mode, want) in [
        (5, 110),
        (6, 110),
        (7, 110),
        (8, 110),
        (9, 80),
        (10, 110),
        (18, 110),
    ] {
        let mut data = vec![90, 110, 130, 70, 100, 80, 140, 120, 80];
        RemoveGrain::parse(&format!("m0={mode}"))
            .unwrap()
            .apply_plane(&mut data, 3, 3, 8, 0, 0, None)
            .unwrap();
        assert_eq!(data[4], want, "mode {mode}");
    }
}
