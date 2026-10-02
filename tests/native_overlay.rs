use fvid::{
    media_control::{CancelFlag, ProgressHook},
    native_export,
    native_geometry::VideoGeometry,
    playback_native::NativeReader,
};
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-overlay-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
fn samples(path: &Path) -> Vec<Vec<u8>> {
    let mut reader =
        NativeReader::software(BufReader::new(File::open(path).unwrap()), usize::MAX).unwrap();
    let mut result = vec![];
    while let Some(frame) = reader.read_frame_raw().unwrap() {
        let [w, h] = reader.dimensions();
        result.push(
            VideoGeometry::default()
                .apply_cropped_display(&frame, w, h, reader.rotation(), reader.insets())
                .unwrap()
                .data,
        );
    }
    result
}
#[test]
fn overlay_exports_avc_hevc_main10_exact_samples_with_atomic_publication() {
    let d = dir("samples");
    for (i, name) in [
        "video.mp4",
        "hevc/main-ipb.mp4",
        "hevc/main10-ipb.mp4",
        "audio/two-audio.mp4",
    ]
    .iter()
    .enumerate()
    {
        let source = fixture(name);
        let output = d.0.join(format!("{i}.mkv"));
        let expected = samples(&source);
        let plan = fvid::native_plan::overlay(&source, &source, 0, 0).unwrap();
        assert!(plan.steps.iter().any(|s| s.action == "overlay"));
        assert_eq!(plan.inputs.len(), 2);
        let published = output.clone();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = done.clone();
        let hook = ProgressHook::new(move |e| {
            if e.done {
                assert!(published.exists());
                seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        });
        let stats =
            native_export::overlay_video(&source, &source, &output, 0, 0, None, Some(&hook))
                .unwrap();
        assert_eq!(stats.video_frames, expected.len() as u64);
        assert_eq!(samples(&output), expected);
        assert_eq!(done.load(std::sync::atomic::Ordering::Relaxed), 1);
        let bytes = std::fs::read(&output).unwrap();
        let cli = d.0.join(format!("cli-{i}.mkv"));
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "overlay"])
            .arg(&source)
            .arg(&source)
            .arg(&cli)
            .arg("--progress")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let cli_stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(cli_stats["backend"], "fvid");
        assert_eq!(std::fs::read(cli).unwrap(), bytes);
        let events: Vec<serde_json::Value> = String::from_utf8(result.stderr)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(events.iter().filter(|e| e["done"] == true).count(), 1);
        assert_eq!(events.last().unwrap()["done"], true);
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "overlay"])
            .arg(&source)
            .arg("--overlay")
            .arg(&source)
            .output()
            .unwrap();
        assert!(result.status.success());
        let plan: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert!(
            plan["steps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["action"] == "overlay")
        );

        if name.contains("two-audio") {
            assert!(stats.copied_packets > 0);
            let second = d.0.join("matroska-aac-overlay.mkv");
            let copied =
                native_export::overlay_video(&output, &output, &second, 0, 0, None, None).unwrap();
            assert_eq!(copied.copied_packets, stats.copied_packets);
            assert_eq!(samples(&second), expected);
            let bytes = std::fs::read(second).unwrap();

            let mut input = fvid::container::mp4::Mp4Reader::open(
                BufReader::new(File::open(&source).unwrap()),
                Default::default(),
            )
            .unwrap();
            let mut mkv = fvid::container::webm::WebmReader::open(
                std::io::Cursor::new(&bytes),
                Default::default(),
            )
            .unwrap();
            mkv.scan_all().unwrap();
            assert_eq!(input.tracks().len(), mkv.tracks.len());
            assert_eq!(input.tags(), &mkv.tags);
            for (i, track) in input.tracks().to_vec().iter().enumerate() {
                if track.handler != *b"soun" {
                    continue;
                }
                let indices: Vec<_> = mkv
                    .packets
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.track == i as u64 + 1)
                    .map(|(j, _)| j)
                    .collect();
                assert_eq!(mkv.tracks[i].codec, "A_AAC");
                assert_eq!(indices.len(), track.samples.len());
                for (j, index) in indices.into_iter().enumerate() {
                    let mut original = Vec::new();
                    input.read_packet(i, j, &mut original).unwrap();
                    assert_eq!(mkv.read_packet(index).unwrap(), original);
                }
            }
        }

        assert!(native_export::overlay_video(&source, &source, &output, 0, 0, None, None).is_err());
        assert_eq!(std::fs::read(&output).unwrap(), bytes);

        #[cfg(feature = "media")]
        {
            let public = d.0.join(format!("public-{i}.mkv"));
            fvid::media::overlay_video(&source, &source, &public, 0, 0, &Default::default())
                .unwrap();
            assert_eq!(std::fs::read(public).unwrap(), bytes);
        }
    }
}
#[test]
fn cancellation_and_unaligned_chroma_leave_no_destination_or_temporaries() {
    let d = dir("control");
    let source = fixture("video.mp4");
    let output = d.0.join("out.mkv");
    let cancel = CancelFlag::new();
    let trigger = cancel.clone();
    let hook = ProgressHook::new(move |e| {
        assert!(!e.done);
        trigger.cancel();
    });
    assert!(
        native_export::overlay_video(&source, &source, &output, 0, 0, Some(&cancel), Some(&hook))
            .is_err()
    );
    assert!(!output.exists());
    assert!(native_export::overlay_video(&source, &source, &output, 1, 0, None, None).is_err());
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 0);
}

