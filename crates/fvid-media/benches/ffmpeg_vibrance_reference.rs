//! Explicit RGB reference comparison, never invoked by ordinary tests.
use std::{path::Path, process::Command};
fn main() {
    let root = std::env::temp_dir().join(format!("fvid-vibrance-reference-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for depth in [8, 16] {
        let source = root.join(format!("rgb-{depth}.raw"));
        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        let input = std::fs::read(fixtures.join(format!("vibrance-grid-{depth}.rgba"))).unwrap();
        std::fs::write(&source, &input).unwrap();
        let format = if depth == 8 { "rgba" } else { "rgba64le" };
        for (case, args) in [
            "",
            "intensity=1",
            "intensity=-1",
            "intensity=2:alternate=1",
            "intensity=0.5:rbal=-1:gbal=0.2:bbal=2",
            "intensity=-0.7:rlum=0.3:glum=0.6:blum=0.1",
        ]
        .into_iter()
        .enumerate()
        {
            let mut actual = input.clone();
            fvid_media::owned_vibrance::Vibrance::parse(args)
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
                        &format!("vibrance={args}"),
                        "-frames:v",
                        "3",
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
            if std::env::var_os("FVID_WRITE_SYNTHETIC_REFERENCES").is_some() {
                std::fs::write(
                    fixtures.join(format!("vibrance-reference-{depth}-{case}.raw")),
                    &reference.stdout,
                )
                .unwrap();
            }
            println!("depth={depth} args={args} samples=768 matched");
        }
    }
    std::fs::remove_dir_all(Path::new(&root)).unwrap();
}
