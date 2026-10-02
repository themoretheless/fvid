//! Explicit synthetic Y4M lossless pixels and filter reference comparisons.
use std::path::PathBuf;
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-y4m-ffv1-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn source(layout: &str, sx: usize, sy: usize) -> (Vec<u8>, Vec<Vec<u8>>) {
    let range = if layout == "444" { "FULL" } else { "LIMITED" };
    let mut source =
        format!("YUV4MPEG2 W8 H6 F30000:1001 Ip A2:1 C{layout} XCOLORRANGE={range}\n").into_bytes();
    let mut frames = Vec::new();
    for i in 0..4 {
        let data = (0..48 + 2 * (8 / sx) * (6 / sy))
            .map(|j| ((j * 17 + i * 31) % 256) as u8)
            .collect::<Vec<_>>();
        source.extend(b"FRAME\n");
        source.extend(&data);
        frames.push(data);
    }
    (source, frames)
}
fn independent_decoder_reads_own_export_without_pixel_changes() {
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let d = dir("reference");
    for (layout, sx, sy, format) in [
        ("420jpeg", 2, 2, "yuv420p"),
        ("422", 2, 1, "yuv422p"),
        ("444", 1, 1, "yuv444p"),
    ] {
        let (bytes, frames) = source(layout, sx, sy);
        let src = d.0.join(format!("{layout}.y4m"));
        std::fs::write(&src, bytes).unwrap();
        let dst = d.0.join(format!("{layout}.mkv"));
        fvid::native_export::transcode_mp4_ffv1(&src, &dst, None, None).unwrap();
        let out = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&dst)
            .args(["-f", "rawvideo", "-pix_fmt", format, "pipe:1"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.stdout, frames.concat());
        let request = fvid::media_info::LosslessTransform {
            horizontal_flip: true,
            avgblur: Some("sizeX=2:sizeY=1".into()),
            boxblur: Some("1:2:1:1".into()),
            negate: Some("0".into()),
            chromashift: Some("cbh=1:edge=wrap".into()),
            ..Default::default()
        };
        let (geometry, filters) = fvid::native_lossless::configuration(&request).unwrap();
        let filtered = d.0.join(format!("{layout}-filtered.mkv"));
        fvid::native_export::transcode_ffv1_transformed(
            &src, &filtered, &geometry, &filters, None, None,
        )
        .unwrap();
        let expected = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&src)
            .args([
                "-vf",
                "hflip,avgblur=sizeX=2:sizeY=1,boxblur=1:2:1:1,negate,chromashift=cbh=1:edge=wrap",
                "-f",
                "rawvideo",
                "-pix_fmt",
                format,
                "pipe:1",
            ])
            .output()
            .unwrap();
        let actual = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&filtered)
            .args(["-f", "rawvideo", "-pix_fmt", format, "pipe:1"])
            .output()
            .unwrap();
        assert!(
            expected.status.success(),
            "{}",
            String::from_utf8_lossy(&expected.stderr)
        );
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, expected.stdout, "filtered {layout}");
    }
}


fn main() {
    std::env::var_os("FVID_REFERENCE_FFMPEG").expect("Set FVID_REFERENCE_FFMPEG for this explicit reference benchmark");
    independent_decoder_reads_own_export_without_pixel_changes();
    println!("Y4M lossless pixels and filter reference comparisons passed");
}
