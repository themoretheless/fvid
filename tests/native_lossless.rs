use fvid::{
    container::{mp4, webm},
    playback_native::NativeReader,
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn directory(name: &str) -> Directory {
    let p = std::env::temp_dir().join(format!("fvid-lossless-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Directory(p)
}
#[test]
fn packets_timing_metadata_and_audio_survive_transcode() {
    for name in [
        "video.mp4",
        "hevc/main-ipb.mp4",
        "hevc/main10-ipb.mp4",
        "hevc/hdr10.mp4",
        "display/par-2x1.mp4",
        "audio/two-audio.mp4",
    ] {
        let source = fixture(name);
        let bytes = std::fs::read(&source).unwrap();
        let mut output = Cursor::new(Vec::new());
        let (stats, event) =
            fvid::native_lossless::write_mp4(&source, &mut output, None, None).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(!event.done);
        let mut input = mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
        let mut mkv =
            webm::WebmReader::open(Cursor::new(output.into_inner()), Default::default()).unwrap();
        mkv.scan_all().unwrap();
        assert_eq!(input.tracks().len(), mkv.tracks.len());
        assert_eq!(input.tags(), &mkv.tags);
        let mut raw = NativeReader::software(Cursor::new(&bytes), usize::MAX).unwrap();
        let mut times = Vec::new();
        while raw.read_frame_raw().unwrap().is_some() {
            let (a, b, scale) = raw.frame_interval().unwrap();
            times.push((
                (a * 1_000_000_000 / u128::from(scale)) as i64,
                ((b * 1_000_000_000 / u128::from(scale)) - (a * 1_000_000_000 / u128::from(scale)))
                    as u64,
            ));
        }
        for (i, track) in input.tracks().to_vec().iter().enumerate() {
            assert_eq!(track.name, mkv.tracks[i].name);
            let indices = mkv
                .packets
                .iter()
                .enumerate()
                .filter(|(_, p)| p.track == i as u64 + 1)
                .map(|(j, _)| j)
                .collect::<Vec<_>>();
            if track.handler == *b"vide" {
                assert_eq!(mkv.tracks[i].codec, "V_FFV1");
                assert_eq!(indices.len(), times.len());
                assert_eq!(stats.video_frames, times.len() as u64);
                for (j, &index) in indices.iter().enumerate() {
                    let p = &mkv.packets[index];
                    assert_eq!(p.pts_ns, times[j].0);
                    assert_eq!(p.duration_ns, Some(times[j].1));
                    assert!(!p.invisible);
                }
            } else {
                assert_eq!(mkv.tracks[i].codec, "A_AAC");
                for (j, index) in indices.iter().enumerate() {
                    let mut original = Vec::new();
                    input.read_packet(i, j, &mut original).unwrap();
                    assert_eq!(mkv.read_packet(*index).unwrap(), original);
                }
            }
        }
    }
}
#[test]
fn cli_api_cancel_and_atomic_publication() {
    let dir = directory("publish");
    let source = fixture("audio/two-audio.mp4");
    let destination = dir.0.join("result.mkv");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "transcode-lossless"])
        .arg(&source)
        .arg(&destination)
        .arg("--progress")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["backend"],
        "fvid"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .last()
            .unwrap()
            .contains("\"done\":true")
    );
    let before = std::fs::read(&destination).unwrap();
    assert!(fvid::native_export::transcode_mp4_ffv1(&source, &destination, None, None).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), before);
    let cancel = fvid::media_control::CancelFlag::default();
    let stop = cancel.clone();
    let hook = fvid::media_control::ProgressHook::new(move |e| {
        assert!(!e.done);
        if e.packets >= 2 {
            stop.cancel();
        }
    });
    let cancelled = dir.0.join("cancelled.mkv");
    assert!(
        fvid::native_export::transcode_mp4_ffv1(&source, &cancelled, Some(&cancel), Some(&hook))
            .is_err()
    );
    assert!(!cancelled.exists());
    assert!(
        !std::fs::read_dir(&dir.0).unwrap().any(|p| p
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp"))
    );
    #[cfg(feature = "media")]
    {
        let path = dir.0.join("api.mkv");
        let stats = fvid::media::transcode_lossless(
            &source,
            &path,
            Default::default(),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
}

#[test]
fn rotation_aspect_hdr_and_completion_publication_are_preserved() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let dir = directory("display");
    for rotation in [90, 180, 270] {
        let mut bytes = std::fs::read(fixture("display/par-2x1.mp4")).unwrap();
        let matrix = bytes.windows(4).position(|w| w == b"tkhd").unwrap() + 44;
        let values: [i32; 4] = match rotation {
            90 => [0, 65536, -65536, 0],
            180 => [-65536, 0, 0, -65536],
            _ => [0, -65536, 65536, 0],
        };
        for (offset, value) in [0, 4, 12, 16].into_iter().zip(values) {
            bytes[matrix + offset..matrix + offset + 4].copy_from_slice(&value.to_be_bytes());
        }
        let source = dir.0.join(format!("{rotation}.mp4"));
        std::fs::write(&source, &bytes).unwrap();
        let destination = dir.0.join(format!("{rotation}.mkv"));
        let published = destination.clone();
        let done = Arc::new(AtomicBool::new(false));
        let seen = done.clone();
        let hook = fvid::media_control::ProgressHook::new(move |event| {
            if event.done {
                assert!(published.exists());
                seen.store(true, Ordering::Relaxed);
            }
        });
        fvid::native_export::transcode_mp4_ffv1(&source, &destination, None, Some(&hook)).unwrap();
        assert!(done.load(Ordering::Relaxed));
        let input = mp4::Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
        let result = webm::WebmReader::open(
            Cursor::new(std::fs::read(destination).unwrap()),
            Default::default(),
        )
        .unwrap();
        assert_eq!(result.tracks[0].rotation, rotation);
        let aspect = input.tracks()[0].pixel_aspect;
        assert_eq!(
            result.tracks[0].pixel_aspect(),
            if rotation == 180 {
                aspect
            } else {
                (aspect.1, aspect.0)
            }
        );
    }
    let source = fixture("hevc/hdr10.mp4");
    let mut result = Cursor::new(Vec::new());
    fvid::native_lossless::write_mp4(&source, &mut result, None, None).unwrap();
    let input = mp4::Mp4Reader::open(
        Cursor::new(std::fs::read(source).unwrap()),
        Default::default(),
    )
    .unwrap();
    let output =
        webm::WebmReader::open(Cursor::new(result.into_inner()), Default::default()).unwrap();
    assert_eq!(input.tracks()[0].hdr, output.tracks[0].hdr);
}

#[test]
fn edited_hevc_window_encodes_only_presented_pictures() {
    let dir = directory("edit");
    for name in ["hevc/main-ipb.mp4", "hevc/main10-ipb.mp4"] {
        let mut bytes = std::fs::read(fixture(name)).unwrap();
        let input = mp4::Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
        let track = &input.tracks()[0];
        let mut times = (0..track.samples.len())
            .map(|i| track.samples.get(i).unwrap().pts)
            .collect::<Vec<_>>();
        times.sort();
        let begin = times[2];
        let end = times[times.len() - 2];
        let duration = u32::try_from(
            (end - begin) as u128 * u128::from(input.movie_timescale())
                / u128::from(track.timescale),
        )
        .unwrap();
        let at = bytes.windows(4).position(|w| w == b"elst").unwrap();
        assert_eq!(bytes[at + 4], 0);
        assert_eq!(&bytes[at + 8..at + 12], &1u32.to_be_bytes());
        bytes[at + 12..at + 16].copy_from_slice(&duration.to_be_bytes());
        bytes[at + 16..at + 20].copy_from_slice(&i32::try_from(begin).unwrap().to_be_bytes());
        let source = dir.0.join(name.replace('/', "-"));
        std::fs::write(&source, &bytes).unwrap();
        let mut output = Cursor::new(Vec::new());
        fvid::native_lossless::write_mp4(&source, &mut output, None, None).unwrap();
        let mut raw = NativeReader::software(Cursor::new(bytes), usize::MAX).unwrap();
        let mut expected = Vec::new();
        while raw.read_frame_raw().unwrap().is_some() {
            let (a, b, s) = raw.frame_interval().unwrap();
            expected.push((
                (a * 1_000_000_000 / u128::from(s)) as i64,
                ((b * 1_000_000_000 / u128::from(s)) - (a * 1_000_000_000 / u128::from(s))) as u64,
            ));
        }
        let mut result =
            webm::WebmReader::open(Cursor::new(output.into_inner()), Default::default()).unwrap();
        result.scan_all().unwrap();
        assert_eq!(result.packets.len(), expected.len());
        for (p, (pts, duration)) in result.packets.iter().zip(expected) {
            assert_eq!(p.pts_ns, pts);
            assert_eq!(p.duration_ns, Some(duration));
            assert!(p.keyframe && !p.invisible);
        }
    }
}

fn spatial_request() -> fvid::media_info::LosslessTransform {
    use fvid::media_info::{CropRect, LosslessTransform, PadRect, ScaleSize, TransposeMode};
    LosslessTransform {
        crop: Some(CropRect {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        }),
        horizontal_flip: true,
        vertical_flip: true,
        transpose: Some(TransposeMode::Clock),
        pad: Some(PadRect {
            width: 16,
            height: 12,
            x: 2,
            y: 2,
        }),
        scale: Some(ScaleSize {
            width: 12,
            height: 8,
        }),
        chromashift: Some("cbh=1:cbv=-2:crh=-3:crv=2:edge=wrap".into()),
        negate: Some("1".into()),
        sobel: Some("planes=1:scale=0.125".into()),
        dilation: Some("coordinates=170:threshold0=3:threshold1=0:threshold2=0".into()),
        ..Default::default()
    }
}
#[test]
fn spatial_export_cli_api_metadata_and_failures() {
    let dir = directory("spatial");
    let source = fixture("audio/two-audio.mp4");
    let request = spatial_request();
    assert!(fvid::native_lossless::supports(&request));
    assert!(!fvid::native_lossless::identity(&request));
    let (geometry, filters) = fvid::native_lossless::configuration(&request).unwrap();
    let output = dir.0.join("spatial.mkv");
    let stats = fvid::native_export::transcode_mp4_ffv1_transformed(
        &source, &output, &geometry, &filters, None, None,
    )
    .unwrap();
    assert!(stats.horizontal_flip && stats.vertical_flip);
    assert_eq!(stats.fvid_crop_payload_copies, stats.video_frames);
    let bytes = std::fs::read(&output).unwrap();
    let mut reader = webm::WebmReader::open(Cursor::new(&bytes), Default::default()).unwrap();
    reader.scan_all().unwrap();
    assert_eq!(reader.tracks.len(), 3);
    assert_eq!(reader.tracks[0].visible(), (12, 8));
    assert_eq!(reader.tracks[0].rotation, 0);
    assert_eq!(reader.tracks[0].pixel_aspect(), (8, 9));
    let cli = dir.0.join("cli.mkv");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "transcode-lossless"])
        .arg(&source)
        .arg(&cli)
        .args([
            "--crop",
            "0:0:8:8",
            "--hflip",
            "--vflip",
            "--transpose",
            "clock",
            "--pad",
            "16:12:2:2",
            "--scale",
            "12:8",
            "--chromashift",
            "cbh=1:cbv=-2:crh=-3:crv=2:edge=wrap",
            "--negate",
            "1",
            "--sobel",
            "planes=1:scale=0.125",
            "--dilation",
            "coordinates=170:threshold0=3:threshold1=0:threshold2=0",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read(cli).unwrap(), bytes);
    #[cfg(feature = "media")]
    {
        let api = dir.0.join("api.mkv");
        let stats =
            fvid::media::transcode_lossless(&source, &api, request.clone(), &Default::default())
                .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(std::fs::read(api).unwrap(), bytes);
    }
    let invalid = dir.0.join("invalid.mkv");
    let bad = fvid::native_geometry::VideoGeometry {
        crop: Some([10000, 0, 8, 8]),
        ..Default::default()
    };
    assert!(
        fvid::native_export::transcode_mp4_ffv1_transformed(
            &source, &invalid, &bad, &filters, None, None
        )
        .is_err()
    );
    assert!(!invalid.exists());
    let unsupported = fvid::media_info::LosslessTransform {
        gblur: Some("1".into()),
        ..Default::default()
    };
    assert!(!fvid::native_lossless::supports(&unsupported));
    let original = dir.0.join("original.mkv");
    fvid::native_export::transcode_mp4_ffv1(&source, &original, None, None).unwrap();
    let mut unchanged = webm::WebmReader::open(
        Cursor::new(std::fs::read(original).unwrap()),
        Default::default(),
    )
    .unwrap();
    unchanged.scan_all().unwrap();
    for track in [2, 3] {
        let original = unchanged
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == track)
            .map(|(i, p)| (i, p.pts_ns, p.duration_ns, p.discard_padding_ns))
            .collect::<Vec<_>>();
        let changed = reader
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == track)
            .map(|(i, p)| (i, p.pts_ns, p.duration_ns, p.discard_padding_ns))
            .collect::<Vec<_>>();
        assert_eq!(original.len(), changed.len());
        for ((a, ap, ad, at), (b, bp, bd, bt)) in original.into_iter().zip(changed) {
            assert_eq!((ap, ad, at), (bp, bd, bt));
            assert_eq!(
                unchanged.read_packet(a).unwrap(),
                reader.read_packet(b).unwrap()
            );
        }
    }
}

