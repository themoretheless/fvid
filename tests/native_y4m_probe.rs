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
    std::fs::write(&file, "YUV4MPEG2 W4 H2 F25:1 Ip C420p10\n").unwrap();
    let supported = fvid::native_probe::try_probe_as(&file, None)
        .unwrap()
        .unwrap();
    assert_eq!(
        (supported.streams[0].width, supported.streams[0].height),
        (4, 2)
    );
    assert_eq!(supported.duration_us, Some(0));
    for header in ["YUV4MPEG2 W4 H2 F25:1 It C420jpeg\n"] {
        std::fs::write(&file, header).unwrap();
        assert!(
            fvid::native_probe::try_probe_as(&file, None)
                .unwrap()
                .is_none()
        );
    }
    std::fs::write(&file, b"YUV4MPEG2 W4 H2 Ip C420jpeg\n").unwrap();
    assert_eq!(
        fvid::native_probe::probe(&file).unwrap().duration_us,
        Some(0)
    );
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

#[test]
#[ignore = "requires FVID_REFERENCE_FFPROBE"]
fn native_y4m_rate_and_duration_match_independent_probe() {
    let binary = std::env::var_os("FVID_REFERENCE_FFPROBE").unwrap();
    let file = std::env::temp_dir().join(format!("fvid-y4m-oracle-{}.y4m", std::process::id()));
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _clean = Clean(file.clone());
    for fps in ["", "F25:1 ", "F60000:2002 ", "F24:1 ", "F24000:1001 "] {
        for frames in [1, 3, 7] {
            let mut bytes = format!("YUV4MPEG2 W4 H2 {fps}Ip C420jpeg\n").into_bytes();
            for _ in 0..frames {
                bytes.extend(b"FRAME\n");
                bytes.extend(vec![128; 12]);
            }
            std::fs::write(&file, bytes).unwrap();
            let actual = fvid::native_probe::probe(&file).unwrap();
            let result=std::process::Command::new(&binary).args(["-v","error","-show_entries","stream=codec_name,width,height,time_base,avg_frame_rate,duration_ts:format=duration","-of","json"]).arg(&file).output().unwrap();
            assert!(result.status.success());
            let reference: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
            let stream = &actual.streams[0];
            let expected = &reference["streams"][0];
            assert_eq!(expected["codec_name"], stream.codec);
            assert_eq!(expected["width"], stream.width);
            assert_eq!(expected["height"], stream.height);
            assert_eq!(expected["duration_ts"], stream.duration.unwrap());
            assert_eq!(
                expected["time_base"],
                format!("{}/{}", stream.time_base[0], stream.time_base[1])
            );
            assert_eq!(
                expected["avg_frame_rate"],
                format!(
                    "{}/{}",
                    stream.average_frame_rate[0], stream.average_frame_rate[1]
                )
            );
            let micros = (reference["format"]["duration"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap()
                * 1_000_000.0)
                .round() as i64;
            assert_eq!(actual.duration_us, Some(micros), "{fps} {frames}");
        }
    }
}
