use fvid::{native_chromashift::ChromaShift, native_geometry::GeometryFrame};
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
fn independent_expected_edges_and_invalid_storage() {
    for (edge, expected) in [
        ("smear", vec![1, 1, 2, 4, 4, 5]),
        ("wrap", vec![3, 1, 2, 6, 4, 5]),
    ] {
        let mut f = GeometryFrame {
            width: 3,
            height: 2,
            subsampling: Some([1, 1]),
            data: [vec![9; 6], vec![1, 2, 3, 4, 5, 6], vec![7; 6]].concat(),
        };
        ChromaShift::parse(&format!("1:0:0:0:{edge}"))
            .unwrap()
            .apply(&mut f, 8)
            .unwrap();
        assert_eq!(&f.data[..6], &[9; 6]);
        assert_eq!(&f.data[6..12], expected);
        assert_eq!(&f.data[12..], &[7; 6]);
    }
    for option in [
        "cbh=256",
        "crv=-256",
        "edge=2",
        "cbv=nan",
        "foo=1",
        "1:2:3:4:wrap:5",
    ] {
        assert!(ChromaShift::parse(option).is_err());
    }
    let mut f = frame(10, [2, 2]);
    f.data.pop();
    let before = f.data.clone();
    assert!(
        ChromaShift::parse("cbh=1")
            .unwrap()
            .apply(&mut f, 10)
            .is_err()
    );
    assert_eq!(f.data, before);
}
#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn independent_reference_all_yuv_layouts_and_depths() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    for (layout, sub, depths) in [
        ("420", [2, 2], vec![8, 9, 10, 12, 14, 16]),
        ("422", [2, 1], vec![8, 9, 10, 12, 14, 16]),
        ("444", [1, 1], vec![8, 9, 10, 12, 14, 16]),
        ("440", [1, 2], vec![8, 10, 12]),
        ("411", [4, 1], vec![8]),
        ("410", [4, 4], vec![8]),
    ] {
        for depth in depths {
            let format = if depth == 8 {
                format!("yuv{layout}p")
            } else {
                format!("yuv{layout}p{depth}le")
            };
            for args in [
                "",
                "1:-2:-3:2:smear",
                "-255:255:255:-255:wrap",
                "cbh=3:crv=-2:edge=1",
            ] {
                let mut f = frame(depth, sub);
                let input = f.data.clone();
                ChromaShift::parse(args)
                    .unwrap()
                    .apply(&mut f, depth)
                    .unwrap();
                let mut child = Command::new(&ffmpeg)
                    .args([
                        "-v",
                        "error",
                        "-f",
                        "rawvideo",
                        "-pixel_format",
                        &format,
                        "-video_size",
                        "17x13",
                        "-i",
                        "pipe:0",
                        "-vf",
                        &format!("chromashift={args}"),
                        "-frames:v",
                        "1",
                        "-f",
                        "rawvideo",
                        "-pix_fmt",
                        &format,
                        "pipe:1",
                    ])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(&input).unwrap();
                let out = child.wait_with_output().unwrap();
                assert!(
                    out.status.success(),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                assert_eq!(out.stdout, f.data, "{format} {args}");
            }
        }
    }
}
