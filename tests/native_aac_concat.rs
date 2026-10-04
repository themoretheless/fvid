use fvid::container::{adts, mp4};
use std::path::{Path, PathBuf};
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-aac-concat-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio")
        .join(name)
}
#[test]
fn packets_timestamps_cli_and_api_match_owned_concat() {
    let d = dir("packets");
    for name in [
        "aac-mono-44k.aac",
        "aac-stereo.aac",
        "aac-51-active.aac",
        "aac-96k.aac",
    ] {
        let source = fixture(name);
        let sources = vec![source.clone(), source.clone()];
        let bytes = std::fs::read(source).unwrap();
        let input = adts::Aac::parse(&bytes, &Default::default()).unwrap();
        let dest = d.0.join(format!("{name}.m4a"));
        let stats = fvid::native_export::concat_adts_aac(&sources, &dest, None, None).unwrap();
        assert_eq!(stats.packets, 2 * input.packets() as u64);
        assert!(stats.done);
        let output = std::fs::read(&dest).unwrap();
        let mut reader =
            mp4::Mp4Reader::open(std::io::Cursor::new(&output), Default::default()).unwrap();
        assert_eq!(reader.tracks()[0].duration, 2 * input.samples());
        let mut packet = Vec::new();
        for index in 0..2 * input.packets() {
            reader.read_packet(0, index, &mut packet).unwrap();
            assert_eq!(packet, input.packet(index % input.packets()));
            assert_eq!(
                reader.tracks()[0].samples.get(index).unwrap().pts,
                index as i64 * 1024
            );
        }
        let cli = d.0.join(format!("cli-{name}.mp4"));
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "concat"])
            .arg(&cli)
            .args(&sources)
            .args(["--streams", "0"])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(std::fs::read(cli).unwrap(), output);
        let plan = fvid::native_plan::concat_adts(&sources).unwrap();
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "plan", "concat"])
            .args(&sources)
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
        {
            let dest = d.0.join(format!("api-{name}.m4a"));
            let stats = fvid::media::concat(&sources, &dest, &Default::default()).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(std::fs::read(dest).unwrap(), output);
            assert_eq!(
                fvid::media::plan_concat(&sources, &Default::default()).unwrap(),
                plan
            );
        }
        assert!(fvid::native_export::concat_adts_aac(&sources, &dest, None, None).is_err());
        assert_eq!(std::fs::read(dest).unwrap(), output);
    }
}
#[test]
fn truncated_segments_and_mismatch_never_publish() {
    let d = dir("invalid");
    let source = fixture("aac-mono-44k.aac");
    let bad = d.0.join("bad.aac");
    let dest = d.0.join("out.mp4");
    let mut bytes = std::fs::read(&source).unwrap();
    bytes.pop();
    std::fs::write(&bad, bytes).unwrap();
    for sources in [
        vec![bad.clone(), source.clone()],
        vec![source.clone(), bad.clone()],
        vec![source.clone(), fixture("aac-stereo.aac")],
    ] {
        assert!(fvid::native_export::concat_adts_aac(&sources, &dest, None, None).is_err());
        assert!(fvid::native_plan::concat_adts(&sources).is_err());
        assert!(!dest.exists());
        assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
    }
    let flag = fvid::media_control::CancelFlag::default();
    let cancel = flag.clone();
    let hook = fvid::media_control::ProgressHook::new(move |event| {
        assert!(!event.done);
        if event.packets > 0 {
            cancel.cancel();
        }
    });
    assert!(
        fvid::native_export::concat_adts_aac(
            &[source.clone(), source.clone()],
            &dest,
            Some(&flag),
            Some(&hook)
        )
        .is_err()
    );
    assert!(!dest.exists());
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
    assert!(fvid::native_export::concat_adts_aac(&[source], &dest, None, None).is_err());
}
