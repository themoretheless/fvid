use std::{io::Cursor, time::Duration};
#[test]
fn synthetic_alac_delay_padding_gap_and_interval_match_frontend() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-presentation-timeline.mka");
    let bytes = std::fs::read(&source).unwrap();
    let mut pcm = Vec::new();
    let stats = fvid_media::owned_matroska_alac::decode_matroska_alac_pcm(
        Cursor::new(&bytes),
        &mut pcm,
        None,
        &Default::default(),
    )
    .unwrap();
    let expected: Vec<u8> = [3, 4, 5, 6, 7, 0, 9, 10, 11, 12]
        .iter()
        .flat_map(|&s| (s as f32 / 32768.0).to_le_bytes())
        .collect();
    assert_eq!(pcm, expected);
    assert_eq!((stats.sample_frames, stats.decoded_frames), (10, 3));
    let output =
        std::env::temp_dir().join(format!("fvid-alac-timeline-{}.f32le", std::process::id()));
    fvid::native_export::export_audio_pcm_selected(
        &source, &output, None, 1.0, None, None, None, None, None,
    )
    .unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), pcm);
    std::fs::remove_file(output).unwrap();
    let mut window = Vec::new();
    let stats = fvid_media::owned_matroska_alac::decode_matroska_alac_pcm(
        Cursor::new(&bytes),
        &mut window,
        Some((Duration::from_micros(80), Duration::from_micros(160))),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(window, expected[4 * 4..8 * 4]);
    assert_eq!(stats.sample_frames, 4);
    let mut prefix = Vec::new();
    let options = fvid_control::CopyOptions {
        max_packets: Some(1),
        ..Default::default()
    };
    let stats = fvid_media::owned_matroska_alac::decode_matroska_alac_pcm(
        Cursor::new(&bytes),
        &mut prefix,
        None,
        &options,
    )
    .unwrap();
    assert_eq!(prefix, expected[..8]);
    assert_eq!(stats.decoded_frames, 1);
    for options in [
        fvid_control::CopyOptions {
            max_packet_bytes: 1,
            ..Default::default()
        },
        fvid_control::CopyOptions {
            max_controlled_bytes: Some(4096),
            ..Default::default()
        },
    ] {
        let mut refused = Vec::new();
        assert!(
            fvid_media::owned_matroska_alac::decode_matroska_alac_pcm(
                Cursor::new(&bytes),
                &mut refused,
                None,
                &options
            )
            .is_err()
        );
        assert!(refused.is_empty());
    }
    let cancel = fvid_control::CancelFlag::default();
    cancel.cancel();
    let options = fvid_control::CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    let mut output = Vec::new();
    assert!(
        fvid_media::owned_matroska_alac::decode_matroska_alac_pcm(
            Cursor::new(&bytes),
            &mut output,
            None,
            &options
        )
        .is_err()
    );
    assert!(output.is_empty());
}
