use std::{io::Cursor, time::Duration};
#[test]
fn library_mp4_audio_pcm_and_file_export_match_frontend_edits() {
    let dir = std::env::temp_dir().join(format!("fvid-mp4-owned-audio-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    for name in [
        "audio.mp4",
        "alac/mono-16.m4a",
        "alac/mono-24.m4a",
        "alac/stereo-16.m4a",
        "alac/stereo-24.m4a",
        "audio/aac-native-edit.m4a",
        "playback-errors/aac-gap-repeat.m4a",
    ] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let bytes = std::fs::read(&source).unwrap();
        let mut pcm = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        let front = dir.join("front.f32le");
        fvid::native_export::export_audio_pcm_selected(
            &source, &front, None, 1.0, None, None, None, None, None,
        )
        .unwrap();
        assert_eq!(std::fs::read(&front).unwrap(), pcm, "{name}");
        std::fs::remove_file(&front).unwrap();
        let interval = Some((Duration::from_micros(2000), Duration::from_micros(10000)));
        let mut window = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut window,
            interval,
            &Default::default(),
        )
        .unwrap();
        fvid::native_export::export_audio_pcm_selected(
            &source, &front, interval, 1.0, None, None, None, None, None,
        )
        .unwrap();
        assert_eq!(std::fs::read(&front).unwrap(), window, "{name}: interval");
        std::fs::remove_file(&front).unwrap();
        let output = dir.join("owned.wav");
        let written = fvid_media::decode_audio(&source, &output, &Default::default()).unwrap();
        assert_eq!(
            (written.sample_frames, written.decoded_frames),
            (stats.sample_frames, stats.decoded_frames)
        );
        let wave = std::fs::read(&output).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&wave), None).unwrap();
        assert_eq!(info.bits_per_sample, 32);
        assert_eq!(
            &wave[info.data_offset as usize..info.data_offset as usize + info.data_bytes as usize],
            pcm
        );
        std::fs::remove_file(&output).unwrap();
        let reference_source = dir.join("reference.wav");
        let mask =
            fvid_media::owned_pcm_channels::standard_mask(stats.channels).unwrap_or(0) as u32;
        let mut wave = fvid_media::owned_wav::float_wav_header_with_mask(
            stats.sample_rate,
            stats.channels,
            stats.sample_frames,
            mask,
        )
        .unwrap();
        wave.extend_from_slice(&pcm);
        std::fs::write(&reference_source, wave).unwrap();
        let reference = dir.join("reference-dsp.wav");
        let transform = fvid_media_info::AudioDecodeTransform {
            interval: Some((2000, 10000)),
            volume: Some(0.5),
            sample_rate: Some(44100),
            channels: Some(1),
        };
        fvid_media::owned_audio_export::decode_audio_transformed(
            &reference_source,
            &reference,
            transform,
            &Default::default(),
        )
        .unwrap();
        fvid_media::decode_audio_transformed(&source, &output, transform, &Default::default())
            .unwrap();
        assert_eq!(
            std::fs::read(&output).unwrap(),
            std::fs::read(&reference).unwrap(),
            "{name}: combined interval/DSP differs from full decoded timeline"
        );
        std::fs::remove_file(&output).unwrap();
        std::fs::remove_file(reference).unwrap();
        std::fs::remove_file(reference_source).unwrap();
    }
}

