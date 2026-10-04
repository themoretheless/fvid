//! Explicit reference benchmark. No external executable in ordinary tests.
use fvid_media::{owned_bitplanenoise::BitPlaneNoise, owned_frame::GeometryFrame};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let oracle = std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit reference required");
    let mut count = 0;
    for (w, h) in [(16usize, 12usize), (17, 13), (16, 1)] {
        for depth in [8u8, 9, 10, 12, 14, 16] {
            for (base, sampling) in [
                ("yuv444p", [1, 1]),
                ("yuv422p", [2, 1]),
                ("yuv420p", [2, 2]),
            ] {
                let format = if depth == 8 {
                    base.to_owned()
                } else {
                    format!("{base}{depth}le")
                };
                for args in [
                    "",
                    "bitplane=1:filter=1",
                    "bitplane=8:filter=1",
                    "bitplane=16:filter=1",
                    "bitplane=3:filter=1:enable='eq(n,2)'",
                ] {
                    let program = BitPlaneNoise::parse(args).unwrap();
                    let mut input = Vec::new();
                    let mut expected = Vec::new();
                    let mut metadata = Vec::new();
                    let size = w * h + 2 * w.div_ceil(sampling[0]) * h.div_ceil(sampling[1]);
                    for n in 0..4usize {
                        let mut data = Vec::new();
                        for i in 0..size {
                            let v = ((i * 37 + n * 23 + 101) % (1usize << depth)) as u16;
                            if depth == 8 {
                                data.push(v as u8)
                            } else {
                                data.extend(v.to_le_bytes())
                            }
                        }
                        input.extend(&data);
                        let mut frame = GeometryFrame {
                            width: w,
                            height: h,
                            subsampling: Some(sampling),
                            data,
                        };
                        if let Some(report) = program
                            .apply(&mut frame, depth, n as u64, Some(n as f64 / 25.))
                            .unwrap()
                        {
                            metadata.extend(
                                report
                                    .metadata()
                                    .into_iter()
                                    .map(|(k, v)| format!("{k}={v}")),
                            );
                        }
                        expected.extend(frame.data);
                    }
                    let path = std::env::temp_dir().join(format!(
                        "fvid-bitplane-reference-{}.txt",
                        std::process::id()
                    ));
                    let graph = format!(
                        "bitplanenoise={args},metadata=print:file={}",
                        path.display()
                    );
                    let mut child = Command::new(&oracle)
                        .args([
                            "-v",
                            "error",
                            "-f",
                            "rawvideo",
                            "-pixel_format",
                            &format,
                            "-video_size",
                            &format!("{w}x{h}"),
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
                            &format,
                            "pipe:1",
                        ])
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                        .unwrap();
                    child.stdin.take().unwrap().write_all(&input).unwrap();
                    let result = child.wait_with_output().unwrap();
                    assert!(
                        result.status.success(),
                        "{}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                    assert_eq!(result.stdout, expected, "{format} {w}x{h} {args}");
                    let text = std::fs::read_to_string(&path).unwrap();
                    let actual: Vec<_> = text
                        .lines()
                        .filter(|l| l.starts_with("lavfi.bitplanenoise."))
                        .map(str::to_owned)
                        .collect();
                    assert_eq!(actual, metadata, "metadata {format} {args}");
                    std::fs::remove_file(path).unwrap();
                    if w == 16
                        && h == 12
                        && depth == 8
                        && base == "yuv420p"
                        && args == "bitplane=1:filter=1"
                    {
                        std::fs::write("/tmp/fvid-bitplanenoise.expected.raw", &result.stdout)
                            .unwrap();
                        std::fs::write(
                            "/tmp/fvid-bitplanenoise.expected.txt",
                            actual.join("\n") + "\n",
                        )
                        .unwrap();
                    }
                    count += 1;
                }
            }
        }
    }
    println!("bitplanenoise: {count} exact four-frame pixel and metadata comparisons passed");
}
