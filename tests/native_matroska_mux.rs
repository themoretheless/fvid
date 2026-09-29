use fvid::container::{adts, matroska_write, webm};
use std::path::{Path, PathBuf};
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-mka-mux-{name}-{}", std::process::id()));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/audio")
        .join(name)
}
#[test]
fn packets_configuration_nanosecond_clock_and_pcm_survive_matroska() {
    let d = dir("roundtrip");
    for name in [
        "aac-mono-44k.aac",
        "aac-stereo.aac",
        "aac-51-active.aac",
        "aac-96k.aac",
        "aac-88k.aac",
    ] {
        let path = fixture(name);
        let data = std::fs::read(&path).unwrap();
        let source = adts::Aac::parse(&data, &Default::default()).unwrap();
        let mut output = std::io::Cursor::new(Vec::new());
        let event = matroska_write::write_adts(
            adts::StreamReader::open(data.as_slice()).unwrap(),
            &mut output,
            None,
            None,
        )
        .unwrap();
        assert!(!event.done);
        assert_eq!(event.packets, source.packets() as u64);
        let mut reader =
            webm::WebmReader::open(std::io::Cursor::new(output.get_ref()), Default::default())
                .unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks.len(), 1);
        assert_eq!(reader.tracks[0].codec, "A_AAC");
        assert_eq!(reader.tracks[0].sample_rate, u64::from(source.sample_rate));
        assert_eq!(reader.tracks[0].channels, u64::from(source.channels));
        assert_eq!(reader.packets.len(), source.packets());
        for i in 0..source.packets() {
            assert_eq!(reader.read_packet(i).unwrap(), source.packet(i));
            assert_eq!(
                reader.packets[i].pts_ns,
                (i as u128 * 1024 * 1000000000 / u128::from(source.sample_rate)) as i64
            );
        }
        let (mut before, mut after) = (Vec::new(), Vec::new());
        fvid::native_media::decode_aac_pcm(&data, &mut before, &Default::default()).unwrap();
        fvid::native_media::decode_matroska_aac_pcm_interval(output.get_ref(), &mut after, None).unwrap();
        assert!(before == after, "PCM differs for {name}");
        let dest = d.0.join(format!("{name}.mka"));
        let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", "remux"])
            .arg(&path)
            .arg(&dest)
            .arg("--progress")
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(std::fs::read(&dest).unwrap(), *output.get_ref());
        #[cfg(feature = "media")]
        {
            let api = d.0.join(format!("api-{name}.mkv"));
            let stats = fvid::media::remux(&path, &api, &Default::default()).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(std::fs::read(api).unwrap(), *output.get_ref());
        }
    }
}
#[test]
fn writer_errors_cancel_and_publication_never_leave_outputs() {
    let d = dir("errors");
    let src = d.0.join("source.aac");
    let dest = d.0.join("out.mka");
    let mut data = std::fs::read(fixture("aac-mono-44k.aac")).unwrap();
    std::fs::write(&src, &data).unwrap();
    let flag = fvid::media_control::CancelFlag::default();
    let stop = flag.clone();
    let hook = fvid::media_control::ProgressHook::new(move |e| {
        assert!(!e.done);
        if e.packets > 0 {
            stop.cancel();
        }
    });
    assert!(
        fvid::native_export::remux_adts_aac_controlled(&src, &dest, Some(&flag), Some(&hook))
            .is_err()
    );
    assert!(!dest.exists());
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
    data.pop();
    std::fs::write(&src, &data).unwrap();
    assert!(fvid::native_export::remux_adts_aac(&src, &dest).is_err());
    assert_eq!(std::fs::read_dir(&d.0).unwrap().count(), 1);
    std::fs::copy(fixture("aac-mono-44k.aac"), &src).unwrap();
    std::fs::write(&dest, b"keep").unwrap();
    assert!(fvid::native_export::remux_adts_aac(&src, &dest).is_err());
    assert_eq!(std::fs::read(dest).unwrap(), b"keep");
    struct Fails(std::io::Cursor<Vec<u8>>);
    impl std::io::Write for Fails {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("output failed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl std::io::Seek for Fails {
        fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
            std::io::Seek::seek(&mut self.0, p)
        }
    }
    assert!(
        matroska_write::write_adts(
            adts::StreamReader::open(std::fs::read(src).unwrap().as_slice()).unwrap(),
            &mut Fails(std::io::Cursor::new(Vec::new())),
            None,
            None
        )
        .is_err()
    );
}
