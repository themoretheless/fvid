use std::{io::Cursor, time::Duration};
#[test]
fn owned_matroska_aac_matches_frontend_pcm_and_intervals() {
    for name in [
        "aac-stereo.mka",
        "aac-960-48000.mka",
        "aac-960-8000.mka",
        "aac-960-96000.mka",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio")
            .join(name);
        let bytes = std::fs::read(path).unwrap();
        for interval in [
            None,
            Some((Duration::from_micros(2000), Duration::from_micros(12000))),
        ] {
            let mut expected = Vec::new();
            let front = fvid::native_media::decode_matroska_aac_pcm_interval(
                &bytes,
                &mut expected,
                interval,
            )
            .unwrap();
            let mut actual = Vec::new();
            let own = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                Cursor::new(&bytes),
                &mut actual,
                interval,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(actual, expected, "{name} {interval:?}");
            assert_eq!(
                (
                    own.sample_frames,
                    own.decoded_frames,
                    own.sample_rate,
                    own.channels
                ),
                (
                    front.sample_frames,
                    front.decoded_frames,
                    front.sample_rate,
                    front.channels
                )
            );
        }
        let mut prefix = Vec::new();
        let stats = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&bytes),
            &mut prefix,
            None,
            &fvid_control::CopyOptions {
                max_packets: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        // The stereo fixture discards its entire first priming packet.
        assert_eq!(stats.decoded_frames, 2);
        let mut full = Vec::new();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&bytes),
            &mut full,
            None,
            &Default::default(),
        )
        .unwrap();
        assert!(full.starts_with(&prefix));
    }
}
#[test]
fn synthetic_aac_matroska_public_file_export_uses_owned_dsp() {
    let dir = std::env::temp_dir().join(format!("fvid-owned-matroska-aac-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/adts-concat-a.aac");
    let matroska = dir.join("source.mka");
    fvid_media::owned_adts_remux::remux(&source, &matroska, &Default::default()).unwrap();
    let output = dir.join("output.wav");
    let stats = fvid_media::decode_audio_transformed(
        &matroska,
        &output,
        fvid_media_info::AudioDecodeTransform {
            interval: Some((2000, 10000)),
            volume: Some(0.5),
            channels: Some(2),
            sample_rate: Some(48000),
        },
        &Default::default(),
    )
    .unwrap();
    assert_eq!(
        (
            stats.sample_frames,
            stats.decoded_frames,
            stats.channels,
            stats.sample_rate
        ),
        (384, 1, 2, 48000)
    );
    let bytes = std::fs::read(&output).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&bytes), None).unwrap();
    assert_eq!(info.sample_frames, 384);
    assert!(bytes
        [info.data_offset as usize..(info.data_offset + u64::from(info.data_bytes)) as usize]
        .iter()
        .all(|b| *b == 0));
    assert!(fvid_media::decode_audio(&matroska, &output, &Default::default()).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
    let limited = dir.join("limited.wav");
    let options = fvid_control::CopyOptions {
        streams: vec![0],
        max_packets: Some(1),
        ..Default::default()
    };
    let prefix =
        fvid_media::owned_audio_export::decode_audio(&matroska, &limited, &options).unwrap();
    assert_eq!((prefix.sample_frames, prefix.decoded_frames), (1024, 1));
    let refused = dir.join("budget.wav");
    assert!(
        fvid_media::owned_audio_export::decode_audio(
            &matroska,
            &refused,
            &fvid_control::CopyOptions {
                max_controlled_bytes: Some(1024),
                ..Default::default()
            }
        )
        .unwrap_err()
        .contains("controlled memory budget exceeded")
    );
    assert!(!refused.exists());
    let admitted = dir.join("admitted.wav");
    fvid_media::decode_audio_transformed(
        &matroska,
        &admitted,
        fvid_media_info::AudioDecodeTransform {
            interval: Some((2000, 10000)),
            volume: Some(0.5),
            channels: Some(2),
            sample_rate: Some(48000),
        },
        &fvid_control::CopyOptions {
            max_controlled_bytes: Some(32 * 1024 * 1024),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(std::fs::read(&admitted).unwrap(), bytes);

    let cancelled = dir.join("cancelled.wav");
    let token = fvid_control::CancelFlag::default();
    token.cancel();
    assert!(fvid_media::owned_audio_export::decode_audio(
        &matroska,
        &cancelled,
        &fvid_control::CopyOptions {
            cancel: Some(token),
            ..Default::default()
        }
    )
    .unwrap_err()
    .contains("cancelled"));
    assert!(!cancelled.exists());
}
