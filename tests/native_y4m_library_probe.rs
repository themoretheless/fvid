//! The media library owns the same Y4M grammar as the frontend.
use std::{fs, path::PathBuf};
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn temp() -> Temp {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "fvid-y4m-owner-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    Temp(path)
}
#[test]
fn every_owned_chroma_and_depth_has_exact_frontend_and_library_metadata() {
    let folder = temp();
    for chroma in [
        "420", "420jpeg", "420mpeg2", "420paldv", "422", "444", "420p9", "420p10", "420p12",
        "420p14", "420p16", "422p10", "444p16", "440", "440p10", "440p16", "411",
    ] {
        let header = format!("YUV4MPEG2 W8 H6 F60000:2002 Ip C{chroma}\n");
        let frontend = fvid::Header::parse(header.as_bytes()).unwrap();
        let library = fvid_media::owned_y4m::Header::parse(header.as_bytes()).unwrap();
        assert_eq!(
            (
                frontend.width,
                frontend.height,
                frontend.depth(),
                frontend.frame_len().unwrap()
            ),
            (
                library.width,
                library.height,
                library.depth(),
                library.frame_len().unwrap()
            )
        );
        let source = folder.0.join(format!("{chroma}.y4m"));
        let mut bytes = header.into_bytes();
        for _ in 0..3 {
            bytes.extend_from_slice(b"FRAME Xsynthetic=1\n");
            bytes.resize(bytes.len() + library.frame_len().unwrap(), 96);
        }
        fs::write(&source, &bytes).unwrap();
        let own = fvid_media::owned_probe::probe(&source).unwrap();
        let native = fvid::native_probe::probe(&source).unwrap();
        let public = fvid_media::probe(&source).unwrap();
        assert_eq!(
            serde_json::to_value(&own).unwrap(),
            serde_json::to_value(&native).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&own).unwrap(),
            serde_json::to_value(&public).unwrap()
        );
        assert_eq!(own.duration_us, Some(100100));
        assert_eq!(own.streams[0].average_frame_rate, [30000, 1001]);
        assert_eq!(own.streams[0].duration, Some(3));
        let decoded = fvid_media::decode_video(&source).unwrap();
        assert_eq!(
            (
                decoded.video_frames,
                decoded.width,
                decoded.height,
                decoded.decode_errors
            ),
            (3, 8, 6, 0)
        );
        assert_eq!(decoded.backend, "owned Y4M raw decode");
        assert!(fvid_media::probe_as(&source, Some("yuv4mpegpipe")).is_ok());
    }
}
#[test]
fn shared_parser_retains_bounded_lines_and_truncated_payload_refusal() {
    let folder = temp();
    let path = folder.0.join("bad.y4m");
    for bytes in [
        b"YUV4MPEG2 W8 H6 C420 F30:1\nFRAME\n".to_vec(),
        b"YUV4MPEG2 W8 H6 C420 F30:1 F60:1\n".to_vec(),
        [
            b"YUV4MPEG2 W8 H6 C420 ".as_slice(),
            &vec![b'X'; 4096],
            b"\n",
        ]
        .concat(),
    ] {
        fs::write(&path, &bytes).unwrap();
        let owned = fvid_media::owned_y4m_probe::probe_y4m(&path).unwrap_err();
        let frontend = fvid::native_probe::probe(&path).unwrap_err();
        assert_eq!(owned, frontend);
    }
}

#[test]
fn owned_y4m_decoder_accepts_short_reads_and_refuses_incomplete_payload() {
    use std::io::{BufReader, Cursor, Read};
    struct Short(Cursor<Vec<u8>>);
    impl Read for Short {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            let count = bytes.len().min(3);
            self.0.read(&mut bytes[..count])
        }
    }
    let data = include_bytes!("fixtures/playback-errors/wave-probe-info.y4m").to_vec();
    let full = fvid_media::owned_y4m_decode::decode_reader(BufReader::new(Short(Cursor::new(
        data.clone(),
    ))))
    .unwrap();
    assert_eq!((full.video_frames, full.width, full.height), (3, 16, 16));
    let error = fvid_media::owned_y4m_decode::decode_reader(BufReader::new(Short(Cursor::new(
        data[..data.len() - 1].to_vec(),
    ))))
    .unwrap_err();
    assert_eq!(error, "truncated Y4M frame payload");
}
