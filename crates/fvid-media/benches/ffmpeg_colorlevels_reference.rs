//! Explicit RGB reference comparison, never invoked by ordinary tests.
use std::{path::Path, process::Command};
fn main() {
    let root =
        std::env::temp_dir().join(format!("fvid-colorlevels-reference-{}", std::process::id()));
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
            "rimin=0.2:rimax=0.8",
            "romin=0.1:bomax=0.7",
            "aimin=0.2:aimax=0.8:aomin=0.1:aomax=0.9",
            "rimin=-1:rimax=-1:gimin=-1:gimax=-1:bimin=-1:bimax=-1:aimin=-1:aimax=-1",
            "rimin=0.8:rimax=0.2",
            "romin=0.1:bomax=0.7:preserve=lum",
            "romin=0.1:bomax=0.7:preserve=max",
            "romin=0.1:bomax=0.7:preserve=avg",
            "romin=0.1:bomax=0.7:preserve=sum",
            "romin=0.1:bomax=0.7:preserve=nrm",
            "romin=0.1:bomax=0.7:preserve=pwr",
        ]
        .into_iter()
        .enumerate()
        {
            let mut actual = input.clone();
            for frame in actual.chunks_exact_mut(8 * 8 * 4 * if depth == 8 { 1 } else { 2 }) {
                fvid_media::owned_colorlevels::ColorLevels::parse(args)
                    .unwrap()
                    .apply_rgb(frame, depth, 4)
                    .unwrap();
            }
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
                        &format!("colorlevels={args}"),
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
                    fixtures.join(format!("colorlevels-reference-{depth}-{case}.raw")),
                    &reference.stdout,
                )
                .unwrap();
            }
            println!("depth={depth} args={args} samples=768 matched");
        }
    }
    std::fs::remove_dir_all(Path::new(&root)).unwrap();
}
