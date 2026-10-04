//! Explicit RGB reference comparison, never invoked by ordinary tests.
use std::{path::Path, process::Command};
fn main() {
    let root = std::env::temp_dir().join(format!(
        "fvid-colorchannelmixer-reference-{}",
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
            "rr=0:rg=1:gg=0:gb=1:bb=0:br=1",
            "rr=0.5:rg=0.2:bb=0.7",
            "ra=0.5:ga=-0.5:ba=0.2:ar=0.2:ag=0.3:ab=0.5:aa=0",
            "rr=-1:rg=2:gg=0.5:gb=-0.2:bb=2",
            "rr=0.5:rg=0.5:rb=0.5",
            "rr=0.5:rg=0.2:bb=0.7:pc=lum:pa=1",
            "rr=0.5:rg=0.2:bb=0.7:pc=max:pa=0.5",
            "rr=0.5:rg=0.2:bb=0.7:pc=avg:pa=0.7",
            "rr=0.5:rg=0.2:bb=0.7:pc=sum:pa=1",
            "rr=0.5:rg=0.2:bb=0.7:pc=nrm:pa=1",
            "rr=0.5:rg=0.2:bb=0.7:pc=pwr:pa=1",
        ]
        .into_iter()
        .enumerate()
        {
            let mut actual = input.clone();
            for frame in actual.chunks_exact_mut(8 * 8 * 4 * if depth == 8 { 1 } else { 2 }) {
                fvid_media::owned_colorchannelmixer::ColorChannelMixer::parse(args)
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
                        &format!("colorchannelmixer={args}"),
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
                    fixtures.join(format!("colorchannelmixer-reference-{depth}-{case}.raw")),
                    &reference.stdout,
                )
                .unwrap();
            }
            println!("depth={depth} args={args} samples=768 matched");
        }
    }
    qualify_float(&root);
    std::fs::remove_dir_all(Path::new(&root)).unwrap();
}

fn qualify_float(root: &Path) {
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
    let input: Vec<f32> = std::fs::read(fixtures.join("vibrance-grid-8.rgba"))
        .unwrap()
        .iter()
        .map(|v| *v as f32 / 255.0 * 2.0 - 0.25)
        .collect();
    let source = root.join("float.raw");
    let planar: Vec<u8> = input
        .chunks_exact(256)
        .flat_map(|frame| {
            [1usize, 2, 0, 3].into_iter().flat_map(move |channel| {
                frame
                    .chunks_exact(4)
                    .flat_map(move |pixel| pixel[channel].to_le_bytes())
            })
        })
        .collect();
    std::fs::write(&source, planar).unwrap();
    for (case, args) in [
        "",
        "rr=0:rg=1:gg=0:gb=1:bb=0:br=1",
        "rr=0.5:rg=0.2:bb=0.7",
        "ra=0.5:ga=-0.5:ba=0.2:ar=0.2:ag=0.3:ab=0.5:aa=0",
        "rr=-1:rg=2:gg=0.5:gb=-0.2:bb=2",
        "rr=0.5:rg=0.5:rb=0.5",
        "rr=0.5:rg=0.2:bb=0.7:pc=lum:pa=1",
        "rr=0.5:rg=0.2:bb=0.7:pc=max:pa=0.5",
        "rr=0.5:rg=0.2:bb=0.7:pc=avg:pa=0.7",
        "rr=0.5:rg=0.2:bb=0.7:pc=sum:pa=1",
        "rr=0.5:rg=0.2:bb=0.7:pc=nrm:pa=1",
        "rr=0.5:rg=0.2:bb=0.7:pc=pwr:pa=1",
    ]
    .into_iter()
    .enumerate()
    {
        let mut actual = input.clone();
        fvid_media::owned_colorchannelmixer::ColorChannelMixer::parse(args)
            .unwrap()
            .apply_rgb_f32(&mut actual, 4)
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
                    "gbrapf32le",
                    "-video_size",
                    "8x8",
                    "-i",
                ])
                .arg(&source)
                .args([
                    "-vf",
                    &format!("colorchannelmixer={args}"),
                    "-frames:v",
                    "3",
                    "-pix_fmt",
                    "gbrapf32le",
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
        let planar: Vec<f32> = reference
            .stdout
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(planar.len(), input.len());
        let expected: Vec<f32> = planar
            .chunks_exact(256)
            .flat_map(|frame| {
                (0..64).flat_map(move |i| [frame[128 + i], frame[i], frame[64 + i], frame[192 + i]])
            })
            .collect();
        let mut maximum = 0.0f32;
        for (a, b) in actual.iter().zip(&expected) {
            let delta = (a - b).abs();
            maximum = maximum.max(delta);
            assert!(
                a.is_finite() && b.is_finite() && delta <= 8.0 * f32::EPSILON * b.abs().max(1.0),
                "case={case} own={a} reference={b} delta={delta}"
            );
        }
        println!("float case={case} samples=768 max_absolute_error={maximum}");
        if std::env::var_os("FVID_WRITE_SYNTHETIC_REFERENCES").is_some() {
            std::fs::write(
                fixtures.join(format!("colorchannelmixer-reference-float-{case}.raw")),
                expected
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        }
    }
}