#[cfg(feature = "media")]
#[test]
fn crop_convenience_api_uses_owned_codec_and_exact_planes() {
    use fvid::native_geometry::VideoGeometry;
    let directory = directory("crop-api");
    for (index, name) in ["video.mp4", "hevc/main-ipb.mp4", "hevc/main10-ipb.mp4"]
        .iter()
        .enumerate()
    {
        let source = fixture(name);
        let output = directory.0.join(format!("{index}.mkv"));
        let crop = fvid::media::CropRect {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        };
        let transform = fvid::media::LosslessTransform {
            crop: Some(crop),
            ..Default::default()
        };
        let plan =
            fvid::media::plan_transcode_lossless(&source, &transform, &Default::default(), None)
                .unwrap();
        assert!(plan.graph.is_none());
        assert!(plan.notes.iter().any(|n| n.contains("backend: fvid")));
        assert!(plan.steps.iter().any(|s| s.action == "geometry"));
        assert!(
            plan.steps
                .iter()
                .any(|s| s.action == "encode" && s.detail.contains("FFV1"))
        );
        assert_eq!(
            plan.streams
                .iter()
                .filter(|s| s.disposition == "primary_video")
                .count(),
            1
        );
        let stats =
            fvid::media::crop_lossless(&source, &output, crop, &Default::default()).unwrap();
        assert_eq!(stats.backend, "fvid");
        let explicit = directory.0.join(format!("explicit-{index}.mkv"));
        let settings = fvid::media::EncoderSettings {
            name: "ffv1".into(),
            options: vec![],
        };
        let explicit_stats = fvid::media::transcode(
            &source,
            &explicit,
            transform.clone(),
            &Default::default(),
            &settings,
        )
        .unwrap();
        assert_eq!(explicit_stats.backend, "fvid");
        assert_eq!(
            std::fs::read(&explicit).unwrap(),
            std::fs::read(&output).unwrap()
        );
        let explicit_plan = fvid::media::plan_transcode_lossless(
            &source,
            &transform,
            &Default::default(),
            Some("ffv1"),
        )
        .unwrap();
        assert_eq!(explicit_plan, plan);
        let cli = directory.0.join(format!("cli-explicit-{index}.mkv"));
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "transcode"])
            .arg(&source)
            .arg(&cli)
            .args(["--encoder", "ffv1", "--crop", "0:0:8:8"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(json["backend"], "fvid");
        assert_eq!(
            std::fs::read(&cli).unwrap(),
            std::fs::read(&output).unwrap()
        );
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "transcode-lossless"])
            .arg(&source)
            .args(["--crop", "0:0:8:8"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let cli_plan: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(cli_plan, serde_json::to_value(&plan).unwrap());

        let mut original = NativeReader::software(
            std::io::BufReader::new(std::fs::File::open(&source).unwrap()),
            usize::MAX,
        )
        .unwrap();
        let mut decoded = NativeReader::software(
            std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
            usize::MAX,
        )
        .unwrap();
        let geometry = VideoGeometry {
            crop: Some([0, 0, 8, 8]),
            ..Default::default()
        };
        let mut count = 0;
        while let Some(frame) = original.read_frame_raw().unwrap() {
            let [w, h] = original.dimensions();
            let expected = geometry
                .apply_display(&frame, w, h, original.rotation())
                .unwrap();
            let frame = decoded.read_frame_raw().unwrap().unwrap();
            let [w, h] = decoded.dimensions();
            let actual = VideoGeometry::default()
                .apply_display(&frame, w, h, decoded.rotation())
                .unwrap();
            assert_eq!(actual.data, expected.data, "{name} frame {count}");
            assert_eq!((actual.width, actual.height), (8, 8));
            count += 1;
        }
        assert!(decoded.read_frame_raw().unwrap().is_none());
        assert_eq!(stats.video_frames, count);
    }
}

