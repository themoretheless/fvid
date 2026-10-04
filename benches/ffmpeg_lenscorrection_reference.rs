//! Explicit external reference. Ordinary tests use saved synthetic bytes.
use fvid_media::{
    owned_frame::GeometryFrame,
    owned_lenscorrection::{Component, LensCorrection},
};
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
                (
                    "yuv444p",
                    [1, 1],
                    vec![Component::Luma, Component::Cb, Component::Cr],
                ),
                (
                    "yuv422p",
                    [2, 1],
                    vec![Component::Luma, Component::Cb, Component::Cr],
                ),
                (
                    "yuv420p",
                    [2, 2],
                    vec![Component::Luma, Component::Cb, Component::Cr],
                ),
                (
                    "yuv411p",
                    [4, 1],
                    vec![Component::Luma, Component::Cb, Component::Cr],
                ),
                (
                    "yuv410p",
                    [4, 4],
                    vec![Component::Luma, Component::Cb, Component::Cr],
                ),
                (
                    "yuv440p",
                    [1, 2],
                    vec![Component::Luma, Component::Cb, Component::Cr],
                ),
                (
                    "gbrp",
                    [1, 1],
                    vec![Component::Green, Component::Blue, Component::Red],
                ),
                ("gray", [1, 1], vec![Component::Luma]),
                (
                    "yuva444p",
                    [1, 1],
                    vec![
                        Component::Luma,
                        Component::Cb,
                        Component::Cr,
                        Component::Alpha,
                    ],
                ),
                (
                    "gbrap",
                    [1, 1],
                    vec![
                        Component::Green,
                        Component::Blue,
                        Component::Red,
                        Component::Alpha,
                    ],
                ),
            ] {
                if depth > 8 && ["yuv411p", "yuv410p"].contains(&base) {
                    continue;
                }
                if depth > 8 && base == "yuv440p" && ![10, 12].contains(&depth) {
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
                    "k1=.3:k2=.1",
                    "k1=-.3:k2=.1:i=bilinear:fc=0xff0050@.4",
                    "cx=0:cy=1:k1=1:k2=-1",
                    "cx=1:cy=0:k1=-1:k2=1:i=bilinear:fc=AliceBlue",
                    "cx=.3:cy=.8:k1=.1:k2=-.6:i=nearest:fc=red",
                    "k1=1:k2=1:i=64:fc=0x11223344",
                    "k1=-1:k2=-1:i=1:fc=white@0x40",
                    "i=bilinear",
                    "k1=.3:enable='eq(n,2)'",
                    "0.2:0.7:-0.5:0.8:1:blue",
                ] {
                    let filter = LensCorrection::parse(options).unwrap();
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
                        if components.len() == 3 && base.starts_with("yuv") {
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
                                    .apply_plane(
                                        &mut result[offset..offset + size],
                                        pw,
                                        ph,
                                        depth,
                                        component,
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
                            &format!("lenscorrection={options}"),
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
                            "" => Some("identity"),
                            "k1=.3:k2=.1" => Some("warp"),
                            "k1=-.3:k2=.1:i=bilinear:fc=0xff0050@.4" => Some("bilinear"),
                            _ => None,
                        };
                        if let Some(name) = name {
                            std::fs::write(
                                format!("/tmp/fvid-lenscorrection-{name}.reference.raw"),
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
    println!("lenscorrection: {count} exact four-frame comparisons passed");
}
