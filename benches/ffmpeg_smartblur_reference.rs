//! Explicit external pixel oracle; ordinary tests never run it.
use fvid_media::{owned_frame::GeometryFrame, owned_smartblur::SmartBlur};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let oracle =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit oracle executable required");
    let mut count = 0;
    for (width, height) in [(16usize, 12usize), (17, 13), (32, 24), (5, 3)] {
        for (format, sampling) in [
            ("yuv444p", [1, 1]),
            ("yuv422p", [2, 1]),
            ("yuv420p", [2, 2]),
            ("yuv411p", [4, 1]),
            ("yuv410p", [4, 4]),
            ("yuv440p", [1, 2]),
            ("gray", [1, 1]),
            ("yuva444p", [1, 1]),
            ("yuva422p", [2, 1]),
            ("yuva420p", [2, 2]),
        ] {
            for options in [
                "",
                "lr=0.1",
                "lr=0.5",
                "lr=5",
                "ls=-1",
                "ls=-0.5",
                "lt=12",
                "lt=-12",
                "lr=1.5:ls=0.8:lt=3:cr=2:cs=-0.4:ct=-3",
                "ls=0.0001",
                "lr=5:ls=-1:lt=-30",
                "cr=0.1:cs=0:ct=-31",
                "luma_radius=1.7:luma_strength=0.5:luma_threshold=8:chroma_strength=0.1",
                "enable='between(n,1,2)'",
                "0.8:-0.1:-3:1.2:0.9:5",
                "ar=3:as=-1:at=-7",
            ] {
                let filter = SmartBlur::parse(options).unwrap();
                let mut input = Vec::new();
                let mut expected = Vec::new();
                let yuv_size =
                    width * height + 2 * width.div_ceil(sampling[0]) * height.div_ceil(sampling[1]);
                let alpha = format.starts_with("yuva");
                let gray = format == "gray";
                let size = if gray {
                    width * height
                } else {
                    yuv_size + if alpha { width * height } else { 0 }
                };
                for n in 0..4 {
                    let mut data: Vec<u8> = (0..size)
                        .map(|i| ((i * 37 + n * 23 + 101) % 256) as u8)
                        .collect();
                    input.extend_from_slice(&data);
                    if gray {
                        filter
                            .apply_plane(
                                &mut data,
                                width,
                                height,
                                8,
                                0,
                                n as u64,
                                Some(n as f64 / 25.),
                            )
                            .unwrap();
                        expected.extend(data);
                    } else {
                        let mut frame = GeometryFrame {
                            width,
                            height,
                            subsampling: Some(sampling),
                            data: data[..yuv_size].to_vec(),
                        };
                        filter
                            .apply(&mut frame, 8, n as u64, Some(n as f64 / 25.))
                            .unwrap();
                        expected.extend(frame.data);
                        if alpha {
                            filter
                                .apply_plane(
                                    &mut data[yuv_size..],
                                    width,
                                    height,
                                    8,
                                    2,
                                    n as u64,
                                    Some(n as f64 / 25.),
                                )
                                .unwrap();
                            expected.extend_from_slice(&data[yuv_size..]);
                        }
                    }
                }
                let graph = format!("smartblur={options}");
                let size_arg = format!("{width}x{height}");
                let mut child = Command::new(&oracle)
                    .args([
                        "-v",
                        "error",
                        "-f",
                        "rawvideo",
                        "-pixel_format",
                        format,
                        "-video_size",
                        &size_arg,
                        "-framerate",
                        "25",
                        "-i",
                        "pipe:0",
                        "-vf",
                        &graph,
                        "-threads",
                        "1",
                        "-frames:v",
                        "4",
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
                    "{format} {size_arg} {options}: {}",
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
                        "{format} {size_arg} {options} byte{at} ref{} own{}",
                        out.stdout[at], expected[at]
                    );
                }
                if width == 16
                    && height == 12
                    && format == "yuv420p"
                    && (options.is_empty() || options == "lr=5:ls=-1:lt=-30")
                {
                    let name = if options.is_empty() {
                        "blur"
                    } else {
                        "sharpen"
                    };
                    std::fs::write(
                        format!("/tmp/fvid-smartblur-{name}.reference.raw"),
                        &out.stdout,
                    )
                    .unwrap();
                }
                count += 1;
            }
        }
    }
    println!("smartblur: {count} exact four-frame pixel comparisons passed");
}