#[cfg(feature = "media")]
#[test]
fn owned_lossless_plans_show_filter_order_and_y4m_without_audio() {
    use fvid::media::{LosslessTransform, plan_transcode_lossless};
    let d = directory("plans");
    let y4m = d.0.join("input.y4m");
    let mut bytes = b"YUV4MPEG2 W16 H16 F25:1 Ip C420jpeg\nFRAME\n".to_vec();
    bytes.extend(vec![128; 384]);
    std::fs::write(&y4m, bytes).unwrap();
    let request = LosslessTransform {
        avgblur: Some("1:7:1".into()),
        boxblur: Some("1:1".into()),
        negate: Some("".into()),
        sobel: Some("planes=1".into()),
        pixelize: Some("3:5:avg:7".into()),
        dilation: Some("coordinates=170".into()),
        chromashift: Some("cbh=1".into()),
        ..Default::default()
    };
    for source in [y4m, fixture("video.mp4"), fixture("audio/two-audio.mp4")] {
        let plan = plan_transcode_lossless(&source, &request, &Default::default(), None).unwrap();
        let details: Vec<_> = plan
            .steps
            .iter()
            .filter(|s| s.action == "filter")
            .map(|s| s.detail.split(';').next().unwrap())
            .collect();
        assert_eq!(
            details,
            vec![
                "FVid avgblur=1:7:1",
                "FVid boxblur=1:1",
                "FVid negate=",
                "FVid sobel=planes=1",
                "FVid pixelize=3:5:avg:7",
                "FVid dilation=coordinates=170",
                "FVid chromashift=cbh=1"
            ]
        );
        assert!(plan.graph.is_none());
        assert!(
            plan.streams
                .iter()
                .filter(|s| s.media_type == "audio")
                .all(|s| s.disposition == "copy")
        );
        let output = d.0.join(format!(
            "{}.mkv",
            source.file_name().unwrap().to_string_lossy()
        ));
        let stats =
            fvid::media::transcode_lossless(&source, &output, request.clone(), &Default::default())
                .unwrap();
        assert_eq!(stats.backend, "fvid");
        assert!(stats.video_frames > 0);
    }
}

