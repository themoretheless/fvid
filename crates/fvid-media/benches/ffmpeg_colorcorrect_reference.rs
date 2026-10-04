//! Explicit single-thread reference qualification; not part of ordinary tests.
use std::{path::Path, process::Command};
fn main() {
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
    for depth in [8, 12, 16] {
        let source = fixtures.join(format!("colorcorrect-grid-{depth}.y4m"));
        let input = std::fs::read(&source).unwrap();
        let format = if depth == 8 {
            "yuv420p"
        } else if depth == 12 {
            "yuv420p12le"
        } else {
            "yuv420p16le"
        };
        for (case, args) in [
            "",
            "rl=0.2:bl=-0.3:rh=-0.1:bh=0.4",
            "saturation=-2",
            "saturation=0",
            "analyze=average",
            "analyze=minmax:saturation=0.7",
            "analyze=median",
            "rl=1:bl=-1:rh=-1:bh=1:saturation=3",
        ]
        .into_iter()
        .enumerate()
        {
            let transform = fvid_media_info::DecodeTransform {
                colorcorrect: Some(args.into()),
                ..Default::default()
            };
            let mut actual = Vec::new();
            fvid_media::owned_y4m_decode::visit_reader_transformed(
                std::io::Cursor::new(&input),
                &transform,
                |_, frame, _, _| {
                    actual.extend_from_slice(frame);
                    Ok(())
                },
            )
            .unwrap();
            let reference =
                Command::new(std::env::var("FVID_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()))
                    .args(["-nostdin", "-v", "error", "-filter_threads", "1", "-i"])
                    .arg(&source)
                    .args([
                        "-vf",
                        &format!("colorcorrect={args}"),
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
                    fixtures.join(format!("colorcorrect-reference-{depth}-{case}.raw")),
                    &reference.stdout,
                )
                .unwrap();
            }
            println!("depth={depth} args={args} samples=129 matched");
        }
    }
}
