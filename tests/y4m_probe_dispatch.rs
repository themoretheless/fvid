use std::path::PathBuf;
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
#[test]
fn malformed_supported_y4m_never_retries_through_legacy_probe() {
    let source = fixture("y4m-truncated-frame.y4m");
    let expected = fvid_media::owned_y4m_probe::try_y4m(&source).unwrap_err();
    assert_eq!(expected, "truncated Y4M frame payload");
    assert_eq!(fvid_media::probe(&source).unwrap_err(), expected);
    assert_eq!(
        fvid_media::probe_as(&source, Some("y4m")).unwrap_err(),
        expected
    );
}
#[test]
fn unknown_signature_is_not_a_malformed_y4m() {
    let source = fixture("opus-lace-xiph.mka");
    assert!(fvid_media::owned_y4m_probe::try_y4m(&source)
        .unwrap()
        .is_none());
}