#[test]
fn crop_lossless_cli_uses_owned_export_and_keeps_command_constraints() {
    let dir = directory("crop-cli");
    let source = fixture("video.mp4");
    let destination = dir.0.join("crop.mkv");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "crop-lossless"])
        .arg(&source)
        .arg(&destination)
        .args(["--crop", "0:0:16:16", "--hflip", "--progress"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(stats["backend"], "fvid");
    assert_eq!(stats["video_frames"], 25);
    let mut reader = NativeReader::software(
        Cursor::new(std::fs::read(&destination).unwrap()),
        usize::MAX,
    )
    .unwrap();
    let mut original =
        NativeReader::software(Cursor::new(std::fs::read(&source).unwrap()), usize::MAX).unwrap();
    let geometry = fvid::native_geometry::VideoGeometry {
        crop: Some([0, 0, 16, 16]),
        horizontal_flip: true,
        ..Default::default()
    };
    let mut count = 0;
    while let Some(frame) = reader.read_frame_raw().unwrap() {
        assert_eq!(reader.dimensions(), [16, 16]);
        let input = original.read_frame_raw().unwrap().unwrap();
        let [w, h] = original.dimensions();
        let expected = geometry
            .apply_display(&input, w, h, original.rotation())
            .unwrap();
        let actual = fvid::native_geometry::VideoGeometry::default()
            .apply_display(&frame, 16, 16, reader.rotation())
            .unwrap();
        assert_eq!(actual.data, expected.data, "frame {count}");
        count += 1;
    }
    assert!(original.read_frame_raw().unwrap().is_none());
    assert_eq!(count, 25);
    for (options, error) in [
        (vec![], "--crop required"),
        (
            vec!["--crop", "0:0:16:16", "--scale", "8:8"],
            "does not take --scale",
        ),
        (
            vec!["--crop", "0:0:16:16", "--hue", "h=90"],
            "does not take pixel filters",
        ),
    ] {
        let rejected = dir.0.join("rejected.mkv");
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "crop-lossless"])
            .arg(&source)
            .arg(&rejected)
            .args(options)
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(error),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!rejected.exists());
    }
}

