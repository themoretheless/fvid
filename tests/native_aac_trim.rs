use std::path::{Path, PathBuf};
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-aac-trim-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio")
        .join(name)
}
#[test]
fn trim_keeps_decoder_preroll_and_matches_full_decode_sample_slice() {
    let d = dir("samples");
    for name in [
        "aac-mono-44k.aac",
        "aac-stereo.aac",
        "aac-51-active.aac",
        "aac-96k.aac",
    ] {
        let source = fixture(name);
        let data = std::fs::read(&source).unwrap();
        let mut full = Vec::new();
        let original =
            fvid::native_media::decode_aac_pcm(&data, &mut full, &Default::default()).unwrap();
        for (from, to) in [(33333, 100001), (1, 2000000)] {
            let dest = d.0.join(format!("{name}-{from}.wav"));
            let stats =
                fvid::native_export::trim_adts_wave(&source, &dest, from, to, None, None).unwrap();
            let first = (from as u64 * u64::from(original.sample_rate))
                .div_ceil(1000000)
                .min(original.sample_frames);
            let last = (to as u64 * u64::from(original.sample_rate))
                .div_ceil(1000000)
                .min(original.sample_frames);
            assert_eq!(stats.sample_frames, last - first);
            let bytes = std::fs::read(&dest).unwrap();
            let stride = usize::from(original.channels) * 4;
            assert_eq!(
                &bytes[80..],
                &full[first as usize * stride..last as usize * stride]
            );
            let cli = d.0.join(format!("cli-{name}-{from}.wav"));
            let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
                .args(["media", "trim"])
                .arg(&source)
                .arg(&cli)
                .args([
                    "--from",
                    &format!("{}.{:06}", from / 1000000, from % 1000000),
                    "--to",
                    &format!("{}.{:06}", to / 1000000, to % 1000000),
                    "--streams",
                    "0",
                ])
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
            assert_eq!(std::fs::read(cli).unwrap(), bytes);
            let plan = fvid::native_plan::trim_adts(&source, from, to).unwrap();
            assert!(
                plan.steps
                    .last()
                    .unwrap()
                    .detail
                    .contains(&format!("{} float32", last - first))
            );
            #[cfg(feature = "media")]
            {
                let out = d.0.join(format!("api-{name}-{from}.wav"));
                let stats =
                    fvid::media::trim(&source, &out, from, to, &Default::default()).unwrap();
                assert_eq!(stats.backend, "fvid");
                assert_eq!(std::fs::read(out).unwrap(), bytes);
                assert_eq!(
                    fvid::media::plan_trim(&source, from, to, &Default::default()).unwrap(),
                    plan
                );
            }
        }
    }
}
#[test]
fn invalid_ranges_cancel_and_existing_outputs_do_not_publish() {
    let d = dir("invalid");
    let source = fixture("aac-mono-44k.aac");
    let dest = d.0.join("out.wav");
    for (from, to) in [(-1, 1000), (1000, 1000), (2000000, 3000000)] {
        assert!(fvid::native_export::trim_adts_wave(&source, &dest, from, to, None, None).is_err());
        assert!(fvid::native_plan::trim_adts(&source, from, to).is_err());
        assert!(!dest.exists());
    }
    let flag = fvid::media_control::CancelFlag::default();
    let stop = flag.clone();
    let hook = fvid::media_control::ProgressHook::new(move |event| {
        assert!(!event.done);
        if event.packets > 0 {
            stop.cancel();
        }
    });
    assert!(
        fvid::native_export::trim_adts_wave(&source, &dest, 1000, 90000, Some(&flag), Some(&hook))
            .is_err()
    );
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 0);
    std::fs::write(&dest, b"keep").unwrap();
    assert!(fvid::native_export::trim_adts_wave(&source, &dest, 1000, 90000, None, None).is_err());
    assert_eq!(std::fs::read(&dest).unwrap(), b"keep");
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "trim"])
        .arg(&source)
        .args(["--from", "0.033333", "--to", "0.100001"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&run.stdout).unwrap(),
        serde_json::to_value(fvid::native_plan::trim_adts(&source, 33333, 100001).unwrap())
            .unwrap()
    );
}
