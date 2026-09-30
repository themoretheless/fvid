use fvid::{
    container::webm::WebmReader,
    native_geometry::VideoGeometry,
    native_pixels::PixelFilters,
    playback_native::{NativeReader, RawFrame},
};
use std::{io::Cursor, path::PathBuf};
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
#[test]
fn exact_pixels_clock_and_aspect_in_all_y4m_layouts() {
    let d = dir("roundtrip");
    for (layout, sx, sy) in [("420jpeg", 2, 2), ("422", 2, 1), ("444", 1, 1)] {
        let (input, expected) = source(layout, sx, sy);
        let path = d.0.join(format!("{layout}.y4m"));
        std::fs::write(&path, input).unwrap();
        let mut output = Cursor::new(Vec::new());
        let (stats, event) = fvid::native_lossless_y4m::write(
            &path,
            &mut output,
            &Default::default(),
            &Default::default(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(stats.video_frames, 4);
        assert!(!event.done);
        let data = output.into_inner();
        let mut mux = WebmReader::open(Cursor::new(&data), Default::default()).unwrap();
        mux.scan_all().unwrap();
        assert_eq!(mux.tracks[0].pixel_aspect(), (2, 1));
        assert_eq!(mux.tracks[0].colour.full_range, layout == "444");
        assert_eq!(mux.packets.len(), 4);
        for (i, p) in mux.packets.iter().enumerate() {
            let start = (i as u64 * 1001 * 1_000_000_000) / 30000;
            let end = ((i + 1) as u64 * 1001 * 1_000_000_000) / 30000;
            assert_eq!(p.pts_ns, start as i64);
            assert_eq!(p.duration_ns, Some(end - start));
        }
        let mut reader = NativeReader::software(Cursor::new(data), usize::MAX).unwrap();
        for want in expected {
            let RawFrame::Planar(p) = reader.read_frame_raw().unwrap().unwrap() else {
                panic!("expected FFV1 planes")
            };
            assert_eq!(p.frame.data, want);
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
    }
}
#[test]
fn cli_api_filters_and_failure_publication() {
    let d = dir("cli");
    let src = d.0.join("input.y4m");
    let (data, _) = source("420jpeg", 2, 2);
    std::fs::write(&src, &data).unwrap();
    let request = fvid::media_info::LosslessTransform {
        horizontal_flip: true,
        chromashift: Some("cbh=1:edge=wrap".into()),
        ..Default::default()
    };
    assert!(fvid::native_lossless::supports(&request));
    assert!(fvid::native_lossless::eligible(&src).unwrap());
    let (geometry, filters) = fvid::native_lossless::configuration(&request).unwrap();
    let output = d.0.join("out.mkv");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "transcode-lossless"])
        .arg(&src)
        .arg(&output)
        .args(["--hflip", "--chromashift", "cbh=1:edge=wrap"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(&output).unwrap();
    let mut expected = Cursor::new(Vec::new());
    fvid::native_lossless_y4m::write(&src, &mut expected, &geometry, &filters, None, None).unwrap();
    assert_eq!(bytes, expected.into_inner());
    #[cfg(feature = "media")]
    {
        let dest = d.0.join("api.mkv");
        let stats =
            fvid::media::transcode_lossless(&src, &dest, request, &Default::default()).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(std::fs::read(dest).unwrap(), bytes);
    }
    let malformed = d.0.join("truncated.y4m");
    std::fs::write(&malformed, &data[..data.len() - 1]).unwrap();
    let dest = d.0.join("bad.mkv");
    assert!(
        fvid::native_export::transcode_mp4_ffv1_transformed(
            &malformed, &dest, &geometry, &filters, None, None
        )
        .is_err()
    );
    assert!(!dest.exists());
    assert!(
        fvid::native_export::transcode_mp4_ffv1_transformed(
            &src, &output, &geometry, &filters, None, None
        )
        .is_err()
    );
    assert_eq!(std::fs::read(output).unwrap(), bytes);
    let cancel = fvid::media_control::CancelFlag::new();
    let c = cancel.clone();
    let hook = fvid::media_control::ProgressHook::new(move |e| {
        assert!(!e.done);
        if e.packets > 0 {
            c.cancel();
        }
    });
    assert!(
        fvid::native_export::transcode_mp4_ffv1_transformed(
            &src,
            &dest,
            &VideoGeometry::default(),
            &PixelFilters::default(),
            Some(&cancel),
            Some(&hook)
        )
        .is_err()
    );
    assert!(!dest.exists());
    assert!(!std::fs::read_dir(&d.0).unwrap().any(|p| {
        p.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fvid-")
    }));
}

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
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
    }
}

#[test]
fn unsupported_y4m_profiles_do_not_take_over_the_legacy_route() {
    let d = dir("admission");
    let p = d.0.join("in.y4m");
    for header in [
        "C420p10 Ip F25:1",
        "C420mpeg2 Ip F25:1",
        "C420paldv Ip F25:1",
        "C420jpeg It F25:1",
        "C444 Ip",
    ] {
        std::fs::write(&p, format!("YUV4MPEG2 W8 H6 {header}\n")).unwrap();
        assert!(!fvid::native_lossless::eligible(&p).unwrap(), "{header}");
    }
}
