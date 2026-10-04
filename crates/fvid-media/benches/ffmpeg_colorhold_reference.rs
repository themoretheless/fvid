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
    std::fs::remove_dir_all(Path::new(&root)).unwrap();
}
