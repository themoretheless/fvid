//! Explicit external reference. Ordinary tests use saved synthetic bytes.
use fvid_media::{owned_frame::GeometryFrame, owned_yaepblur::YaepBlur};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() {
    let oracle =
        std::env::var_os("FVID_REFERENCE_FFMPEG").expect("explicit reference executable required");
    let mut count = 0;
    for (w, h) in [(16usize, 12usize), (17, 13), (1, 1)] {
        for depth in [8u8, 9, 10, 12, 14, 16] {
            for (base, sampling, components) in [
                ("yuv444p", [1, 1], vec![0usize, 1usize, 2usize]),
                ("rgb24", [1, 1], vec![0usize, 1usize, 2usize]),
                ("yuv422p", [2, 1], vec![0usize, 1usize, 2usize]),
                ("yuv420p", [2, 2], vec![0usize, 1usize, 2usize]),
                ("yuv411p", [4, 1], vec![0usize, 1usize, 2usize]),
                ("yuv410p", [4, 4], vec![0usize, 1usize, 2usize]),
                ("yuv440p", [1, 2], vec![0usize, 1usize, 2usize]),
                ("gbrp", [1, 1], vec![0usize, 1usize, 2usize]),
                ("gray", [1, 1], vec![0usize]),
                ("yuva444p", [1, 1], vec![0usize, 1usize, 2usize, 3usize]),
                ("gbrap", [1, 1], vec![0usize, 1usize, 2usize, 3usize]),
            ] {
                if depth > 8 && base == "rgb24" {
                    continue;
                }
                if depth > 8 && ["yuv411p", "yuv410p"].contains(&base) {
                    continue;
                }
                if depth > 8 && base == "yuv440p" && depth != 12 {
                    continue;
                }
                if depth == 14 && base == "yuva444p" {
                    continue;
                }
                if [9, 14].contains(&depth) && base == "gbrap" {
                    continue;
                }
                let format = if depth == 8 {
                    base.to_string()
                } else if base == "gray" {
                    format!("gray{depth}le")
                } else {
                    format!("{base}{depth}le")
                };
                for options in [
                    "",
                    "radius=0",
                    "r=1:p=15:s=1",
                    "r=4:p=7:s=1024",
                    "radius=2147483647:planes=15:sigma=2147483647",
                    "r=7:p=0",
                    "r=4:p=2:s=4096",
                    "r=4:p=4:s=4096",
                    "r=4:p=8:s=4096",
                    "3:7:1024",
                    "r=2:p=15:s=100000:enable='eq(n,2)'",
                    "r=2:p=7:s=1024:enable='eq(w,17)*eq(h,13)'",
                ] {
                    let filter = YaepBlur::parse(options).unwrap();
                    let mut input = Vec::new();
                    let mut expected = Vec::new();
                    let dimensions: Vec<_> = (0..components.len())
                        .map(|p| {
                            if (p == 1 || p == 2) && base.starts_with("yuv") {
                                (w.div_ceil(sampling[0]), h.div_ceil(sampling[1]))
                            } else {
                                (w, h)
                            }
                        })
                        .collect();
                    let samples: usize = dimensions.iter().map(|(w, h)| w * h).sum();
                    let bytes = if depth == 8 { 1 } else { 2 };
                    for n in 0..4usize {
                        let data: Vec<u8> = (0..samples)
                            .flat_map(|i| {
                                let v = ((i * 37 + n * 23 + 101) % (1usize << depth)) as u16;
                                if depth == 8 {
                                    vec![v as u8]
                                } else {
                                    v.to_le_bytes().to_vec()
                                }
                            })
                            .collect();
                        input.extend(&data);
                        let mut result = data;
                        if base == "rgb24" {
                            let mut f = GeometryFrame {
                                width: w,
                                height: h,
                                subsampling: None,
                                data: result,
                            };
                            filter
                                .apply(&mut f, depth, n as u64, Some(n as f64 / 25.))
                                .unwrap();
                            result = f.data;
                        } else if components.len() == 3 && base.starts_with("yuv") {
                            let mut f = GeometryFrame {
                                width: w,
                                height: h,
                                subsampling: Some(sampling),
                                data: result,
                            };
                            filter
                                .apply(&mut f, depth, n as u64, Some(n as f64 / 25.))
                                .unwrap();
                            result = f.data;
                        } else {
                            let mut offset = 0;
                            for (&(pw, ph), &component) in dimensions.iter().zip(&components) {
                                let size = pw * ph * bytes;
                                filter
                                    .apply_plane_in_frame(
                                        &mut result[offset..offset + size],
                                        pw,
                                        ph,
                                        depth,
                                        component,
                                        [w, h],
                                        n as u64,
                                        Some(n as f64 / 25.),
                                    )
                                    .unwrap();
                                offset += size;
                            }
                        }
                        expected.extend(result);
                    }
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
                            &format!("yaepblur={options}"),
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
                    let mut stdin = child.stdin.take().unwrap();
                    let writer = std::thread::spawn(move || stdin.write_all(&input).unwrap());
                    let out = child.wait_with_output().unwrap();
                    writer.join().unwrap();
                    assert!(
                        out.status.success(),
                        "{format} {options}: {}",
                        String::from_utf8_lossy(&out.stderr)
                    );
                    if expected != out.stdout {
                        let at = expected
                            .iter()
                            .zip(&out.stdout)
                            .position(|(a, b)| a != b)
                            .unwrap();
                        panic!(
                            "{format} {w}x{h} {options} byte{at}: own {} reference {}",
                            expected[at], out.stdout[at]
                        );
                    }
                    if w == 16 && h == 12 && format == "yuv420p" {
                        let name = match options {
                            "" => Some("default"),
                            "r=4:p=7:s=1024" => Some("strong"),
                            _ => None,
                        };
                        if let Some(name) = name {
                            std::fs::write(
                                format!("/tmp/fvid-yaepblur-{name}.reference.raw"),
                                &out.stdout,
                            )
                            .unwrap();
                        }
                    }
                    count += 1;
                }
            }
        }
    }
    println!("yaepblur: {count} exact four-frame comparisons passed");
    wide_window(&oracle);
    chain(&oracle);
}

