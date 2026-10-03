use std::path::{Path, PathBuf};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let p = std::env::temp_dir().join(format!("fvid-lib-adts-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&p).unwrap();
    Dir(p)
}
#[test]
fn adts_file_dsp_matches_owned_decoded_wave_pipeline() {
    let directory = dir("dsp");
    for name in [
        "aac-mono-44k.aac",
        "aac-stereo.aac",
        "aac-51-active.aac",
        "aac-pce-wide8.aac",
    ] {
        let source = fixture(&format!("audio/{name}"));
        let bytes = std::fs::read(&source).unwrap();
        let reader = fvid_media::owned_aac::adts::StreamReader::open(bytes.as_slice()).unwrap();
        let config = reader.configuration();
        let mask = fvid_media::owned_aac::NativeAacDecoder::new(reader.audio_specific_config())
            .unwrap()
            .channel_mask();
        let mut pcm = Vec::new();
        let stats = fvid_media::owned_aac::decode_adts_pcm(
            bytes.as_slice(),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        let wave = directory.0.join(format!("{name}.wav"));
        let mut data = fvid_media::owned_wav::float_wav_header_with_mask(
            config.sample_rate,
            config.channels,
            stats.sample_frames,
            mask,
        )
        .unwrap();
        data.extend_from_slice(&pcm);
        std::fs::write(&wave, data).unwrap();
        let mut transforms = vec![
            Default::default(),
            fvid_media_info::AudioDecodeTransform {
                sample_rate: Some(16000),
                volume: Some(0.5),
                ..Default::default()
            },
        ];
        if name != "aac-pce-wide8.aac" {
            transforms.push(fvid_media_info::AudioDecodeTransform {
                channels: Some(1),
                volume: Some(0.5),
                ..Default::default()
            });
        }
        for (i, transform) in transforms.into_iter().enumerate() {
            let output = directory.0.join(format!("{name}-{i}-actual.wav"));
            let expected = directory.0.join(format!("{name}-{i}-expected.wav"));
            let result = fvid_media::owned_audio_export::decode_audio_transformed(
                &source,
                &output,
                transform,
                &Default::default(),
            )
            .unwrap();
            fvid_media::owned_audio_export::decode_audio_transformed(
                &wave,
                &expected,
                transform,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                std::fs::read(&output).unwrap(),
                std::fs::read(&expected).unwrap(),
                "{name}"
            );
            let public_output = directory.0.join(format!("{name}-{i}-public.wav"));
            let public_stats = fvid_media::decode_audio_transformed(
                &source,
                &public_output,
                transform,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                std::fs::read(&public_output).unwrap(),
                std::fs::read(&output).unwrap()
            );
            assert_eq!(public_stats.decoded_frames, result.decoded_frames);
            assert_eq!(result.decoded_frames, stats.decoded_frames);
            assert!(
                fvid_media::owned_audio_export::decode_audio(&source, &output, &Default::default())
                    .is_err()
            );
        }
    }
}
#[test]
fn float_wide_mask_is_preserved_by_shared_dsp() {
    let directory = dir("mask");
    let source = fixture("playback-errors/wave-float-wide-mask.wav");
    let video =
        fvid::native_probe::probe(&fixture("playback-errors/wave-float-wide-mask.y4m")).unwrap();
    assert_eq!(video.duration_us, Some(333));
    let output = directory.0.join("out.wav");
    fvid_media::owned_audio_export::decode_audio(&source, &output, &Default::default()).unwrap();
    let info =
        fvid_media::owned_wave_inspect::inspect(&mut std::fs::File::open(&output).unwrap(), None)
            .unwrap();
    assert_eq!(
        (info.channel_mask, info.channels, info.sample_frames),
        (0xff, 8, 16)
    );
}
#[test]
fn prefix_count_metadata_and_completion_are_not_internal_pcm_policies() {
    let directory = dir("prefix");
    let source = fixture("playback-errors/aac-packet-prefix.aac");
    let output = directory.0.join("out.wav");
    assert!(
        fvid_media::owned_audio_export::decode_audio(&source, &output, &Default::default())
            .unwrap_err()
            .contains("fill whole buffer")
    );
    assert!(!output.exists());
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = events.clone();
    let options = fvid_control::CopyOptions {
        max_packets: Some(3),
        max_packet_bytes: 1024,
        metadata_set: vec![("title".into(), "Synthetic AAC export".into())],
        progress: Some(fvid_control::ProgressHook::new(move |event| {
            captured.lock().unwrap().push(event)
        })),
        ..Default::default()
    };
    let result = fvid_media::owned_audio_export::decode_audio(&source, &output, &options).unwrap();
    assert_eq!((result.sample_frames, result.decoded_frames), (3072, 3));
    assert_eq!(
        fvid_media::owned_probe::probe_wave(&output)
            .unwrap()
            .metadata["title"],
        "Synthetic AAC export"
    );
    let events = events.lock().unwrap();
    assert_eq!(events.iter().filter(|e| e.done).count(), 1);
    assert!(events.last().unwrap().done);
    assert!(
        events
            .windows(2)
            .all(|e| e[0].packets <= e[1].packets && e[0].payload_bytes <= e[1].payload_bytes)
    );
}

#[test]
fn interval_stops_before_bad_tail_and_failures_never_publish() {
    let directory = dir("interval");
    let source = fixture("playback-errors/aac-packet-prefix.aac");
    let output = directory.0.join("range.wav");
    let transform = fvid_media_info::AudioDecodeTransform {
        interval: Some((10000, 50000)),
        ..Default::default()
    };
    let stats = fvid_media::owned_audio_export::decode_audio_transformed(
        &source,
        &output,
        transform,
        &Default::default(),
    )
    .unwrap();
    assert_eq!((stats.sample_frames, stats.decoded_frames), (1764, 3));
    let cancelled = directory.0.join("cancel.wav");
    let cancel = fvid_control::CancelFlag::default();
    let captured = cancel.clone();
    let options = fvid_control::CopyOptions {
        cancel: Some(cancel),
        progress: Some(fvid_control::ProgressHook::new(move |event| {
            assert!(!event.done);
            if event.packets == 1 {
                captured.cancel();
            }
        })),
        ..Default::default()
    };
    assert!(
        fvid_media::owned_audio_export::decode_audio(&source, &cancelled, &options)
            .unwrap_err()
            .contains("cancelled")
    );
    assert!(!cancelled.exists());
    for (name, options, expected) in [
        (
            "rss.wav",
            fvid_control::CopyOptions {
                max_rss_bytes: Some(1),
                ..Default::default()
            },
            "rss",
        ),
        (
            "packet.wav",
            fvid_control::CopyOptions {
                max_packet_bytes: 1,
                ..Default::default()
            },
            "packet exceeds budget",
        ),
        (
            "allocation.wav",
            fvid_control::CopyOptions {
                max_controlled_bytes: Some(1),
                ..Default::default()
            },
            "controlled memory budget exceeded",
        ),
    ] {
        let failed = directory.0.join(name);
        let error = fvid_media::owned_audio_export::decode_audio(&source, &failed, &options).unwrap_err();
        assert!(error.contains(expected), "{name}: expected {expected:?}, got {error:?}");
        assert!(!failed.exists());
    }
}
#[cfg(not(feature = "media"))]
#[test]
fn existing_public_library_entrypoint_decodes_adts_without_legacy() {
    let directory = dir("public");
    let source = fixture("audio/aac-mono-44k.aac");
    let output = directory.0.join("public.wav");
    let result = fvid_media::decode_audio(&source, &output, &Default::default()).unwrap();
    assert_eq!(
        (
            result.sample_frames,
            result.decoded_frames,
            result.sample_rate,
            result.channels
        ),
        (7168, 7, 44100, 1)
    );
}
