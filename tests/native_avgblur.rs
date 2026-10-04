use fvid::{native_avgblur::AverageBlur, native_geometry::GeometryFrame};
fn frame(depth: u8, subsampling: [usize; 2]) -> GeometryFrame {
    let (w, h) = (17usize, 13usize);
    let n = w * h + 2 * w.div_ceil(subsampling[0]) * h.div_ceil(subsampling[1]);
    let mut data = Vec::new();
    for i in 0..n {
        let sample = ((i * 173 + i * i * 11) % (1usize << depth)) as u16;
        if depth == 8 {
            data.push(sample as u8);
        } else {
            data.extend_from_slice(&sample.to_le_bytes());
        }
    }
    GeometryFrame {
        width: w,
        height: h,
        subsampling: Some(subsampling),
        data,
    }
}
#[test]
fn expected_average_and_invalid_storage() {
    let mut f = GeometryFrame {
        width: 3,
        height: 3,
        subsampling: Some([1, 1]),
        data: [vec![0, 0, 0, 0, 90, 0, 0, 0, 0], vec![77; 18]].concat(),
    };
    AverageBlur::parse("1:1:1")
        .unwrap()
        .apply(&mut f, 8)
        .unwrap();
    assert_eq!(&f.data[..9], &[10; 9]);
    assert_eq!(&f.data[9..], &[77; 18]);
    for args in ["0", "1025", "planes=16", "sizeY=-1", "foo=1", "1:2:3:4"] {
        assert!(AverageBlur::parse(args).is_err());
    }
    let mut f = frame(10, [2, 2]);
    f.data.pop();
    let before = f.data.clone();
    assert!(AverageBlur::parse("").unwrap().apply(&mut f, 10).is_err());
    assert_eq!(f.data, before);
}

#[test]
fn decode_cli_and_api_preserve_native_backend_and_intervals() {
    use fvid::media_info::DecodeTransform;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in ["video.mp4", "hevc/main10-ipb.mp4"] {
        let path = root.join(name);
        let request = DecodeTransform {
            avgblur: Some("sizeX=2:sizeY=1:planes=7".into()),
            interval: Some((0, 100000)),
            ..Default::default()
        };
        let direct = fvid::native_media::decode_video_request(&path, &request).unwrap();
        assert_eq!(direct.backend, "fvid");
        assert!(direct.video_frames > 0);
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "decode"])
            .arg(&path)
            .args([
                "--avgblur",
                "sizeX=2:sizeY=1:planes=7",
                "--from",
                "0",
                "--to",
                "0.1",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["backend"], "fvid");
        assert_eq!(value["video_frames"], direct.video_frames);
        {
            let actual = fvid::media::decode_video_transformed(&path, request).unwrap();
            assert_eq!(actual.backend, "fvid");
            assert_eq!(actual.video_frames, direct.video_frames);
        }
    }
}

#[test]
fn tiny_planes_and_full_precision_match_direct_sum() {
    for (w, h, sub, depth) in [
        (1usize, 1usize, [2, 2], 8),
        (1, 5, [1, 1], 16),
        (5, 1, [2, 1], 10),
        (7, 5, [2, 2], 16),
    ] {
        let (cw, ch) = (w.div_ceil(sub[0]), h.div_ceil(sub[1]));
        let count = w * h + 2 * cw * ch;
        let samples: Vec<u16> = (0..count)
            .map(|i| {
                if i % 3 == 0 {
                    ((1u32 << depth) - 1) as u16
                } else {
                    (i * 117 % (1usize << depth)) as u16
                }
            })
            .collect();
        let data = if depth == 8 {
            samples.iter().map(|&s| s as u8).collect()
        } else {
            samples.iter().flat_map(|s| s.to_le_bytes()).collect()
        };
        let mut f = GeometryFrame {
            width: w,
            height: h,
            subsampling: Some(sub),
            data,
        };
        AverageBlur::parse("1024:7:1024")
            .unwrap()
            .apply(&mut f, depth)
            .unwrap();
        let (rx, ry) = (cw / 2, ch / 2);
        let mut expected = Vec::new();
        let mut offset = 0;
        for (pw, ph) in [(w, h), (cw, ch), (cw, ch)] {
            for y in 0..ph {
                for x in 0..pw {
                    let mut sum = 0u64;
                    for dy in -(ry as i64)..=ry as i64 {
                        for dx in -(rx as i64)..=rx as i64 {
                            let xx = (x as i64 + dx).clamp(0, pw as i64 - 1) as usize;
                            let yy = (y as i64 + dy).clamp(0, ph as i64 - 1) as usize;
                            sum += u64::from(samples[offset + yy * pw + xx]);
                        }
                    }
                    let value = (sum / ((rx * 2 + 1) * (ry * 2 + 1)) as u64) as u16;
                    if depth == 8 {
                        expected.push(value as u8);
                    } else {
                        expected.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
            offset += pw * ph;
        }
        assert_eq!(f.data, expected, "{w}x{h} {depth}");
    }
}
