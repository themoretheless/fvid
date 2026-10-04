//! Explicit benchmark oracle; production and ordinary tests never execute it.
use fvid_media::{owned_eq::EqualizerProgram, owned_frame::GeometryFrame};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let executable =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit benchmark oracle required");
    let mut cases = 0;
    for (format, sub) in [
        ("yuv420p", [2, 2]),
        ("yuv422p", [2, 1]),
        ("yuv444p", [1, 1]),
    ] {
        for (rate, num, den) in [("25", 25u64, 1u64), ("30000/1001", 30000, 1001)] {
            for args in [
                "brightness=n/10:eval=frame",
                "brightness=2.5*t:eval=frame",
                "brightness=n/10",
                "contrast=1+n/10:eval=frame",
                "saturation=n/2:eval=frame",
                "gamma=1+n/10:gamma_weight=0.5:eval=frame",
                "gamma_r=1+n/10:gamma_b=1+n/5:eval=frame",
                "brightness='if(gte(n,4),0.5,0)':eval=frame",
                "brightness=t:eval=init",
            ] {
                let program = EqualizerProgram::parse(args).unwrap();
                let mut input = Vec::new();
                let mut expected = Vec::new();
                for n in 0..9usize {
                    let count = 15 + 2 * 5usize.div_ceil(sub[0]) * 3usize.div_ceil(sub[1]);
                    let data = (0..count)
                        .map(|i| ((i * 173 + n * 31) % 256) as u8)
                        .collect::<Vec<_>>();
                    input.extend(&data);
                    let mut frame = GeometryFrame {
                        width: 5,
                        height: 3,
                        subsampling: Some(sub),
                        data,
                    };
                    program
                        .apply(
                            &mut frame,
                            8,
                            n as u64,
                            Some(n as f64 * den as f64 / num as f64),
                        )
                        .unwrap();
                    expected.extend(frame.data);
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
                        "5x3",
                        "-framerate",
                        rate,
                        "-i",
                        "pipe:0",
                        "-vf",
                        &format!("eq={args}"),
                        "-frames:v",
                        "9",
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
                assert_eq!(out.stdout, expected, "{format} {rate} {args}");
                cases += 1;
            }
        }
    }
    println!("eq timeline: {cases} exact pixel comparisons passed");
}