#[test]
fn matroska_vp9_av1_high_depth_and_y4m_overlay_preserve_samples() {
    let d = dir("webm");
    for (i, name) in [
        "vp9/adaptive.webm",
        "vp9/odd10.webm",
        "vp9/lossless12.webm",
        "av1/ramp.webm",
        "av1/tiles.webm",
        "display/vp9-rot90.mkv",
        "geometry/422.y4m",
    ]
    .iter()
    .enumerate()
    {
        let source = fixture(name);
        let output = d.0.join(format!("{i}.mkv"));
        assert!(native_export::overlay_eligible(&source).unwrap());
        let expected = samples(&source);
        let stats =
            native_export::overlay_video(&source, &source, &output, 0, 0, None, None).unwrap();
        assert_eq!(stats.video_frames, expected.len() as u64);
        assert_eq!(samples(&output), expected, "{name}");
        let cli = d.0.join(format!("cli-{i}.mkv"));
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "overlay"])
            .arg(&source)
            .arg(&source)
            .arg(&cli)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(std::fs::read(cli).unwrap(), std::fs::read(&output).unwrap());
    }
}

#[test]
fn overlay_transform_uses_owned_lossless_export_plan_and_headless_commands() {
    let d = dir("transform");
    let source = fixture("video.mp4");
    let reference = d.0.join("reference.mkv");
    native_export::overlay_video(&source, &source, &reference, 0, 0, None, None).unwrap();
    let expected = std::fs::read(reference).unwrap();
    let transform = fvid::media_info::LosslessTransform {
        overlay: Some(fvid::media_info::OverlaySpec {
            path: source.clone(),
            x: 0,
            y: 0,
        }),
        ..Default::default()
    };
    assert!(fvid::native_lossless::overlay_only(&transform).is_some());
    let filtered = fvid::media_info::LosslessTransform {
        negate: Some("1".into()),
        ..transform.clone()
    };
    assert!(fvid::native_lossless::overlay_only(&filtered).is_none());
    for command in ["transcode-lossless", "transcode"] {
        let output = d.0.join(format!("{command}.mkv"));
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"));
        cmd.args(["media", command])
            .arg(&source)
            .arg(&output)
            .arg("--overlay")
            .arg(&source);
        if command == "transcode" {
            cmd.args(["--encoder", "ffv1"]);
        }
        let result = cmd.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(stats["backend"], "fvid");
        assert_eq!(std::fs::read(output).unwrap(), expected);
    }
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "transcode-lossless"])
        .arg(&source)
        .arg("--overlay")
        .arg(&source)
        .output()
        .unwrap();
    assert!(result.status.success());
    let plan: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(plan["command"], "transcode-lossless");
    #[cfg(feature = "media")]
    {
        let options = Default::default();
        let a = d.0.join("api.mkv");
        assert_eq!(
            fvid::media::transcode_lossless(&source, &a, transform.clone(), &options)
                .unwrap()
                .backend,
            "fvid"
        );
        assert_eq!(std::fs::read(a).unwrap(), expected);
        let b = d.0.join("explicit.mkv");
        let encoder = fvid::media_info::EncoderSettings {
            name: "ffv1".into(),
            options: vec![],
        };
        assert_eq!(
            fvid::media::transcode(&source, &b, transform.clone(), &options, &encoder)
                .unwrap()
                .backend,
            "fvid"
        );
        assert_eq!(std::fs::read(b).unwrap(), expected);
        let plan =
            fvid::media::plan_transcode_lossless(&source, &transform, &options, Some("ffv1"))
                .unwrap();
        assert_eq!(plan.command, "transcode-lossless");
        assert!(plan.steps.iter().any(|s| s.action == "overlay"));
    }
}

