//! Explicit benchmark oracle. Never called by ordinary tests or production.
use fvid_media::{owned_frame::GeometryFrame, owned_hue::HueProgram};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let executable =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit benchmark oracle required");
    let mut cases = 0;
    for (format, sub, depth) in [
        ("yuv420p", [2, 2], 8),
        ("yuv444p", [1, 1], 8),
        ("yuv422p", [2, 1], 8),
        ("yuv420p10le", [2, 2], 10),
        ("yuv444p10le", [1, 1], 10),
        ("yuv422p10le", [2, 1], 10),
    ] {
        for (rate, num, den) in [("25", 25u64, 1u64), ("30000/1001", 30000, 1001)] {
            for args in [
                "h=90*n",
                "H=n*PI/2",
                "h=2250*t",
                "s=1-t:b=n/10",
                "h='if(lt(n,3),90,180)':s='if(gte(n,4),0,1)'",
                "s=-10+n*3:b=10-n*4",
            ] {
                let program = HueProgram::parse(args).unwrap();
                let mut input = Vec::new();
                let mut expected = Vec::new();
                for n in 0..9usize {
                    let samples = 15 + 2 * 5usize.div_ceil(sub[0]) * 3usize.div_ceil(sub[1]);
                    let mut data = Vec::new();
                    for i in 0..samples {
                        let value = ((i * 173 + n * 31) % (1usize << depth)) as u16;
                        if depth == 8 {
                            data.push(value as u8);
                        } else {
                            data.extend(value.to_le_bytes());
                        }
                    }
                    input.extend(&data);
                    let mut frame = GeometryFrame {
                        width: 5,
                        height: 3,
                        subsampling: Some(sub),
                        data,
                    };
                    program
                        .at(n as u64, Some(n as f64 * den as f64 / num as f64))
                        .unwrap()
                        .apply(&mut frame, depth)
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
                        &format!("hue={args}"),
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
    println!("hue timeline: {cases} exact pixel comparisons passed");
}
