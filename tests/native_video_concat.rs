use std::path::{Path, PathBuf};
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-video-concat-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
#[test]
fn avc_hevc_concat_preserves_exported_samples_and_cli_api_agree() {
    let d = dir("pixels");
    for (i, name) in ["video.mp4", "hevc/main-ipb.mp4", "hevc/main10-ipb.mp4"]
        .iter()
        .enumerate()
    {
        let source = fixture(name);
        let sources = vec![source.clone(), source.clone()];
        let single = d.0.join(format!("{i}-single.y4m"));
        let count = fvid::native_export::export_y4m(&source, &single).unwrap();
        let bytes = std::fs::read(single).unwrap();
        let split = bytes.iter().position(|&b| b == b'\n').unwrap() + 1;
        let expected = [bytes.as_slice(), &bytes[split..]].concat();
        let dest = d.0.join(format!("{i}-out.y4m"));
        let stats = fvid::native_export::concat_y4m(&sources, &dest, None, None, None).unwrap();
        assert_eq!(stats.packets, 2 * count);
        assert!(stats.done);
        assert!(stats.payload_bytes > 0);
        assert_eq!(std::fs::read(&dest).unwrap(), expected);
        assert!(fvid::native_export::concat_y4m(&sources, &dest, None, None, None).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), expected);
        let cli = d.0.join(format!("{i}-cli.y4m"));
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "concat"])
            .arg(&cli)
            .args(&sources)
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(std::fs::read(cli).unwrap(), expected);
        let plan = fvid::native_plan::concat_y4m(&sources, None).unwrap();
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "concat"])
            .args(&sources)
            .args(["--output-format", "y4m"])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&run.stdout).unwrap(),
            serde_json::to_value(&plan).unwrap()
        );
        #[cfg(feature = "media")]
        {
            let api = d.0.join(format!("{i}-api.y4m"));
            let stats = fvid::media::concat(&sources, &api, &Default::default()).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(std::fs::read(api).unwrap(), expected);
            assert_eq!(
                fvid::media::plan_concat_y4m(&sources, &Default::default()).unwrap(),
                plan
            );
        }
    }
}
#[test]
fn mismatched_rates_range_empty_segment_and_cancel_never_publish() {
    let d = dir("invalid");
    let a = d.0.join("a.y4m");
    let b = d.0.join("b.y4m");
    let out = d.0.join("out.y4m");
    let data =
        b"YUV4MPEG2 W2 H2 F30:1 Ip A1:1 C420 XCOLORRANGE=LIMITED\nFRAME\n\x10\x11\x12\x13\x80\x80";
    std::fs::write(&a, data).unwrap();
    let sources = vec![a.clone(), b.clone()];
    let payload = &data[data.iter().position(|&b| b == b'\n').unwrap() + 1..];
    let fast = [
        b"YUV4MPEG2 W2 H2 F60:1 Ip A1:1 C420 XCOLORRANGE=LIMITED\n".as_slice(),
        payload,
    ]
    .concat();
    for bytes in [fast, b"YUV4MPEG2 W2 H2 F30:1 Ip A1:1 C420\n".to_vec()] {
        std::fs::write(&b, bytes).unwrap();
        assert!(fvid::native_export::concat_y4m(&sources, &out, None, None, None).is_err());
        assert!(!out.exists());
    }
    let mut full = b"YUV4MPEG2 W2 H2 F30:1 Ip A1:1 C420 XCOLORRANGE=FULL\n".to_vec();
    full.extend_from_slice(&data[data.iter().position(|&b| b == b'\n').unwrap() + 1..]);
    std::fs::write(&b, full).unwrap();
    assert!(fvid::native_export::concat_y4m(&sources, &out, None, None, None).is_err());
    assert!(!out.exists());
    std::fs::write(&b, data).unwrap();
    let flag = fvid::media_control::CancelFlag::default();
    let cancel = flag.clone();
    let hook = fvid::media_control::ProgressHook::new(move |e| {
        assert!(!e.done);
        if e.packets > 0 {
            cancel.cancel();
        }
    });
    assert!(
        fvid::native_export::concat_y4m(&sources, &out, None, Some(&flag), Some(&hook)).is_err()
    );
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 2);
    let mixed = vec![fixture("audio/two-audio.mp4"); 2];
    assert!(fvid::native_plan::concat_y4m(&mixed, None).is_err());
    assert!(fvid::native_plan::concat_y4m(&mixed, Some(1)).is_err());
    assert!(fvid::native_plan::concat_y4m(&mixed, Some(0)).is_ok());
}

#[test]
fn interval_trim_matches_full_decode_presentation_frames() {
    let d = dir("trim");
    for (i, name) in ["video.mp4", "hevc/main-ipb.mp4", "hevc/main10-ipb.mp4"]
        .iter()
        .enumerate()
    {
        let source = fixture(name);
        let full = d.0.join(format!("{i}-full.y4m"));
        let total = fvid::native_export::export_y4m(&source, &full).unwrap();
        let bytes = std::fs::read(full).unwrap();
        let split = bytes.iter().position(|&b| b == b'\n').unwrap() + 1;
        let header = std::str::from_utf8(&bytes[..split]).unwrap();
        let rate = header
            .split_whitespace()
            .find(|s| s.starts_with('F'))
            .unwrap();
        let (num, den) = rate[1..].split_once(':').unwrap();
        let num = num.parse::<u128>().unwrap();
        let den = den.parse::<u128>().unwrap();
        let stride = (bytes.len() - split) / total as usize;
        assert_eq!((bytes.len() - split) % total as usize, 0);
        let (from, to) = (41000i64, 221000i64);
        let mut expected = bytes[..split].to_vec();
        let mut count = 0;
        for frame in 0..total as usize {
            let time = frame as u128 * den * 1000000;
            if time >= from as u128 * num && time < to as u128 * num {
                expected.extend_from_slice(
                    &bytes[split + frame * stride..split + (frame + 1) * stride],
                );
                count += 1;
            }
        }
        assert!(count > 0);
        let output = d.0.join(format!("{i}-trim.y4m"));
        let stats =
            fvid::native_export::trim_y4m(&source, &output, from, to, None, None, None).unwrap();
        assert_eq!(stats.packets, count);
        assert!(std::fs::read(output).unwrap()==expected,"trim pixels differ for {name}");
        let cli = d.0.join(format!("{i}-cli.y4m"));
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "trim"])
            .arg(&source)
            .arg(&cli)
            .args(["--from", "0.041", "--to", "0.221", "--progress"])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(std::fs::read(cli).unwrap(), expected);
        let plan = fvid::native_plan::trim_y4m(&source, from, to, None).unwrap();
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "trim"])
            .arg(&source)
            .args(["--from", "0.041", "--to", "0.221", "--output-format", "y4m"])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&run.stdout).unwrap(),
            serde_json::to_value(&plan).unwrap()
        );
        #[cfg(feature = "media")]
        {
            let api = d.0.join(format!("{i}-api.y4m"));
            let stats = fvid::media::trim(&source, &api, from, to, &Default::default()).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(std::fs::read(api).unwrap(), expected);
            assert_eq!(
                fvid::media::plan_trim_y4m(&source, from, to, &Default::default()).unwrap(),
                plan
            );
        }
        let empty = d.0.join(format!("{i}-empty.y4m"));
        assert!(
            fvid::native_export::trim_y4m(&source, &empty, 9000000, 10000000, None, None, None)
                .is_err()
        );
        assert!(!empty.exists());
    }
}
