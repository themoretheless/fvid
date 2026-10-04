use fvid::{native_boxblur::BoxBlur, native_geometry::GeometryFrame};
fn frame(depth: u8, subsampling: [usize; 2]) -> GeometryFrame {
    let (w, h) = (33usize, 25usize);
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
fn invalid_options_storage_and_identity() {
    for args in ["-1", "lp=-1", "cp=-2", "foo=1", "1:2:3:4:5:6:7"] {
        assert!(BoxBlur::parse(args).is_err());
    }
    let mut f = frame(10, [2, 2]);
    let before = f.data.clone();
    BoxBlur::parse("0").unwrap().apply(&mut f, 10).unwrap();
    assert_eq!(f.data, before);
    BoxBlur::parse("2:0").unwrap().apply(&mut f, 10).unwrap();
    assert_eq!(f.data, before);
    assert!(BoxBlur::parse("1024").unwrap().apply(&mut f, 10).is_err());
    assert_eq!(f.data, before);
    f.data.pop();
    let before = f.data.clone();
    assert!(BoxBlur::parse("").unwrap().apply(&mut f, 10).is_err());
    assert_eq!(f.data, before);
}

#[test]
fn decode_cli_and_api_preserve_native_backend_and_intervals() {
    use fvid::media_info::DecodeTransform;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in ["video.mp4", "hevc/main10-ipb.mp4"] {
        let path = root.join(name);
        let request = DecodeTransform {
            boxblur: Some("1:2".into()),
            interval: Some((0, 100000)),
            ..Default::default()
        };
        let direct = fvid::native_media::decode_video_request(&path, &request).unwrap();
        assert_eq!(direct.backend, "fvid");
        assert!(direct.video_frames > 0);
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "decode"])
            .arg(&path)
            .args(["--boxblur", "1:2", "--from", "0", "--to", "0.1"])
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
fn reflection_and_pass_quantization_have_known_pixels() {
    let mut f = GeometryFrame {
        width: 3,
        height: 3,
        subsampling: Some([1, 1]),
        data: [vec![90, 0, 0, 0, 0, 0, 0, 0, 0], vec![40; 18]].concat(),
    };
    BoxBlur::parse("1:1:0").unwrap().apply(&mut f, 8).unwrap();
    assert_eq!(&f.data[..9], &[40, 20, 0, 20, 10, 0, 0, 0, 0]);
    assert_eq!(&f.data[9..], &[40; 18]);
    let mut f = GeometryFrame {
        width: 3,
        height: 3,
        subsampling: Some([1, 1]),
        data: [vec![90, 0, 0, 0, 0, 0, 0, 0, 0], vec![40; 18]].concat(),
    };
    BoxBlur::parse("lr=1:lp=2:cr=0:cp=-1:ar=0:ap=0")
        .unwrap()
        .apply(&mut f, 8)
        .unwrap();
    assert_eq!(&f.data[..9], &[28, 17, 6, 17, 10, 3, 6, 3, 1]);
    assert_eq!(&f.data[9..], &[40; 18]);
}

#[test]
fn expressions_are_not_claimed_by_native_admission() {
    let transform = fvid::media_info::LosslessTransform {
        boxblur: Some("lr=w".into()),
        ..Default::default()
    };
    assert!(!fvid::native_lossless::supports(&transform));
}
