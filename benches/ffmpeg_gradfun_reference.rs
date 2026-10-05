//! Explicit external pixel oracle; ordinary tests never run it.
use fvid_media::{owned_frame::GeometryFrame, owned_gradfun::GradFun};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let oracle =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit oracle executable required");
    let mut count = 0;
    let mut overflow_cases = 0;
    for (width, height) in [
        (96usize, 80usize),
        (65, 65),
        (17, 19),
        (9, 9),
        (8, 8),
        (97, 83),
    ] {
        for (format, sampling) in [
            ("yuv444p", [1, 1]),
            ("gbrp", [1, 1]),
            ("gray", [1, 1]),
            ("yuv422p", [2, 1]),
            ("yuv420p", [2, 2]),
            ("yuv411p", [4, 1]),
            ("yuv410p", [4, 4]),
            ("yuv440p", [1, 2]),
        ] {
            for options in [
                "",
                "radius=4",
                "strength=64:radius=4",
                "0.51:4",
                "strength=5:radius=7",
                "strength=2:radius=32",
                "enable='eq(n,2)'",
            ] {
                let filter = GradFun::parse(options).unwrap();
                let mut input = Vec::new();
                let mut expected = Vec::new();
                let size = if format == "gray" {
                    width * height
                } else {
                    width * height + 2 * width.div_ceil(sampling[0]) * height.div_ceil(sampling[1])
                };
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
                    if format == "gray" {
                        filter
                            .apply_plane(
                                &mut frame.data,
                                width,
                                height,
                                8,
                                [1, 1],
                                n as u64,
                                Some(n as f64 / 25.),
                            )
                            .unwrap();
                    } else {
                        filter
                            .apply(&mut frame, 8, n as u64, Some(n as f64 / 25.))
                            .unwrap();
                    }
                    expected.extend(frame.data);
                }
                let graph = format!("gradfun={options}");
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
                let mut stdin = child.stdin.take().unwrap();
                let writer = std::thread::spawn(move || stdin.write_all(&input).unwrap());
                let out = child.wait_with_output().unwrap();
                writer.join().unwrap();
                assert!(
                    out.status.success(),
                    "{format} {size_arg} {options}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                if out.stdout != expected && width == 9 && height == 9 && options == "0.51:4" {
                    let (wrapped, overflows) = legacy_overflow_model(format, sampling);
                    assert!(overflows > 0);
                    assert_eq!(out.stdout, wrapped, "32-bit overflow reproduction {format}");
                    if format == "yuv444p" {
                        std::fs::write("/tmp/fvid-gradfun-overflow-safe.raw", &expected).unwrap();
                        std::fs::write("/tmp/fvid-gradfun-overflow-legacy.raw", &out.stdout)
                            .unwrap();
                    }
                    overflow_cases += 1;
                    continue;
                }
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
                if width == 96
                    && height == 80
                    && format == "yuv420p"
                    && (options.is_empty() || options == "strength=64:radius=4")
                {
                    let name = if options.is_empty() {
                        "default"
                    } else {
                        "strong"
                    };
                    std::fs::write(
                        format!("/tmp/fvid-gradfun-{name}.reference.raw"),
                        &out.stdout,
                    )
                    .unwrap();
                }
                if width == 65
                    && height == 65
                    && format == "yuv444p"
                    && options == "strength=2:radius=32"
                {
                    std::fs::write("/tmp/fvid-gradfun-edge.reference.raw", &out.stdout).unwrap();
                }
                count += 1;
            }
        }
    }
    println!(
        "gradfun: {count} exact four-frame pixel comparisons and {overflow_cases} independently reproduced legacy overflow cases passed"
    );
}

// Independent reproduction of the 9x9/radius=4 no-window-update corner.
// All source data is synthetic. This is diagnostic, never a production path.
fn legacy_overflow_model(format: &str, sampling: [usize; 2]) -> (Vec<u8>, usize) {
    let mut memory = [0u16; 72];
    let mut out = Vec::new();
    let mut failures = 0;
    let dimensions = if format == "gray" {
        vec![(9usize, 9usize)]
    } else {
        vec![
            (9, 9),
            (9usize.div_ceil(sampling[0]), 9usize.div_ceil(sampling[1])),
            (9usize.div_ceil(sampling[0]), 9usize.div_ceil(sampling[1])),
        ]
    };
    for n in 0..4 {
        let size: usize = dimensions.iter().map(|(w, h)| w * h).sum();
        let frame: Vec<_> = (0..size)
            .map(|i| ((i * 37 + n * 23 + 101) % 256) as u8)
            .collect();
        let mut result = frame.clone();
        let mut offset = 0;
        for &(w, h) in &dimensions {
            if w == 9 && h == 9 {
                memory[16..40].fill(0);
                let mut dc = [0u16; 4];
                for pair in 0..4 {
                    for (x, dc_value) in dc.iter_mut().enumerate() {
                        let at = 40 + pair * 8 + x;
                        let mut value = memory[at - 8];
                        for row in [2 * pair, 2 * pair + 1] {
                            for col in [2 * x, 2 * x + 1] {
                                value = value.wrapping_add(frame[offset + row * w + col] as u16);
                            }
                        }
                        *dc_value = value.wrapping_sub(memory[at]);
                        memory[at] = value;
                    }
                }
                for y in 0..9 {
                    for x in 0..9 {
                        let pixel = i32::from(frame[offset + y * w + x]) << 7;
                        let base = if x / 2 < 2 { 0 } else { dc[x / 2 - 2] as i32 };
                        let delta = base - pixel;
                        let threshold = (32768f32 / 0.51f32) as i32;
                        let product = delta.abs().wrapping_mul(threshold);
                        if delta.abs().checked_mul(threshold).is_none() {
                            failures += 1;
                        }
                        let weight = (127 - (product >> 16)).max(0);
                        let adjustment = weight.wrapping_mul(weight).wrapping_mul(delta) >> 14;
                        let mut dither = 0;
                        for bit in 0..3 {
                            let xb = (x >> bit) & 1;
                            let yb = (y >> bit) & 1;
                            dither += (((xb ^ yb) << 1) | xb) << (5 - 2 * bit);
                        }
                        result[offset + y * w + x] =
                            ((pixel.wrapping_add(adjustment).wrapping_add(dither as i32)) >> 7)
                                .clamp(0, 255) as u8;
                    }
                }
            }
            offset += w * h;
        }
        out.extend(result);
    }
    (out, failures)
}
