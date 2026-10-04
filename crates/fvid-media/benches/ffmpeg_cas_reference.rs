//! Explicit single-thread reference qualification; not part of ordinary tests.
use std::{path::Path, process::Command};
fn main() {
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
    for depth in [8, 12, 16] {
        let source = fixtures.join(format!("cas-grid-{depth}.y4m"));
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
            "strength=1",
            "planes=0",
            "strength=0.7:planes=1",
            "strength=0.3:planes=6",
            "strength=1:planes=15",
        ]
        .into_iter()
        .enumerate()
        {
            let transform = fvid_media_info::DecodeTransform {
                cas: Some(args.into()),
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
                        &format!("cas={args}"),
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
                    fixtures.join(format!("cas-reference-{depth}-{case}.raw")),
                    &reference.stdout,
                )
                .unwrap();
            }
            println!("depth={depth} args={args} samples=129 matched");
        }
    }
    rgb_reference();
}

fn rgb_reference() {
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
    let temporary =
        std::env::temp_dir().join(format!("fvid-cas-rgb-reference-{}", std::process::id()));
    std::fs::create_dir_all(&temporary).unwrap();
    for depth in [8, 16] {
        let input = std::fs::read(fixtures.join(format!("vibrance-grid-{depth}.rgba"))).unwrap();
        let source = temporary.join(format!("input-{depth}.raw"));
        std::fs::write(&source, &input).unwrap();
        let format = if depth == 8 { "rgba" } else { "rgba64le" };
        let planar = if depth == 8 { "gbrap" } else { "gbrap16le" };
        for (case, args) in [
            "",
            "strength=1",
            "planes=0",
            "strength=0.7:planes=1",
            "strength=0.3:planes=6",
            "strength=1:planes=15",
        ]
        .into_iter()
        .enumerate()
        {
            let mut actual = Vec::new();
            for frame in input.chunks_exact(8 * 8 * 4 * if depth == 8 { 1 } else { 2 }) {
                let mut output = frame.to_vec();
                fvid_media::owned_cas::Cas::parse(args)
                    .unwrap()
                    .apply_rgb(&mut output, 8, 8, depth, 4)
                    .unwrap();
                actual.extend_from_slice(&output);
            }
            let reference =
                Command::new(std::env::var("FVID_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()))
                    .args([
                        "-nostdin",
                        "-v",
                        "error",
                        "-filter_threads",
                        "1",
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
                        &format!("format={planar},cas={args}"),
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
            assert_eq!(actual, reference.stdout, "RGB depth={depth} args={args}");
            if std::env::var_os("FVID_WRITE_SYNTHETIC_REFERENCES").is_some() {
                std::fs::write(
                    fixtures.join(format!("cas-rgb-reference-{depth}-{case}.raw")),
                    &reference.stdout,
                )
                .unwrap();
            }
            println!("RGB depth={depth} args={args} samples=768 matched");
        }
    }
    std::fs::remove_dir_all(temporary).unwrap();
}