#[test]
fn explicit_ffv1_transcode_cli_uses_owned_encoder() {
    let dir = directory("ffv1-transcode");
    let source = fixture("video.mp4");
    let destination = dir.0.join("generic.mkv");
    let baseline = dir.0.join("lossless.mkv");
    for (command, output, options) in [
        (
            "transcode",
            &destination,
            vec![
                "--encoder",
                "ffv1",
                "--encoder-option",
                "level=1",
                "--crop",
                "0:0:16:16",
                "--hue",
                "h=90",
            ],
        ),
        (
            "transcode-lossless",
            &baseline,
            vec!["--crop", "0:0:16:16", "--hue", "h=90"],
        ),
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", command])
            .arg(&source)
            .arg(output)
            .args(options)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stats: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(stats["backend"], "fvid");
        assert_eq!(stats["video_frames"], 25);
    }
    assert_eq!(
        std::fs::read(destination).unwrap(),
        std::fs::read(baseline).unwrap()
    );
}

#[test]
fn temporal_cli_export_preserves_expected_frame_order_without_legacy() {
    let dir = directory("temporal-cli");
    let source = fixture("playback-errors/framestep-six-frames.y4m");
    for (option, value, indices) in [
        ("--framestep", "2", vec![0u8, 2, 4]),
        ("--reverse", "", vec![5, 4, 3, 2, 1, 0]),
        ("--shuffleframes", "2 1 0", vec![2, 1, 0, 5, 4, 3]),
    ] {
        let output = dir.0.join(format!("{}.mkv", &option[2..]));
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "transcode-lossless"])
            .arg(&source)
            .arg(&output)
            .args([option, value, "--progress"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stats: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(stats["backend"], "fvid");
        assert_eq!(stats["video_frames"], indices.len());
        assert!(String::from_utf8_lossy(&out.stderr).contains("\"done\":true"));
        let mut source_reader =
            NativeReader::software(Cursor::new(std::fs::read(&source).unwrap()), usize::MAX)
                .unwrap();
        let mut frames = Vec::new();
        while let Some(frame) = source_reader.read_frame_raw().unwrap() {
            let [w, h] = source_reader.dimensions();
            frames.push(
                fvid::native_geometry::VideoGeometry::default()
                    .apply_display(&frame, w, h, 0)
                    .unwrap()
                    .data,
            );
        }
        let mut result =
            NativeReader::software(Cursor::new(std::fs::read(output).unwrap()), usize::MAX)
                .unwrap();
        for index in indices {
            let frame = result.read_frame_raw().unwrap().unwrap();
            let [w, h] = result.dimensions();
            let actual = fvid::native_geometry::VideoGeometry::default()
                .apply_display(&frame, w, h, 0)
                .unwrap();
            assert_eq!(actual.data, frames[index as usize]);
        }
        assert!(result.read_frame_raw().unwrap().is_none());
    }
}

