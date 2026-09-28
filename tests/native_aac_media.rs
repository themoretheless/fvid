use fvid::{container::adts::Limits, native_media::decode_aac_pcm};

#[test]
fn headless_aac_export_matches_saved_pcm_reference() {
    let mut pcm = Vec::new();
    let stats = decode_aac_pcm(include_bytes!("fixtures/audio/aac-mono-44k.aac"),
        &mut pcm, &Limits::default()).unwrap();
    assert_eq!((stats.sample_rate, stats.channels, stats.decoded_frames, stats.sample_frames),
        (44100, 1, 7, 7168));
    let oracle = include_bytes!("fixtures/audio/aac-mono-reference.f32le");
    assert_eq!(pcm.len(), oracle.len());
    let mut squared = 0.0f64;
    let mut peak = 0.0f64;
    for (a, b) in pcm.chunks_exact(4).zip(oracle.chunks_exact(4)) {
        let delta = f64::from(f32::from_le_bytes(a.try_into().unwrap()))
            - f64::from(f32::from_le_bytes(b.try_into().unwrap()));
        squared += delta * delta;
        peak = peak.max(delta.abs());
    }
    assert!((squared / stats.sample_frames as f64).sqrt() < 4e-5);
    assert!(peak < 3e-4);
}

#[test]
fn destination_errors_are_propagated() {
    struct Fails;
    impl std::io::Write for Fails {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("destination failed"))
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let error = decode_aac_pcm(include_bytes!("fixtures/audio/aac-mono-44k.aac"),
        &mut Fails, &Limits::default()).unwrap_err();
    assert!(error.to_string().contains("destination failed"));
}

#[test]
fn cli_exports_complete_pcm_without_overwriting_or_partial_files() {
    let dir = std::env::temp_dir().join(format!("fvid-aac-export-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    let _cleanup = Cleanup(dir.clone());
    let source = dir.join("source.aac");
    let output = dir.join("out.f32le");
    let fixture = include_bytes!("fixtures/audio/aac-mono-44k.aac");
    std::fs::write(&source, fixture).unwrap();
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"]).arg(&source).arg(&output).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(String::from_utf8_lossy(&run.stdout).contains("\"sample_frames\":7168"));
    let mut expected = Vec::new();
    decode_aac_pcm(fixture, &mut expected, &Limits::default()).unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), expected);
    assert!(fvid::native_export::export_aac_pcm(&source, &output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), expected);
    let frames = fvid::container::adts::Aac::parse(fixture, &Limits::default()).unwrap();
    let second = frames.frames[1];
    let mut corrupt = fixture.to_vec();
    corrupt[second.start + second.header_bytes..second.start + second.size].fill(0xff);
    std::fs::write(&source, corrupt).unwrap();
    let failed = dir.join("failed.f32le");
    assert!(fvid::native_export::export_aac_pcm(&source, &failed).is_err());
    assert!(!failed.exists());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
}