#[test]
fn combined_geometry_overlay_and_negate_apply_in_the_documented_order() {
    let d = dir("filters");
    for (i, name) in ["video.mp4", "vp9/odd10.webm"].iter().enumerate() {
        let source = fixture(name);
        let expected = samples(&source)
            .into_iter()
            .map(|data| {
                if name.contains("odd10") {
                    data.chunks_exact(2)
                        .flat_map(|p| (1023 - u16::from_le_bytes([p[0], p[1]])).to_le_bytes())
                        .collect::<Vec<_>>()
                } else {
                    data.into_iter().map(|v| 255 - v).collect()
                }
            })
            .collect::<Vec<_>>();
        let geometry = fvid::native_geometry::VideoGeometry {
            horizontal_flip: true,
            ..Default::default()
        };
        let mut filters = fvid::native_pixels::PixelFilters::default();
        filters.negate = Some(fvid::native_pixels::Negate);
        let output = d.0.join(format!("{i}.mkv"));
        native_export::overlay_video_transformed(
            &source, &source, &output, 0, 0, None, None, &geometry, &filters,
        )
        .unwrap();
        // Full foreground replacement cancels the flipped main; negate must
        // subsequently affect every foreground sample.
        assert_eq!(samples(&output), expected);
        let cli = d.0.join(format!("cli-{i}.mkv"));
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "transcode-lossless"])
            .arg(&source)
            .arg(&cli)
            .arg("--overlay")
            .arg(&source)
            .args(["--hflip", "--negate", "1"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(std::fs::read(cli).unwrap(), std::fs::read(&output).unwrap());
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "transcode-lossless"])
            .arg(&source)
            .arg("--overlay")
            .arg(&source)
            .args(["--hflip", "--negate", "1"])
            .output()
            .unwrap();
        assert!(result.status.success());
        let planned: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        let actions: Vec<_> = planned["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["action"].as_str().unwrap())
            .collect();
        assert_eq!(
            actions,
            ["decode", "geometry", "overlay", "filter", "encode", "write"]
        );

        let transform = fvid::media_info::LosslessTransform {
            horizontal_flip: true,
            negate: Some("1".into()),
            overlay: Some(fvid::media_info::OverlaySpec {
                path: source.clone(),
                x: 0,
                y: 0,
            }),
            ..Default::default()
        };
        assert!(fvid::native_lossless::supports_overlay(&transform));
        let plan = fvid::native_plan::transcode_lossless(&source, &transform).unwrap();
        let actions: Vec<_> = plan.steps.iter().map(|s| s.action.as_str()).collect();
        assert_eq!(
            actions,
            ["decode", "geometry", "overlay", "filter", "encode", "write"]
        );
        #[cfg(feature = "media")]
        {
            let public = d.0.join(format!("public-{i}.mkv"));
            fvid::media::transcode_lossless(&source, &public, transform, &Default::default())
                .unwrap();
            assert_eq!(
                std::fs::read(public).unwrap(),
                std::fs::read(output).unwrap()
            );
        }
    }
}

