#![cfg(all(target_os = "macos", feature = "player"))]
use fvid::playback_native::NativeReader;
use std::process::Command;

#[test]
#[ignore = "requires physical Metal and VideoToolbox"]
fn cli_copies_all_aac_tracks() {
    let output = std::env::temp_dir().join(format!(
        "fvid-copy-audio-{}-{}.mkv",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(output.clone());
    let input = output.with_extension("mp4");
    let _input_cleanup = Cleanup(input.clone());
    let mut bytes = include_bytes!("fixtures/audio/two-audio.mp4").to_vec();
    let source =
        fvid::container::mp4::Mp4Reader::open(std::io::Cursor::new(&bytes), Default::default())
            .unwrap();
    assert_eq!(source.tracks()[0].handler, *b"vide");
    let track = &source.tracks()[0];
    assert_eq!(track.edits.len(), 1);
    let begin = track.duration / 4;
    let edit_duration = track.edits[0].duration / 2;
    let expected_end_ns =
        u128::from(edit_duration) * 1_000_000_000 / u128::from(source.movie_timescale());
    // This fixture's first elst belongs to the first (video) track. Keep the
    // atom sizes and sample tables unchanged, changing only its media window.
    let elst = bytes.windows(4).position(|b| b == b"elst").unwrap() + 4;
    assert_eq!(bytes[elst], 0);
    assert_eq!(
        u32::from_be_bytes(bytes[elst + 4..elst + 8].try_into().unwrap()),
        1
    );
    bytes[elst + 8..elst + 12]
        .copy_from_slice(&u32::try_from(edit_duration).unwrap().to_be_bytes());
    bytes[elst + 12..elst + 16].copy_from_slice(&i32::try_from(begin).unwrap().to_be_bytes());
    let edited =
        fvid::container::mp4::Mp4Reader::open(std::io::Cursor::new(&bytes), Default::default())
            .unwrap();
    assert_eq!(edited.tracks()[0].edits[0].media_time, begin as i64);
    std::fs::write(&input, bytes).unwrap();
    let result = Command::new(test_binary())
        .arg("shader-export")
        .arg(&input)
        .arg(&output)
        .args(["--copy-audio", "--shader", "shaders/grayscale.wgsl"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut demux = fvid::container::webm::WebmReader::open(
        std::fs::File::open(&output).unwrap(),
        Default::default(),
    )
    .unwrap();
    demux.scan_all().unwrap();
    let video_number = demux.tracks.iter().find(|t| t.kind == 1).unwrap().number;
    let video_packets: Vec<_> = demux
        .packets
        .iter()
        .filter(|p| p.track == video_number)
        .collect();
    assert_eq!(video_packets.first().unwrap().pts_ns, 0);
    let last = video_packets.last().unwrap();
    assert!(
        last.pts_ns as u128 + u128::from(last.duration_ns.unwrap()) <= expected_end_ns + 1_000_000
    );
    assert_eq!(
        demux.tracks.iter().filter(|t| t.codec == "A_AAC").count(),
        2
    );
    for track in demux.tracks.iter().filter(|t| t.codec == "A_AAC") {
        assert!(demux.packets.iter().any(|p| p.track == track.number));
    }
    let mut video = NativeReader::new(
        std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
        64 * 1024 * 1024,
    )
    .unwrap();
    let mut count = 0;
    while video.read_frame_raw().unwrap().is_some() {
        count += 1;
    }
    assert!(count > 0);
}

#[test]
#[ignore = "requires physical Metal and VideoToolbox"]
fn cli_exports_shader_video_and_preserves_existing_output() {
    let directory = std::env::temp_dir().join(format!(
        "fvid-shader-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let lut = directory.join("constant.cube");
    std::fs::write(&lut, "LUT_1D_SIZE 2\n0.25 0.25 0.25\n0.25 0.25 0.25\n").unwrap();
    let y4m = directory.join("input.y4m");
    let mut source = b"YUV4MPEG2 W16 H12 F25:1 Ip A1:1 C420\n".to_vec();
    for value in [40, 100, 180] {
        source.extend_from_slice(b"FRAME\n");
        source.extend_from_slice(&vec![value; 16 * 12]);
        source.extend_from_slice(&vec![128; 2 * 8 * 6]);
    }
    std::fs::write(&y4m, source).unwrap();
    for (index, input) in [
        "tests/fixtures/display/par-2x1.mp4",
        "tests/fixtures/hevc/hdr10.mp4",
        "tests/fixtures/hevc/hlg.mp4",
        y4m.to_str().unwrap(),
    ]
    .into_iter()
    .enumerate()
    {
        for (codec, depth) in [("h264", "8"), ("hevc", "10")] {
            for with_lut in [false, true] {
                let output = directory.join(format!("{index}-{codec}-{with_lut}.mkv"));
                let invoke = || {
                    let mut command = Command::new(test_binary());
                    command.args(["shader-export", input]).arg(&output).args([
                        "--video-only",
                        "--shader",
                        "shaders/grayscale.wgsl",
                        "--size",
                        "32x48",
                        "--codec",
                        codec,
                        "--depth",
                        depth,
                        "--sdr-nits",
                        "100",
                        "--tonemap",
                        "hable",
                    ]);
                    if with_lut {
                        command.arg("--lut").arg(&lut);
                    }
                    if index == 3 {
                        command.args(["--crop", "2:2:8:6"]);
                    }
                    command.output().unwrap()
                };
                let result = invoke();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                let bytes = std::fs::read(&output).unwrap();
                let mut reader =
                    NativeReader::new(std::io::Cursor::new(bytes.clone()), 64 * 1024 * 1024)
                        .unwrap();
                let mut count = 0;
                while let Some(frame) = reader.read_frame_raw().unwrap() {
                    if index == 3 {
                        let (ticks, scale) = reader.current_pts().unwrap();
                        assert_eq!(ticks as f64 / scale as f64, count as f64 / 25.0);
                    }
                    assert_eq!(reader.dimensions(), [32, 48]);
                    let colour = reader.colour();
                    assert_eq!(
                        (colour.primaries, colour.transfer, colour.matrix),
                        (1, 1, 1)
                    );
                    let rgb = frame.into_rgb(64 * 1024 * 1024).unwrap();
                    if with_lut {
                        assert!(
                            rgb.iter().all(|&v| v.abs_diff(64) <= 4),
                            "constant LUT must survive grading, shader and encoding"
                        );
                    }
                    assert!(
                        rgb.chunks_exact(3)
                            .all(|p| p[0].abs_diff(p[1]) <= 4 && p[1].abs_diff(p[2]) <= 4)
                    );
                    count += 1;
                }
                assert!(count > 0);
                if index == 3 {
                    assert_eq!(count, 3);
                }
                let repeated = invoke();
                assert!(!repeated.status.success());
                assert_eq!(std::fs::read(&output).unwrap(), bytes);
            }
        }
    }
    let shader = directory.join("hdr.wgsl");
    std::fs::write(
        &shader,
        "fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return rgb; }",
    )
    .unwrap();
    for (display, transfer) in [("pq", 16), ("hlg", 18)] {
        let output = directory.join(format!("{display}.mkv"));
        let result = Command::new(test_binary())
            .args(["shader-export", "tests/fixtures/hevc/hdr10.mp4"])
            .arg(&output)
            .args([
                "--video-only",
                "--codec",
                "hevc",
                "--depth",
                "10",
                "--display",
                display,
                "--hdr-nits",
                "1000",
                "--max-cll",
                "900",
                "--max-fall",
                "300",
                "--mastering-display",
                "0.68,0.32,0.265,0.69,0.15,0.06,0.3127,0.329,1000,0.005",
                "--shader",
            ])
            .arg(&shader)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let mut reader = NativeReader::new(
            std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
            64 * 1024 * 1024,
        )
        .unwrap();
        let mut count = 0;
        while let Some(frame) = reader.read_frame_raw().unwrap() {
            let NativeReader::Webm(ref video) = reader else {
                panic!("Matroska reader required")
            };
            let coded = video.bitstream_colour();
            let hdr = video.bitstream_hdr();
            assert_eq!(hdr.light.max_cll, 900.0);
            assert_eq!(hdr.light.max_fall, 300.0);
            let container = video.hdr();
            assert_eq!(container.light, hdr.light);
            assert_eq!(
                fvid::color::hdr::mdcv_payload(&container.mastering.unwrap()),
                fvid::color::hdr::mdcv_payload(&hdr.mastering.unwrap())
            );
            assert_eq!(
                (coded.primaries, coded.transfer, coded.matrix),
                (9, transfer, 9),
                "HEVC VUI must describe HDR output independently of container"
            );
            assert_eq!(
                (
                    reader.colour().primaries,
                    reader.colour().transfer,
                    reader.colour().matrix
                ),
                (9, transfer, 9)
            );
            match frame {
                fvid::playback_native::RawFrame::Planar(p) => assert_eq!(p.depth, 10),
                fvid::playback_native::RawFrame::Avc { picture, .. } => {
                    assert_eq!(picture.bit_depth, 10)
                }
                _ => panic!("HDR export must decode as Main10"),
            }
            count += 1;
        }
        assert!(count > 0);
    }
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 21);
    let rejected = directory.join("invalid.mkv");
    let result = Command::new(test_binary())
        .args(["shader-export", "tests/fixtures/hevc/hdr10.mp4"])
        .arg(&rejected)
        .args([
            "--video-only",
            "--codec",
            "hevc",
            "--depth",
            "10",
            "--display",
            "pq",
            "--max-cll",
            "100",
            "--max-fall",
            "200",
            "--shader",
        ])
        .arg(&shader)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!rejected.exists());
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 21);
}
fn test_binary() -> std::path::PathBuf {
    std::env::var_os("FVID_TEST_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_BIN_EXE_fvid")))
}
