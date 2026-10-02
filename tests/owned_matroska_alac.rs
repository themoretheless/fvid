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
        assert!(fvid_media::owned_matroska_alac::decode_matroska_alac_pcm(
            Cursor::new(&bytes),
            &mut refused,
            None,
            &options
        )
        .is_err());
        assert!(refused.is_empty());
    }
    let cancel = fvid_control::CancelFlag::default();
    cancel.cancel();
    let options = fvid_control::CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    let mut output = Vec::new();
    assert!(fvid_media::owned_matroska_alac::decode_matroska_alac_pcm(
        Cursor::new(&bytes),
        &mut output,
        None,
        &options
    )
    .is_err());
    assert!(output.is_empty());
}

#[test]
fn public_matroska_alac_file_export_uses_owned_dsp_and_atomic_publication() {
    use std::io::{Read, Seek, SeekFrom};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-presentation-timeline.mka");
    let dir = std::env::temp_dir().join(format!("fvid-alac-file-export-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    let output = dir.join("out.wav");
    let published = output.clone();
    let options = fvid_control::CopyOptions {
        progress: Some(fvid_control::ProgressHook::new(move |event| {
            if event.done {
                assert!(published.exists());
            }
        })),
        ..Default::default()
    };
    let transform = fvid_media_info::AudioDecodeTransform {
        interval: Some((80, 160)),
        volume: Some(0.5),
        ..Default::default()
    };
    let stats =
        fvid_media::decode_audio_transformed(&source, &output, transform, &options).unwrap();
    assert_eq!(
        (
            stats.sample_frames,
            stats.decoded_frames,
            stats.sample_rate,
            stats.channels
        ),
        (4, 3, 48000, 1)
    );
    let mut file = std::fs::File::open(&output).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
    file.seek(SeekFrom::Start(info.data_offset)).unwrap();
    let mut pcm = vec![0; info.data_bytes as usize];
    file.read_exact(&mut pcm).unwrap();
    let expected: Vec<u8> = [7, 0, 9, 10]
        .iter()
        .flat_map(|&v| (v as f32 / 65536.0).to_le_bytes())
        .collect();
    assert_eq!(pcm, expected);
    let original = std::fs::read(&output).unwrap();
    assert!(fvid_media::decode_audio_transformed(&source, &output, transform, &options).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), original);
    let failed = dir.join("cancel.wav");
    let cancel = fvid_control::CancelFlag::default();
    let captured = cancel.clone();
    let options = fvid_control::CopyOptions {
        cancel: Some(cancel),
        progress: Some(fvid_control::ProgressHook::new(move |event| {
            assert!(!event.done);
            if event.packets > 0 {
                captured.cancel();
            }
        })),
        ..Default::default()
    };
    assert!(fvid_media::owned_audio_export::decode_audio_transformed(
        &source,
        &failed,
        Default::default(),
        &options
    )
    .is_err());
    assert!(!failed.exists());
}

#[test]
fn shared_spool_keeps_owned_adts_file_export_silent_samples() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/adts-concat-a.aac");
    let output = std::env::temp_dir().join(format!(
        "fvid-adts-spool-regression-{}.wav",
        std::process::id()
    ));
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _clean = Clean(output.clone());
    let stats = fvid_media::owned_audio_export::decode_audio(&source, &output, &Default::default())
        .unwrap();
    assert_eq!((stats.sample_frames, stats.decoded_frames), (1024, 1));
    let bytes = std::fs::read(output).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&bytes), None).unwrap();
    assert_eq!(info.sample_frames, 1024);
    assert!(
        bytes[info.data_offset as usize..info.data_offset as usize + info.data_bytes as usize]
            .iter()
            .all(|byte| *byte == 0)
    );
}
