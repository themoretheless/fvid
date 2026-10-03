use std::{fs::File, path::PathBuf};
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
#[test]
fn read_only_pcm_probe_accepts_opaque_ancillary_chunks_without_weakening_trim() {
    let source = fixture("wave-probe-ancillary.wav");
    let expected = fvid_media::owned_probe::probe_wave(&fixture("wave-probe-info.wav")).unwrap();
    let actual = fvid_media::owned_probe::probe_wave(&source).unwrap();
    for public in [
        fvid_media::probe(&source).unwrap(),
        fvid_media::probe_as(&source, Some("wav")).unwrap(),
    ] {
        assert_eq!(public.metadata, actual.metadata);
        assert_eq!(public.streams[0].duration, actual.streams[0].duration);
    }
    assert_eq!(actual.metadata, expected.metadata);
    assert_eq!(actual.duration_us, expected.duration_us);
    assert_eq!(actual.streams[0].codec, expected.streams[0].codec);
    assert_eq!(actual.streams[0].duration, expected.streams[0].duration);
    assert!(
        fvid_media::owned_wave_inspect::inspect(&mut File::open(source).unwrap(), None)
            .unwrap_err()
            .to_string()
            .contains("cannot yet preserve")
    );
}
#[test]
fn opaque_chunk_bounds_remain_validated() {
    let error = fvid_media::owned_probe::probe_wave(&fixture("wave-probe-ancillary-overflow.wav"))
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(error.to_string(), "WAVE chunk exceeds RIFF extent");
}
