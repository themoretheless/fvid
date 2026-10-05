//! Explicit curve oracle; production and ordinary tests never execute FFmpeg.
use fvid_media::owned_curves::Curves;
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let executable =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit benchmark oracle required");
    let mut comparisons = 0;
    for (depth, format, channels, planar) in [
        (8, "rgb24", 3, false),
        (16, "rgb48le", 3, false),
        (8, "rgba", 4, false),
        (16, "rgba64le", 4, false),
        (9, "gbrp9le", 3, true),
        (10, "gbrp10le", 3, true),
        (12, "gbrp12le", 3, true),
        (14, "gbrp14le", 3, true),
    ] {
        for interp in ["natural", "pchip"] {
            for args in [
                "",
                "preset=color_negative",
                "preset=cross_process",
                "preset=darker",
                "preset=increase_contrast",
                "preset=lighter",
                "preset=linear_contrast",
                "preset=medium_contrast",
                "preset=negative",
                "preset=strong_contrast",
                "preset=vintage",
                "all='0/0 1/1'",
                "all='0.2/0.1 0.7/0.9'",
                "all='0.2/0.1 0.4/0.8 0.8/0.3'",
                "r='0/1 1/0':g='0.5/0.2':master='0/0 0.4/0.7 1/1'",
                "all='0/0 0.25/0 0.5/0.5 0.75/1 1/1'",
                "psfile=tests/fixtures/playback-errors/curves-negative.acv",
                "all='0/0 1/1':psfile=tests/fixtures/playback-errors/curves-negative.acv:preset=vintage:red='0/1 1/0'",
            ] {
                let args = if args.is_empty() {
                    format!("interp={interp}")
                } else {
                    format!("{args}:interp={interp}")
                };
                let mut input = Vec::new();
                for i in 0..256usize {
                    for v in [i, 255 - i, (i * 37) % 256] {
                        let v = (v * ((1usize << depth) - 1) / 255) as u16;
                        if depth == 8 {
                            input.push(v as u8);
                        } else {
                            input.extend(v.to_le_bytes());
                        }
                    }
                    if channels == 4 {
                        let v = ((i * 11) % 256 * ((1usize << depth) - 1) / 255) as u16;
                        if depth == 8 {
                            input.push(v as u8);
                        } else {
                            input.extend(v.to_le_bytes());
                        }
                    }
                }
                let mut expected = input.clone();
                Curves::parse(&args)
                    .unwrap()
                    .apply_rgb(&mut expected, depth, channels)
                    .unwrap();
                if planar {
                    input = planarize(&input);
                    expected = planarize(&expected);
                }
                let mut child = Command::new(&executable)
                    .args([
                        "-v",
                        "error",
                        "-f",
                        "rawvideo",
                        "-pixel_format",
                        format,
                        "-video_size",
                        "16x16",
                        "-i",
                        "pipe:0",
                        "-vf",
                        &format!("curves={args}"),
                        "-frames:v",
                        "1",
                        "-threads",
                        "1",
                        "-f",
                        "rawvideo",
                        "-pix_fmt",
                        format,
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
                if out.stdout != expected {
                    let at = out
                        .stdout
                        .iter()
                        .zip(&expected)
                        .position(|(a, b)| a != b)
                        .unwrap();
                    panic!(
                        "{depth} {args}: mismatch byte {at}: reference {} owned {}",
                        out.stdout[at], expected[at]
                    );
                }
                if !planar
                    && channels == 3
                    && args == "all='0/0 0.25/0 0.5/0.5 0.75/1 1/1':interp=pchip"
                {
                    std::fs::write(
                        format!("/tmp/fvid-curves-plateau-{depth}.reference.raw"),
                        &out.stdout,
                    )
                    .unwrap();
                }
                comparisons += 1;
            }
        }
    }
    yuv_reference(&executable);
    comparisons += 2;
    println!("curves: {comparisons} exact RGB pixel comparisons passed");
}

fn planarize(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for channel in [1, 2, 0] {
        for pixel in data.as_chunks::<6>().0 {
            output.extend_from_slice(&pixel[channel * 2..channel * 2 + 2]);
        }
    }
    output
}

fn yuv_reference(executable: &std::ffi::OsStr) {
    use fvid_media::{owned_frame::GeometryFrame, owned_yuv_rgb::Matrix};
    for (depth, format) in [(8, "yuv444p"), (10, "yuv444p10le")] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/playback-errors/curves-gray-{depth}.y4m"
        ));
        let source = std::fs::read(&path).unwrap();
        let start = source.iter().position(|&b| b == b'\n').unwrap() + 1;
        let bytes = if depth == 8 { 1 } else { 2 };
        let filter = Curves::parse("negative").unwrap();
        let mut expected = Vec::new();
        for (n, chunk) in source[start..].chunks_exact(6 + 48 * bytes).enumerate() {
            let mut frame = GeometryFrame {
                width: 4,
                height: 4,
                subsampling: Some([1, 1]),
                data: chunk[6..].to_vec(),
            };
            filter
                .apply_yuv(
                    &mut frame,
                    depth,
                    false,
                    Matrix::Bt601,
                    n as u64,
                    Some(n as f64 / 25.),
                )
                .unwrap();
            expected.extend(frame.data);
        }
        let child = Command::new(executable)
            .args(["-v", "error", "-i"])
            .arg(path)
            .args([
                "-vf",
                &format!("format=rgb48le,curves=negative,format={format}"),
                "-frames:v",
                "4",
                "-f",
                "rawvideo",
                "-pix_fmt",
                format,
                "pipe:1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.stdout, expected, "limited rgb48 YUV {depth}");
        std::fs::write(
            format!("/tmp/fvid-curves-gray-{depth}.reference.raw"),
            &out.stdout,
        )
        .unwrap();
    }
}
