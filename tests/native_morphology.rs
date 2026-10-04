use fvid::{
    native_geometry::GeometryFrame,
    native_morphology::{Morphology, MorphologyKind},
};
const KINDS: [MorphologyKind; 2] = [MorphologyKind::Dilation, MorphologyKind::Erosion];
fn frame(depth: u8, sx: usize, sy: usize, rgb: bool) -> GeometryFrame {
    let max = (1u32 << depth) - 1;
    let count = if rgb { 192 } else { 64 + 128 / (sx * sy) };
    let data = (0..count)
        .flat_map(|i| {
            let v = ((i * i * 31 + i * 17) as u32 % (max + 1)) as u16;
            if depth == 8 {
                vec![v as u8]
            } else {
                v.to_le_bytes().to_vec()
            }
        })
        .collect();
    GeometryFrame {
        width: 8,
        height: 8,
        subsampling: if rgb { None } else { Some([sx, sy]) },
        data,
    }
}
#[test]
fn masks_thresholds_and_rejection_are_preserved() {
    for kind in KINDS {
        for depth in [8, 10, 16] {
            let mut f = frame(depth, 2, 2, false);
            let original = f.data.clone();
            Morphology::parse(kind, "coordinates=0")
                .unwrap()
                .apply(&mut f, depth)
                .unwrap();
            assert_eq!(f.data, original);
            Morphology::parse(kind, "threshold0=0:threshold1=0:threshold2=0")
                .unwrap()
                .apply(&mut f, depth)
                .unwrap();
            assert_eq!(f.data, original);
            Morphology::parse(kind, "threshold0=3:threshold1=0:threshold2=0")
                .unwrap()
                .apply(&mut f, depth)
                .unwrap();
            let bytes = if depth == 8 { 1 } else { 2 };
            assert_eq!(&f.data[64 * bytes..], &original[64 * bytes..]);
            let samples = |data: Vec<u8>| {
                data.chunks_exact(bytes)
                    .map(|b| {
                        if bytes == 1 {
                            u16::from(b[0])
                        } else {
                            u16::from_le_bytes([b[0], b[1]])
                        }
                    })
                    .collect::<Vec<_>>()
            };
            for (before, after) in samples(original).into_iter().zip(samples(f.data.clone())) {
                match kind {
                    MorphologyKind::Dilation => assert!(after >= before && after - before <= 3),
                    MorphologyKind::Erosion => assert!(before >= after && before - after <= 3),
                }
            }
            f.data.pop();
            let before = f.data.clone();
            assert!(
                Morphology::parse(kind, "")
                    .unwrap()
                    .apply(&mut f, depth)
                    .is_err()
            );
            assert_eq!(f.data, before);
        }
    }
    for args in [
        "coordinates=256",
        "threshold0=-1",
        "threshold1=65536",
        "threshold2=NaN",
        "what=3",
        "1:2:3:4:5:6",
    ] {
        assert!(Morphology::parse(MorphologyKind::Dilation, args).is_err());
    }
    for kind in KINDS {
        let mut f = GeometryFrame {
            width: 1,
            height: 1,
            subsampling: Some([2, 2]),
            data: vec![17, 23, 99],
        };
        Morphology::parse(kind, "")
            .unwrap()
            .apply(&mut f, 8)
            .unwrap();
        assert_eq!(f.data, [17, 23, 99]);
        let mut f = frame(10, 1, 1, false);
        f.data[..2].copy_from_slice(&1024u16.to_le_bytes());
        let before = f.data.clone();
        assert!(
            Morphology::parse(kind, "")
                .unwrap()
                .apply(&mut f, 10)
                .is_err()
        );
        assert_eq!(before, f.data);
    }
}
#[test]
fn cli_and_api_use_owned_path() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in [
        "tests/fixtures/video.mp4",
        "tests/fixtures/hevc/main10-ipb.mp4",
    ] {
        let path = root.join(file);
        let base = fvid::native_media::decode_video(&path).unwrap();
        let request = fvid::media_info::DecodeTransform {
            dilation: Some("coordinates=170:threshold0=16".into()),
            erosion: Some("threshold1=0".into()),
            ..Default::default()
        };
        let actual = fvid::native_media::decode_video_request(&path, &request).unwrap();
        assert_eq!(actual.backend, "fvid");
        assert_eq!(actual.video_frames, base.video_frames);
        assert_eq!(actual.pixel_format, base.pixel_format);
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args([
                "media",
                "decode",
                path.to_str().unwrap(),
                "--erosion",
                "threshold1=0",
                "--dilation",
                "coordinates=170:threshold0=16",
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
        assert_eq!(value["video_frames"], base.video_frames);
        assert_eq!(
            fvid::media::decode_video_transformed(&path, request)
                .unwrap()
                .backend,
            "fvid"
        );
    }
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args([
            "media",
            "decode",
            "missing.mp4",
            "--erosion",
            "",
            "--erosion",
            "",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("duplicate erosion"));
}

#[test]
fn constant_options_preserve_owned_pixels_and_api() {
    for kind in KINDS {
        for depth in [8, 10, 16] {
            for (expression, literal) in [
                ("coordinates=2^7:threshold0=PI:threshold1=5/2:threshold2=7/2", "128:3:2:4"),
                ("coordinates=default:threshold0=max:threshold1=min:threshold2=default", "255:65535:0:65535"),
                ("coordinates=0xaa:threshold0=0x10", "170:16"),
            ] {
                let mut actual = frame(depth, 2, 2, false);
                let mut expected = frame(depth, 2, 2, false);
                Morphology::parse(kind, expression).unwrap().apply(&mut actual, depth).unwrap();
                Morphology::parse(kind, literal).unwrap().apply(&mut expected, depth).unwrap();
                assert_eq!(actual.data, expected.data);
            }
        }
        for invalid in ["coordinates=n", "coordinates=255.1", "threshold0=65535.1", "threshold0=1/0"] {
            assert!(Morphology::parse(kind, invalid).is_err(), "{invalid}");
        }
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.mp4");
    let stats = fvid::media::decode_video_transformed(&path, fvid::media::DecodeTransform {
        dilation: Some("coordinates=2^7:threshold0=PI".into()),
        erosion: Some("coordinates=default:threshold0=5/2".into()),
        ..Default::default()
    }).unwrap();
    assert_eq!(stats.backend, "fvid");
    assert_eq!(stats.video_frames, 25);
}
