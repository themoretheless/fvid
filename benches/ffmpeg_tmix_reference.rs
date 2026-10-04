//! Explicit benchmark oracle only; never executed in ordinary tests or production.
use fvid_media::{owned_frame::GeometryFrame, owned_tmix::TemporalMix};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let executable =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit benchmark oracle required");
    let mut cases = 0;
    let mut reference_overflows = 0;
    for (format, sub, depth) in [
        ("yuv420p", [2, 2], 8),
        ("yuv444p", [1, 1], 8),
        ("yuv420p10le", [2, 2], 10),
        ("yuv444p10le", [1, 1], 10),
        ("yuv420p16le", [2, 2], 16),
        ("yuv444p16le", [1, 1], 16),
    ] {
        for args in [
            "",
            "frames=2",
            "frames=4:weights=1 2 3 4",
            "frames=3:weights=1|2",
            "frames=3:weights=1 -0.5 2:scale=0.5",
            "frames=3:planes=1",
            "frames=3:planes=0",
            "frames=1:scale=2",
            "frames=3:weights=0 0 0",
            "frames=1024",
        ] {
            let filter = TemporalMix::parse(args).unwrap();
            let mut input = Vec::new();
            let mut expected = Vec::new();
            for n in 0..8usize {
                let count = 15 + 2 * 5usize.div_ceil(sub[0]) * 3usize.div_ceil(sub[1]);
                let mut data = Vec::new();
                for i in 0..count {
                    let v = if n % 3 == 0 {
                        (i * 173 + n * 31) % (1usize << depth)
                    } else {
                        (i * 17) % 128
                    } as u16;
                    if depth == 8 {
                        data.push(v as u8);
                    } else {
                        data.extend(v.to_le_bytes());
                    }
                }
                input.extend(&data);
                let mut frame = GeometryFrame {
                    width: 5,
                    height: 3,
                    subsampling: Some(sub),
                    data,
                };
                filter.apply(&mut frame, depth, n as u64).unwrap();
                expected.extend(frame.data);
            }
            let graph = if args.is_empty() {
                "tmix".into()
            } else {
                format!("tmix={args}")
            };
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
                    "25",
                    "-i",
                    "pipe:0",
                    "-vf",
                    &graph,
                    "-frames:v",
                    "8",
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
            if depth == 8 && args == "frames=1024" {
                let frame_bytes = input.len() / 8;
                assert_eq!(&expected[..frame_bytes], &input[..frame_bytes]);
                assert_eq!(input[1], 173);
                assert_eq!(out.stdout[1], 45, "known 16-bit reference sum overflow");
                assert_ne!(out.stdout, expected);
                reference_overflows += 1;
            } else {
                assert_eq!(out.stdout, expected, "{format} {args}");
                cases += 1;
            }
        }
    }
    let fixture=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/tmix-ramp-8.y4m");
    let out=Command::new(&executable).args(["-v","error","-i"]).arg(&fixture)
        .args(["-vf","tmix=frames=1024","-frames:v","1","-threads","1","-f","rawvideo","-pix_fmt","yuv420p","pipe:1"])
        .output().unwrap();
    assert!(out.status.success(),"{}",String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.stdout,[vec![0;16],vec![36;4],vec![22;4]].concat(),"stored fixture must reproduce exact reference overflow");
    println!(
        "tmix: {cases} exact pixel comparisons passed; {reference_overflows} reference sum overflows reproduced with correct owned output"
    );
}
