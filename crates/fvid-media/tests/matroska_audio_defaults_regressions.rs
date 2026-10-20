use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/matroska-audio-defaults")
}
#[test]
fn explicit_control_decodes_the_authored_pcm() {
    check("control");
}
fn check(name: &str) {
    let source = root().join(format!("{name}.mkv"));
    let reader = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(fs::read(&source).unwrap()),
        Default::default(),
    )
    .unwrap();
    assert_eq!(
        (reader.tracks[1].sample_rate, reader.tracks[1].channels),
        (8000, 1),
        "{name}"
    );
    let mut output = Vec::new();
    let stats = fvid_media::owned_matroska_pcm::decode_matroska_pcm(
        Cursor::new(fs::read(&source).unwrap()),
        &mut output,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        (stats.sample_rate, stats.channels, stats.sample_frames),
        (8000, 1, 480)
    );
    assert_eq!(output, fs::read(root().join("expected.f32le")).unwrap());
}
#[test]
fn omitted_and_empty_default_fields_decode_the_same_samples() {
    for name in [
        "missing-rate",
        "missing-channels",
        "missing-both",
        "empty-rate",
        "empty-channels",
        "empty-both",
    ] {
        check(name);
    }
}

#[test]
fn defaults_keep_sample_exact_interval_boundaries() {
    let expected = fs::read(root().join("expected.f32le")).unwrap();
    for name in ["control", "missing-both", "empty-both"] {
        let mut pcm = Vec::new();
        let stats = fvid_media::owned_matroska_pcm::decode_matroska_pcm(
            Cursor::new(fs::read(root().join(format!("{name}.mkv"))).unwrap()),
            &mut pcm,
            Some((
                std::time::Duration::from_millis(20),
                std::time::Duration::from_millis(40),
            )),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 160);
        assert_eq!(pcm, &expected[160 * 4..320 * 4]);
    }
}
#[test]
fn explicit_zero_is_not_reinterpreted_as_a_schema_default() {
    for name in ["zero-rate", "zero-channels"] {
        assert!(
            fvid_media::owned_webm::WebmReader::open(
                Cursor::new(fs::read(root().join(format!("{name}.mkv"))).unwrap()),
                Default::default()
            )
            .is_err(),
            "{name}"
        );
    }
}
#[test]
fn defaulted_audio_still_obeys_resource_limits() {
    for options in [
        fvid_control::CopyOptions {
            max_packet_bytes: 1,
            ..Default::default()
        },
        fvid_control::CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        },
    ] {
        assert!(
            fvid_media::owned_matroska_pcm::decode_matroska_pcm(
                Cursor::new(fs::read(root().join("missing-both.mkv")).unwrap()),
                &mut Vec::new(),
                None,
                &options
            )
            .is_err()
        );
    }
}
#[test]
fn empty_output_frequency_inherits_the_sampling_frequency() {
    check("empty-output-rate");
}

#[test]
fn packet_hard_stop_returns_the_exact_decoded_prefix() {
    let mut pcm = Vec::new();
    let stats = fvid_media::owned_matroska_pcm::decode_matroska_pcm(
        Cursor::new(fs::read(root().join("missing-both.mkv")).unwrap()),
        &mut pcm,
        None,
        &fvid_control::CopyOptions {
            max_packets: Some(2),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 320);
    assert_eq!(
        pcm,
        &fs::read(root().join("expected.f32le")).unwrap()[..320 * 4]
    );
}
