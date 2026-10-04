//! Explicit external oracle, never part of normal test or production execution.
use fvid_media::{
    owned_frame::GeometryFrame,
    owned_vignette::{Clock, Vignette},
};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let oracle =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit reference executable required");
    let mut count = 0;
    for (format, subsampling) in [
        ("rgb24", None),
        ("yuv444p", Some([1, 1])),
        ("yuv422p", Some([2, 1])),
        ("yuv420p", Some([2, 2])),
        ("yuv411p", Some([4, 1])),
        ("yuv410p", Some([4, 4])),
        ("yuv440p", Some([1, 2])),
    ] {
        for sar in [1., 2., 0.5] {
            for options in [
                "",
                "dither=0",
                "mode=backward",
                "angle=PI/3:x0=3.7:y0=5.2",
                "aspect=2:dither=0",
                "angle=0",
                "angle=PI/4:eval=frame:x0=w/2+n:y0=h/2+t*10",
                "angle=n/10",
                "enable='between(n,1,2)'",
                "angle=r*tb/2:eval=frame:x0=pts/2:y0=h/2",
            ] {
                let filter = Vignette::parse(options).unwrap();
                let mut input = Vec::new();
                let mut expected = Vec::new();
                for n in 0..4 {
                    let size = if let Some([sx, sy]) = subsampling {
                        16 * 12 + 2 * 16usize.div_ceil(sx) * 12usize.div_ceil(sy)
                    } else {
                        16 * 12 * 3
                    };
                    let data: Vec<u8> = (0..size)
                        .map(|i| ((i * 37 + n * 23 + 101) % 256) as u8)
                        .collect();
                    input.extend_from_slice(&data);
                    let mut frame = GeometryFrame {
                        width: 16,
                        height: 12,
                        subsampling,
                        data,
                    };
                    filter
                        .apply_clock(
                            &mut frame,
                            8,
                            Clock {
                                n: n as u64,
                                t: Some(n as f64 / 25.),
                                pts: Some(n as f64),
                                rate: Some(25.),
                                time_base: Some(1. / 25.),
                                sample_aspect: sar,
                            },
                        )
                        .unwrap();
                    expected.extend(frame.data);
                }
                let graph = format!("setsar={sar},vignette={options}");
                let mut child = Command::new(&oracle)
                    .args([
                        "-v",
                        "error",
                        "-f",
                        "rawvideo",
                        "-pixel_format",
                        format,
                        "-video_size",
                        "16x12",
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
                        "{format} sar={sar} {options} byte{at}: ref{} own{}",
                        out.stdout[at], expected[at]
                    );
                }
                if format == "yuv420p" && sar == 1. && options.is_empty() {
                    std::fs::write("/tmp/fvid-vignette.reference.raw", &out.stdout).unwrap();
                }
                if format == "rgb24" && sar == 1. && options == "angle=PI/3:x0=3.7:y0=5.2" {
                    std::fs::write("/tmp/fvid-vignette-fractional.reference.raw", &out.stdout)
                        .unwrap();
                }
                count += 1;
            }
        }
    }
    println!("vignette: {count} exact four-frame comparisons passed");
}
