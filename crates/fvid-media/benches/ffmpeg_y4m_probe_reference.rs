//! Explicit ffprobe comparison; never part of ordinary tests.
fn main() {
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
            let actual = fvid_media::owned_y4m_probe::probe_y4m(&file).unwrap();
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
    println!("15 synthetic Y4M rate/duration cases match ffprobe");
}
