//! Explicit RGB reference comparison, never invoked by ordinary tests.
use std::{path::Path, process::Command};
fn main() {
    let root = std::env::temp_dir().join(format!(
        "fvid-colorbalance-reference-{}",
        std::process::id()
    ));
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
            "rs=1:gs=-1:bs=0.2",
            "rm=0.7:gm=-0.3:bm=0.1",
            "rh=-0.8:gh=0.6:bh=0.2",
            "rs=0.3:gm=-0.2:bh=0.4:pl=1",
            "rs=1:gs=1:bs=-1:rm=-1:gm=1:bm=1:rh=1:gh=-1:bh=1:pl=1",
        ]
        .into_iter()
        .enumerate()
        {
            let mut actual = input.clone();
            fvid_media::owned_colorbalance::ColorBalance::parse(args)
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
                        &format!("colorbalance={args}"),
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
                    fixtures.join(format!("colorbalance-reference-{depth}-{case}.raw")),
                    &reference.stdout,
                )
                .unwrap();
            }
            println!("depth={depth} args={args} samples=768 matched");
        }
    }
    std::fs::remove_dir_all(Path::new(&root)).unwrap();
}
