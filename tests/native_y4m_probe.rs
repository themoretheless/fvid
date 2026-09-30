#[test]
fn y4m_probe_counts_tagged_frames_and_rejects_truncation() {
    let directory = std::env::temp_dir().join(format!("fvid-y4m-probe-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(directory.clone());
    for layout in ["420jpeg", "422", "444"] {
        let file = directory.join(layout);
        let header = format!("YUV4MPEG2 W4 H2 F30000:1001 Ip C{layout}\n");
        let frame_bytes = match layout {
            "420jpeg" => 12,
            "422" => 16,
            _ => 24,
        };
        let mut bytes = header.as_bytes().to_vec();
        for i in 0..3 {
            bytes.extend(format!("FRAME Xtag={i}\n").bytes());
            bytes.extend(vec![128; frame_bytes]);
        }
        std::fs::write(&file, &bytes).unwrap();
        let info = fvid::native_probe::probe(&file).unwrap();
        assert_eq!(info.format, "yuv4mpegpipe");
        assert_eq!(info.duration_us, Some(100100));
        let s = &info.streams[0];
        assert_eq!(s.duration, Some(3));
        assert_eq!(s.time_base, [1001, 30000]);
        assert_eq!(s.average_frame_rate, [30000, 1001]);
        assert_eq!((s.width, s.height), (4, 2));
        assert_eq!(s.codec, "rawvideo");
        assert_eq!(
            fvid::native_probe::probe_as(&file, Some("yuv4mpegpipe"))
                .unwrap()
                .streams,
            info.streams
        );
        #[cfg(feature = "media")]
        assert_eq!(fvid::media::probe(&file).unwrap().streams, info.streams);
        bytes.pop();
        std::fs::write(&file, &bytes).unwrap();
        assert!(
            fvid::native_probe::probe(&file)
                .unwrap_err()
                .contains("truncated")
        );
        #[cfg(feature = "media")]
        assert!(fvid::media::probe(&file).is_err());
    }
    let file = directory.join("unsupported");
    for header in [
        "YUV4MPEG2 W4 H2 F25:1 Ip C420p10\n",
        "YUV4MPEG2 W4 H2 F25:1 It C420jpeg\n",
    ] {
        std::fs::write(&file, header).unwrap();
        assert!(
            fvid::native_probe::try_probe_as(&file, None)
                .unwrap()
                .is_none()
        );
    }
    std::fs::write(&file, b"YUV4MPEG2 W4 H2 Ip C420jpeg\n").unwrap();
    assert_eq!(fvid::native_probe::probe(&file).unwrap().duration_us, None);
    for header in [
        "YUV4MPEG2 W4 H2 F0:1 Ip C420jpeg\n",
        "YUV4MPEG2 W4 H2 F25:0 Ip C420jpeg\n",
        "YUV4MPEG2 W4 H2 F25:1 F30:1 Ip C420jpeg\n",
    ] {
        let file = directory.join("bad");
        std::fs::write(&file, header).unwrap();
        assert!(fvid::native_probe::probe(&file).is_err());
    }
}