fn synthetic_resample_window(extension: &str) {
    let dir = std::env::temp_dir().join(format!(
        "fvid-resample-window-{extension}-{}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    let precise = extension == "pcm";
    let samples: Vec<f64> = (0..512)
        .map(|i| {
            (f64::from((i * 119) % 24001) - 12000.0) / 32768.0
                + if precise {
                    f64::from(i % 5) * 1e-10
                } else {
                    0.0
                }
        })
        .collect();
    let raw_expected: Vec<u8> = samples
        .iter()
        .flat_map(|&v| {
            if precise {
                v.to_le_bytes().to_vec()
            } else {
                (v as f32).to_le_bytes().to_vec()
            }
        })
        .collect();
    let reference_source = dir.join("reference.wav");
    let mut wave = fvid_media::owned_wav::float_wav_header_with_precision(
        48000,
        1,
        512,
        4,
        if precise { 64 } else { 32 },
    )
    .unwrap();
    wave.extend_from_slice(&raw_expected);
    std::fs::write(&reference_source, wave).unwrap();
    let transform = fvid_media_info::AudioDecodeTransform {
        interval: Some((2000, 6000)),
        sample_rate: Some(44100),
        ..Default::default()
    };
    let reference = dir.join("reference-result.wav");
    fvid_media::owned_audio_export::decode_audio_transformed(
        &reference_source,
        &reference,
        transform,
        &Default::default(),
    )
    .unwrap();
    let expected = std::fs::read(&reference).unwrap();
    for extension in [extension] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(if precise {
                "pcm64-resample-window.mka".to_string()
            } else {
                format!("alac-resample-window.{extension}")
            });
        let bytes = std::fs::read(&source).unwrap();
        let mut raw = Vec::new();
        let stats = if extension == "m4a" {
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&bytes),
                &mut raw,
                None,
                &Default::default(),
            )
            .unwrap()
        } else if precise {
            fvid_media::owned_matroska_pcm::decode_matroska_pcm_f64(
                Cursor::new(&bytes),
                &mut raw,
                None,
                &Default::default(),
            )
            .unwrap()
        } else {
            fvid_media::owned_matroska_alac::decode_matroska_alac_pcm(
                Cursor::new(&bytes),
                &mut raw,
                None,
                &Default::default(),
            )
            .unwrap()
        };
        assert_eq!((stats.sample_frames, stats.decoded_frames), (512, 4));
        assert_eq!(
            raw, raw_expected,
            "synthetic source must decode completely before testing DSP"
        );
        let output = dir.join(format!("{extension}.wav"));
        fvid_media::owned_audio_export::decode_audio_transformed(
            &source,
            &output,
            transform,
            &Default::default(),
        )
        .unwrap();
        let actual = std::fs::read(output).unwrap();
        assert!(
            actual == expected,
            "{extension}: interval resampling lost right-edge lookahead; first byte mismatch {:?}",
            actual.iter().zip(&expected).position(|(a, b)| a != b)
        );
    }
}

#[test]
fn synthetic_mp4_interval_resampling_keeps_future_input() {
    synthetic_resample_window("m4a");
}
#[test]
fn synthetic_matroska_interval_resampling_keeps_future_input() {
    synthetic_resample_window("mka");
}

#[test]
fn synthetic_precise_pcm_interval_resampling_keeps_future_input() {
    synthetic_resample_window("pcm");
}

#[test]
fn synthetic_mp4_audio_controls_preserve_publication() {
    use fvid_control::{CancelFlag, CopyOptions, ProgressHook};
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-resample-window.m4a");
    let bytes = std::fs::read(&source).unwrap();
    let mut full = Vec::new();
    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(&bytes),
        &mut full,
        None,
        &Default::default(),
    )
    .unwrap();
    let options = CopyOptions {
        max_packets: Some(1),
        ..Default::default()
    };
    let mut prefix = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(&bytes),
        &mut prefix,
        None,
        &options,
    )
    .unwrap();
    assert_eq!((stats.sample_frames, stats.decoded_frames), (128, 1));
    assert_eq!(prefix, full[..128 * 4]);
    for options in [
        CopyOptions {
            max_packet_bytes: 1,
            ..Default::default()
        },
        CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        },
    ] {
        let mut output = Vec::new();
        assert!(fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut output,
            None,
            &options,
        )
        .is_err());
        assert!(output.is_empty());
    }
    let dir = std::env::temp_dir().join(format!("fvid-mp4-controls-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    let destination = dir.join("prefix.wav");
    let stats = fvid_media::owned_audio_export::decode_audio_transformed(
        &source,
        &destination,
        Default::default(),
        &options,
    )
    .unwrap();
    assert_eq!((stats.sample_frames, stats.decoded_frames), (128, 1));
    let saved = std::fs::read(&destination).unwrap();
    assert!(fvid_media::owned_audio_export::decode_audio_transformed(
        &source,
        &destination,
        Default::default(),
        &options,
    )
    .is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), saved);
    let cancelled = dir.join("cancelled.wav");
    let cancel = CancelFlag::default();
    let captured = cancel.clone();
    let options = CopyOptions {
        cancel: Some(cancel),
        progress: Some(ProgressHook::new(move |event| {
            assert!(!event.done);
            if event.packets > 0 {
                captured.cancel();
            }
        })),
        ..Default::default()
    };
    assert!(fvid_media::owned_audio_export::decode_audio_transformed(
        &source,
        &cancelled,
        Default::default(),
        &options,
    )
    .is_err());
    assert!(!cancelled.exists());
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "temporary outputs must be cleaned"
    );
}