#[test]
fn filter_flag_permutations_match_media_transform_order() {
    let d = dir("order");
    let source = fixture("video.mp4");
    for (i, (spatial, flags)) in [
        (
            fvid::media_info::LosslessTransform {
                sobel: Some("1:1:0".into()),
                prewitt: Some("1:1:0".into()),
                ..Default::default()
            },
            vec![("--sobel", "1:1:0"), ("--prewitt", "1:1:0")],
        ),
        (
            fvid::media_info::LosslessTransform {
                dilation: Some("".into()),
                erosion: Some("".into()),
                ..Default::default()
            },
            vec![("--dilation", ""), ("--erosion", "")],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let (geometry, filters) = fvid::native_lossless::configuration(&spatial).unwrap();
        let output = d.0.join(format!("reference-{i}.mkv"));
        native_export::overlay_video_transformed(
            &source, &source, &output, 0, 0, None, None, &geometry, &filters,
        )
        .unwrap();
        let expected = std::fs::read(output).unwrap();
        for reversed in [false, true] {
            let out = d.0.join(format!("cli-{i}-{reversed}.mkv"));
            let mut options = flags.clone();
            if reversed {
                options.reverse();
            }
            let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"));
            cmd.args(["media", "transcode-lossless"])
                .arg(&source)
                .arg(&out)
                .arg("--overlay")
                .arg(&source);
            for (flag, value) in options {
                cmd.args([flag, value]);
            }
            let result = cmd.output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(
                std::fs::read(out).unwrap(),
                expected,
                "filter order {i}, reversed={reversed}"
            );
        }
        #[cfg(feature = "media")]
        {
            let mut transform = spatial;
            transform.overlay = Some(fvid::media_info::OverlaySpec {
                path: source.clone(),
                x: 0,
                y: 0,
            });
            let out = d.0.join(format!("api-{i}.mkv"));
            fvid::media::transcode_lossless(&source, &out, transform, &Default::default()).unwrap();
            assert_eq!(std::fs::read(out).unwrap(), expected);
        }
    }
}

#[test]
fn mixed_depth_overlay_converts_codes_and_preserves_main_precision() {
    let d = dir("mixed-depth");
    for full in [false, true] {
        let range = if full { "FULL" } else { "LIMITED" };
        let main = d.0.join(format!("main-{full}.mkv"));
        let foreground = d.0.join(format!("foreground-{full}.y4m"));
        let output = d.0.join(format!("output-{full}.mkv"));
        let mut bytes =
            format!("YUV4MPEG2 W2 H2 F25:1 Ip A1:1 C420 XCOLORRANGE={range}\nFRAME\n").into_bytes();
        bytes.extend([0, 16, 235, 255, 128, 255]);
        std::fs::write(&foreground, bytes).unwrap();
        let mut reader =
            NativeReader::software(BufReader::new(File::open(&foreground).unwrap()), usize::MAX)
                .unwrap();
        reader.read_frame_raw().unwrap().unwrap();
        let colour = reader.colour();
        let frame = fvid::native_geometry::GeometryFrame {
            width: 2,
            height: 2,
            subsampling: Some([2, 2]),
            data: vec![0; 12],
        };
        let packet = fvid::codec::ffv1_encoder::encode(&frame, 10).unwrap();
        use fvid::container::matroska_write::{Encoding, PacketWriter, TrackSpec, VideoMetadata};
        let mut file = File::create(&main).unwrap();
        let mut mux = PacketWriter::new_with_video_metadata(
            &mut file,
            &[TrackSpec {
                encoding: Encoding::Ffv1V1 {
                    width: 2,
                    height: 2,
                },
                name: "main",
                language: "und",
            }],
            &[Some(VideoMetadata {
                colour: Some(colour),
                ..Default::default()
            })],
        )
        .unwrap();
        mux.write_packet(0, 0, 40_000_000, true, &packet).unwrap();
        mux.finish().unwrap();
        drop(file);

        native_export::overlay_video(&main, &foreground, &output, 0, 0, None, None).unwrap();
        let expected: [u16; 6] = if full {
            [0, 64, 943, 1023, 512, 1023]
        } else {
            [0, 64, 940, 1020, 512, 1020]
        };
        assert_eq!(
            samples(&output),
            vec![
                expected
                    .into_iter()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<_>>()
            ]
        );
    }
}
