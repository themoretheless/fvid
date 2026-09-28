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
