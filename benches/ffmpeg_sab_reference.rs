//! Explicit external pixel oracle; ordinary tests never run it.
use fvid_media::{owned_frame::GeometryFrame, owned_sab::Sab};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let oracle =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit oracle executable required");
    let mut count = 0;
    for (width, height) in [(16usize, 12usize), (17, 13), (32, 24), (8, 8)] {
        for (format, sampling) in [
            ("yuv444p", [1, 1]),
            ("yuv422p", [2, 1]),
            ("yuv420p", [2, 2]),
            ("yuv411p", [4, 1]),
            ("yuv410p", [4, 4]),
        ] {
            for options in [
                "",
                "lr=0.1",
                "lr=0.5",
                "lr=4",
                "lpfr=0.1",
                "lpfr=2",
                "ls=0.1",
                "ls=10",
                "ls=100",
                "lr=4:lpfr=2:ls=100",
                "lr=4:lpfr=0.1:ls=100",
                "lr=3:lpfr=0.1:ls=0.1",
                "ls=3.5:cpfr=2:cs=40",
                "lr=1.5:lpfr=0.8:ls=5:cr=2:cpfr=0.5:cs=30",
                "lr=2:lpfr=1.3:ls=7:cr=-0.9:cpfr=-0.5:cs=0",
                "luma_radius=1.7:luma_pre_filter_radius=1.3:luma_strength=9:chroma_radius=1.2:chroma_pre_filter_radius=0.7:chroma_strength=2",
                "0.8:0.4:3:1.2:0.9:5",
                "enable='between(n,1,2)'",
                "enable='eq(n,2)*eq(w,16)*eq(h,12)'",
            ] {
                let filter = Sab::parse(options).unwrap();
                let mut input = Vec::new();
                let mut expected = Vec::new();
                let size =
                    width * height + 2 * width.div_ceil(sampling[0]) * height.div_ceil(sampling[1]);
                for n in 0..4 {
                    let data: Vec<u8> = (0..size)
                        .map(|i| ((i * 37 + n * 23 + 101) % 256) as u8)
                        .collect();
                    input.extend_from_slice(&data);
                    let mut frame = GeometryFrame {
                        width,
                        height,
                        subsampling: Some(sampling),
                        data,
                    };
                    filter
                        .apply(&mut frame, 8, n as u64, Some(n as f64 / 25.))
                        .unwrap();
                    expected.extend(frame.data);
                }
                let graph = format!("sab={options}");
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
                    && (options.is_empty() || options == "lr=4:lpfr=2:ls=100")
                {
                    let name = if options.is_empty() { "blur" } else { "smooth" };
                    std::fs::write(format!("/tmp/fvid-sab-{name}.reference.raw"), &out.stdout)
                        .unwrap();
                }
                count += 1;
            }
        }
    }
    println!("sab: {count} exact four-frame pixel comparisons passed");
}