fn wide_window(oracle: &std::ffi::OsStr) {
    let w = 257usize;
    let size = w * w;
    let args = "radius=2147483647:sigma=2147483647";
    let mut input = Vec::new();
    for p in 0..3 {
        for i in 0..size {
            let value = if p == 0 {
                if i == size / 2 { 0u16 } else { 65535 }
            } else {
                32768
            };
            input.extend(value.to_le_bytes());
        }
    }
    let mut f = GeometryFrame {
        width: w,
        height: w,
        subsampling: Some([1, 1]),
        data: input.clone(),
    };
    YaepBlur::parse(args)
        .unwrap()
        .apply(&mut f, 16, 0, None)
        .unwrap();
    let mut child = Command::new(oracle)
        .args([
            "-v",
            "error",
            "-f",
            "rawvideo",
            "-pixel_format",
            "yuv444p16le",
            "-video_size",
            "257x257",
            "-i",
            "pipe:0",
            "-vf",
            &format!("yaepblur={args}"),
            "-threads",
            "1",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv444p16le",
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
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let sigma = i32::MAX as u128;
    let mut wrapped = Vec::new();
    let mut safe = Vec::new();
    let mut overflow_pixels = 0;
    for y in 0..w {
        for x in 0..w {
            let lowx = x.saturating_sub(129);
            let highx = (x + 130).min(w);
            let lowy = y.saturating_sub(129);
            let highy = (y + 130).min(w);
            let count = ((highx - lowx) * (highy - lowy)) as u128;
            let hole = lowx <= 128 && 128 < highx && lowy <= 128 && 128 < highy;
            let n = count - u128::from(hole);
            let sum = n * 65535;
            let sq = n * 65535 * 65535;
            let mean = sum / count;
            let variance = (sq - sum * sum / count) / count;
            let bad_variance = (sq - (sum as u64).wrapping_mul(sum as u64) as u128 / count) / count;
            let pixel = if x == 128 && y == 128 { 0 } else { 65535 };
            if sum * sum > u64::MAX as u128 {
                overflow_pixels += 1;
            }
            safe.extend(
                (((sigma * mean + variance * pixel) / (sigma + variance)) as u16).to_le_bytes(),
            );
            wrapped.extend(
                (((sigma * mean + bad_variance * pixel) / (sigma + bad_variance)) as u16)
                    .to_le_bytes(),
            );
        }
    }
    for _ in 0..2 * size {
        safe.extend(32768u16.to_le_bytes());
        wrapped.extend(32768u16.to_le_bytes());
    }
    assert_eq!(
        f.data, safe,
        "wide own statistics match the independent closed-form window model"
    );
    assert_eq!(
        out.stdout, wrapped,
        "wide legacy statistics overflow reproduction"
    );
    assert_ne!(safe, wrapped);
    assert!(overflow_pixels > 0);
    std::fs::write("/tmp/fvid-yaepblur-wide.safe.raw", safe).unwrap();
    std::fs::write("/tmp/fvid-yaepblur-wide.legacy.raw", wrapped).unwrap();
    println!("yaepblur: wide-window overflow independently reproduced at {overflow_pixels} pixels");
}

fn chain(oracle: &std::ffi::OsStr) {
    let mut input = Vec::new();
    let mut expected = Vec::new();
    let mut wrong = Vec::new();
    for n in 0..4usize {
        let data: Vec<_> = (0..288)
            .map(|i| ((i * 37 + n * 23 + 101) % 256) as u8)
            .collect();
        input.extend(&data);
        let mut f = GeometryFrame {
            width: 16,
            height: 12,
            subsampling: Some([2, 2]),
            data: data.clone(),
        };
        let mut other = GeometryFrame {
            width: 16,
            height: 12,
            subsampling: Some([2, 2]),
            data,
        };
        let pixelize = fvid_media::owned_pixelize::Pixelize::parse("4:3:avg:7").unwrap();
        let blur = YaepBlur::parse("r=4:p=7:s=1024").unwrap();
        pixelize.apply(&mut f, 8).unwrap();
        blur.apply(&mut f, 8, n as u64, None).unwrap();
        expected.extend(f.data);
        blur.apply(&mut other, 8, n as u64, None).unwrap();
        pixelize.apply(&mut other, 8).unwrap();
        wrong.extend(other.data);
    }
    let mut child = Command::new(oracle)
        .args([
            "-v",
            "error",
            "-f",
            "rawvideo",
            "-pixel_format",
            "yuv420p",
            "-video_size",
            "16x12",
            "-framerate",
            "25",
            "-i",
            "pipe:0",
            "-vf",
            "pixelize=4:3:avg:7,yaepblur=r=4:p=7:s=1024",
            "-threads",
            "1",
            "-frames:v",
            "4",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
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
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, expected);
    assert_ne!(
        wrong, expected,
        "the synthetic chain must distinguish stage order"
    );
    std::fs::write("/tmp/fvid-yaepblur-chain.reference.raw", &out.stdout).unwrap();
    println!("yaepblur: pixelize chain order qualified against the reference");
}
