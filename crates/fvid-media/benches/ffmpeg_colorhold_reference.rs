//! Explicit RGB reference comparison, never invoked by ordinary tests.
use std::{path::Path, process::Command};
fn main() {
    let root =
        std::env::temp_dir().join(format!("fvid-colorhold-reference-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for depth in [8, 16] {
        let source = root.join(format!("rgb-{depth}.raw"));
        let values: Vec<u16> = (0..64)
            .flat_map(|i| {
                [
                    ((i * 37) % 256) as u16,
                    ((i * 71) % 256) as u16,
                    ((i * 131) % 256) as u16,
                    ((i * 11) % 256) as u16,
                ]
            })
            .collect();
        let input: Vec<u8> = if depth == 8 {
            values.iter().map(|v| *v as u8).collect()
        } else {
            values
                .iter()
                .flat_map(|v| (*v * 257).to_le_bytes())
                .collect()
        };
        std::fs::write(&source, &input).unwrap();
        let format = if depth == 8 { "rgba" } else { "rgba64le" };
        for args in [
            "color=black",
            "color=red:similarity=0.2",
            "color=0x234567:similarity=0.15:blend=0.5",
            "color=white:similarity=1",
            "color=#23456780@0.25:similarity=0.15:blend=0.5",
            "color=RED@0x80:similarity=0.2",
            "color=orange:similarity=0.2:blend=0.25",
        ] {
            let mut actual = input.clone();
            fvid_media::owned_colorhold::ColorHold::parse(args)
                .unwrap()
                .apply_rgb(&mut actual, depth, 4)
                .unwrap();
            let reference =
                Command::new(std::env::var("FVID_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()))
                    .args([
                        "-nostdin",
                        "-v",
                        "error",
                        "-f",
                        "rawvideo",
                        "-pixel_format",
                        format,
                        "-video_size",
                        "8x8",
                        "-i",
                    ])
                    .arg(&source)
                    .args([
                        "-vf",
                        &format!("colorhold={args}"),
                        "-frames:v",
                        "1",
                        "-pix_fmt",
                        format,
                        "-f",
                        "rawvideo",
                        "pipe:1",
                    ])
                    .output()
                    .unwrap();
            assert!(
                reference.status.success(),
                "{}",
                String::from_utf8_lossy(&reference.stderr)
            );
            assert_eq!(actual, reference.stdout, "depth={depth} args={args}");
            println!("depth={depth} args={args} samples=256 matched");
        }
    }
    qualify_yuv_rgb();
    std::fs::remove_dir_all(Path::new(&root)).unwrap();
}

fn qualify_yuv_rgb() {
    use fvid_media::owned_yuv_rgb::{ChromaSampling, Matrix, filter_rgb16_sampled};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
    for depth in [8, 12, 16] {
        let source = root.join(format!("colorize-grid-{depth}.y4m"));
        let bytes = std::fs::read(&source).unwrap();
        let start = bytes.iter().position(|v| *v == b'\n').unwrap() + 7;
        let length = 17 * if depth == 8 { 1 } else { 2 };
        let reference =
            Command::new(std::env::var("FVID_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()))
                .args(["-nostdin", "-v", "error", "-i"])
                .arg(&source)
                .args([
                    "-vf",
                    "format=rgba64le",
                    "-frames:v",
                    "1",
                    "-f",
                    "rawvideo",
                    "pipe:1",
                ])
                .output()
                .unwrap();
        assert!(reference.status.success());
        let expected: Vec<u16> = reference
            .stdout
            .chunks_exact(8)
            .flat_map(|rgba| {
                (0..3).map(move |i| u16::from_le_bytes([rgba[2 * i], rgba[2 * i + 1]]))
            })
            .collect();
        for sampling in [ChromaSampling::Average, ChromaSampling::Point] {
            let mut frame = fvid_media::owned_frame::GeometryFrame {
                width: 3,
                height: 3,
                subsampling: Some([2, 2]),
                data: bytes[start..start + length].to_vec(),
            };
            let locations: [&[usize]; 4] = [&[0, 1, 3, 4], &[2, 5], &[6, 7], &[8]];
            let mut cell = 0;
            let mut actual = vec![0u16; 27];
            filter_rgb16_sampled(&mut frame, depth, false, Matrix::Bt601, sampling, |rgb| {
                for (pixel, index) in rgb.chunks_exact(6).zip(locations[cell]) {
                    for c in 0..3 {
                        actual[index * 3 + c] =
                            u16::from_le_bytes([pixel[2 * c], pixel[2 * c + 1]]);
                    }
                }
                cell += 1;
                Ok(())
            })
            .unwrap();
            if matches!(sampling, ChromaSampling::Point) {
                assert_eq!(actual, expected, "forward stage depth={depth}");
                if std::env::var("FVID_WRITE_SYNTHETIC_REFERENCES").as_deref() == Ok("1") {
                    std::fs::write(
                        root.join(format!("colorhold-rgb-stage-{depth}.raw")),
                        expected
                            .iter()
                            .flat_map(|v| v.to_le_bytes())
                            .collect::<Vec<_>>(),
                    )
                    .unwrap();
                }
            }
            let deltas: Vec<u16> = actual
                .iter()
                .zip(&expected)
                .map(|(a, b)| a.abs_diff(*b))
                .collect();
            println!(
                "forward depth={depth} mode={sampling:?} samples=27 different={} max_delta={} sum_delta={}",
                deltas.iter().filter(|d| **d != 0).count(),
                deltas.iter().max().unwrap(),
                deltas.iter().map(|d| *d as u64).sum::<u64>()
            );
        }
    }
}