#[test]
fn owned_temporal_cli_interval_matches_api_and_rejects_incomplete_ranges() {
    let dir = directory("temporal-interval");
    let source = fixture("playback-errors/framestep-six-frames.y4m");
    let output = dir.0.join("cli.mkv");
    let baseline = dir.0.join("api.mkv");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "transcode-lossless"])
        .arg(&source)
        .arg(&output)
        .args(["--reverse", "", "--from", "0.25", "--to", "1.0", "--quiet"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty());
    let stats = fvid_media::owned_lossless::transcode_lossless(
        &source,
        &baseline,
        fvid::media_info::LosslessTransform {
            reverse: Some(String::new()),
            interval: Some((250_000, 1_000_000)),
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap();
    assert_eq!(stats.video_frames, 3);
    assert_eq!(
        std::fs::read(output).unwrap(),
        std::fs::read(baseline).unwrap()
    );
    for options in [
        vec!["--from", "0.25"],
        vec!["--from", "1.0", "--to", "0.25"],
    ] {
        let rejected = dir.0.join("rejected.mkv");
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "transcode-lossless"])
            .arg(&source)
            .arg(&rejected)
            .args(["--reverse", ""])
            .args(options)
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("lossless interval requires"));
        assert!(!rejected.exists());
    }
}

#[test]
fn owned_temporal_cli_enforces_input_packet_limit_and_stream_selection() {
    let dir = directory("temporal-policy");
    let source = fixture("playback-errors/framestep-six-frames.y4m");
    for (limit, success) in [("6", true), ("5", false)] {
        let output = dir.0.join(format!("limit-{limit}.mkv"));
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "transcode-lossless"])
            .arg(&source)
            .arg(&output)
            .args([
                "--framestep",
                "2",
                "--streams",
                "0",
                "--max-packets",
                limit,
            ])
            .output()
            .unwrap();
        assert_eq!(
            out.status.success(),
            success,
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        if success {
            let stats: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(stats["backend"], "fvid");
            assert_eq!(stats["video_frames"], 3);
        } else {
            assert!(!output.exists());
            assert!(String::from_utf8_lossy(&out.stderr).contains("packet"));
        }
    }
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}
