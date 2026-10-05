use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
#[test]
fn synthetic_opus_laces_accept_controlled_memory_and_refuse_before_pcm() {
    // The owned generator emits four DTX packets with different durations.
    // Reuse that short synthetic regression fixture, never private media.
    for name in [
        "opus-controlled-memory.mka",
        "opus-lace-variable-duration-block.mka",
        "opus-lace-xiph-group.mka",
        "opus-lace-ebml-group.mka",
        "opus-stereo.webm",
        "opus-surround.webm",
        "opus-silk.webm",
        "opus-hybrid.webm",
    ] {
        let source = fixture(&format!("playback-errors/{name}"));
        let bytes = std::fs::read(&source).unwrap();
        let mut expected = Vec::new();
        let reference = fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
            Cursor::new(&bytes), &mut expected, None, &Default::default(),
        ).unwrap();
        assert!(!expected.is_empty());
        let options = fvid_control::CopyOptions {
            max_controlled_bytes: Some(64 * 1024 * 1024),
            ..Default::default()
        };
        let mut actual = Vec::new();
        let accepted = fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
            Cursor::new(&bytes), &mut actual, None, &options,
        ).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(accepted.sample_frames, reference.sample_frames);
        let low = fvid_control::CopyOptions {
            max_controlled_bytes: Some(1024 * 1024),
            ..Default::default()
        };
        let mut refused = Vec::new();
        let error = fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
            Cursor::new(&bytes), &mut refused, None, &low,
        ).unwrap_err();
        assert!(error.to_string().contains("controlled memory budget exceeded"), "{error}");
        assert!(refused.is_empty());
        let output = std::env::temp_dir().join(format!("fvid-opus-budget-{}-{name}.wav", std::process::id()));
        let _ = std::fs::remove_file(&output);
        let error = fvid_media::decode_audio(&source, &output, &low).unwrap_err();
        assert!(error.to_string().contains("controlled memory budget exceeded"), "{error}");
        assert!(!output.exists());
        let stats = fvid_media::decode_audio(&source, &output, &options).unwrap();
        assert_eq!(stats.sample_frames, reference.sample_frames);
        assert_eq!(wave_payload(&output), expected);
        std::fs::remove_file(output).unwrap();
    }
}
fn wave_payload(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let count = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        if &bytes[at..at + 4] == b"data" {
            return bytes[at + 8..at + 8 + count].to_vec();
        }
        at += 8 + count + (count & 1);
    }
    panic!("missing PCM data")
}
fn reference(path: &Path) -> (Vec<u8>, u16) {
    let mut reader = fvid_media::owned_webm::WebmReader::open(
        Cursor::new(std::fs::read(path).unwrap()),
        Default::default(),
    )
    .unwrap();
    reader.scan_all().unwrap();
    let track = reader
        .tracks
        .iter()
        .find(|t| t.codec == "A_OPUS")
        .unwrap()
        .clone();
    let header = fvid_opus::OpusHead::parse(&track.codec_private).unwrap();
    let channels = usize::from(header.channel_count);
    assert!(channels <= 2);
    let mut decoder = header.decoder(48000).unwrap();
    let mut scratch = vec![0.0; 5760 * channels];
    let mut pcm = Vec::new();
    let mut padded = false;
    for index in 0..reader.packets.len() {
        let packet = reader.packets[index].clone();
        if packet.track != track.number {
            continue;
        }
        let bytes = reader.read_packet(index).unwrap();
        let frames = decoder.decode(&bytes, 5760, &mut scratch).unwrap();
        let padding = ((u128::from(packet.discard_padding_ns.unsigned_abs()) * 48000 + 500_000_000)
            / 1_000_000_000) as usize;
        let (begin, end) = if packet.discard_padding_ns < 0 {
            (padding, frames)
        } else {
            (0, frames - padding)
        };
        padded |= padding > 0;
        pcm.extend_from_slice(&scratch[begin * channels..end * channels]);
    }
    assert!(padded && track.codec_delay_ns > 0);
    let delay = ((u128::from(track.codec_delay_ns) * 48000 + 500_000_000) / 1_000_000_000) as usize;
    (
        pcm[delay * channels..]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect(),
        channels as u16,
    )
}
#[test]
fn opus_audio_export_matches_native_pcm_priming_padding_and_interval() {
    for name in [
        "shared-vp9-companions.mkv",
        "shared-vp9-untimed-companions.mkv",
        "shared-vp9-opus-input-rate.mkv",
        "shared-vp9-opus-unknown-rate.mkv",
    ] {
        let source = fixture(&format!("playback-errors/{name}"));
        let (expected, channels) = reference(&source);
        for (case, interval) in [None, Some((30001, 100003))].into_iter().enumerate() {
            let output = std::env::temp_dir().join(format!(
                "fvid-opus-export-{}-{name}-{case}.wav",
                std::process::id()
            ));
            let options = fvid_control::CopyOptions {
                streams: vec![2],
                ..Default::default()
            };
            let plan = fvid_media::plan_decode_audio(
                &source,
                &fvid_media::AudioDecodeTransform {
                    interval,
                    ..Default::default()
                },
                &options,
            )
            .unwrap();
            assert_eq!(plan.streams[0].codec, "opus");
            let stats =
                fvid_media::decode_audio_interval(&source, &output, interval, &options).unwrap();
            assert_eq!(
                (stats.sample_rate, stats.channels),
                (48000, i32::from(channels))
            );
            let frame = usize::from(channels) * 4;
            let total = expected.len() / frame;
            let (from, to) = interval.map_or((0, total), |(a, b)| {
                (
                    (a as u128 * 48000).div_ceil(1_000_000) as usize,
                    ((b as u128 * 48000).div_ceil(1_000_000) as usize).min(total),
                )
            });
            assert_eq!(stats.sample_frames, (to - from) as u64);
            assert_eq!(
                wave_payload(&output),
                expected[from * frame..to * frame],
                "{name}/{case}"
            );
            std::fs::remove_file(output).unwrap();
        }
        let mux =
            std::env::temp_dir().join(format!("fvid-opus-rate-{}-{name}.mkv", std::process::id()));
        fvid_media::transcode_lossless(
            &source,
            &mux,
            fvid_media::LosslessTransform {
                negate: Some("".into()),
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        let wav = mux.with_extension("wav");
        let stats = fvid_media::decode_audio(
            &mux,
            &wav,
            &fvid_control::CopyOptions {
                streams: vec![2],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.sample_rate, 48000);
        assert_eq!(wave_payload(&wav), expected);
        std::fs::remove_file(mux).unwrap();
        std::fs::remove_file(wav).unwrap();
    }
}
#[test]
fn opus_dsp_uses_the_existing_wave_pipeline() {
    let source = fixture("playback-errors/shared-vp9-companions.mkv");
    let base = std::env::temp_dir().join(format!("fvid-opus-dsp-{}", std::process::id()));
    let plain = base.with_extension("plain.wav");
    let actual = base.with_extension("actual.wav");
    let control = base.with_extension("control.wav");
    let options = fvid_control::CopyOptions {
        streams: vec![2],
        ..Default::default()
    };
    fvid_media::decode_audio(&source, &plain, &options).unwrap();
    let transform = fvid_media::AudioDecodeTransform {
        interval: Some((30001, 100003)),
        sample_rate: Some(16000),
        channels: Some(1),
        volume: Some(0.5),
    };
    let a = fvid_media::decode_audio_transformed(&source, &actual, transform, &options).unwrap();
    let b = fvid_media::decode_audio_transformed(&plain, &control, transform, &Default::default())
        .unwrap();
    assert_eq!(
        (a.sample_frames, a.sample_rate, a.channels),
        (b.sample_frames, b.sample_rate, b.channels)
    );
    assert_eq!(wave_payload(&actual), wave_payload(&control));
    let a = fvid_media::measure_loudness(&source, &options).unwrap();
    assert_eq!(a.backend, "owned container audio loudness");
    let b = fvid_media::measure_loudness(&plain, &Default::default()).unwrap();
    assert_eq!(
        (a.sample_frames, a.sample_rate, a.channels),
        (b.sample_frames, b.sample_rate, b.channels)
    );
    assert_eq!(
        (a.integrated_lufs, a.true_peak_dbfs, a.sample_peak_dbfs),
        (b.integrated_lufs, b.true_peak_dbfs, b.sample_peak_dbfs)
    );
    for p in [plain, actual, control] {
        std::fs::remove_file(p).unwrap();
    }
}

#[test]
fn opus_packet_limit_cancel_and_completion_keep_atomic_publication() {
    let source = fixture("playback-errors/shared-vp9-companions.mkv");
    let (full, channels) = reference(&source);
    let output = std::env::temp_dir().join(format!("fvid-opus-prefix-{}.wav", std::process::id()));
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let capture = events.clone();
    let published = output.clone();
    let options = fvid_control::CopyOptions {
        streams: vec![2],
        max_packets: Some(1),
        metadata_set: vec![("title".into(), "native opus".into())],
        progress: Some(fvid_control::ProgressHook::new(move |event| {
            if event.done {
                assert!(published.exists());
            }
            capture.lock().unwrap().push(event);
        })),
        ..Default::default()
    };
    let stats = fvid_media::decode_audio(&source, &output, &options).unwrap();
    assert_eq!(stats.decoded_frames, 1);
    let count = stats.sample_frames as usize * usize::from(channels) * 4;
    assert!(count > 0 && count < full.len());
    assert_eq!(wave_payload(&output), full[..count]);
    assert_eq!(events.lock().unwrap().iter().filter(|e| e.done).count(), 1);
    std::fs::remove_file(output).unwrap();
    let flag = fvid_control::CancelFlag::new();
    let trigger = flag.clone();
    let output = std::env::temp_dir().join(format!("fvid-opus-cancel-{}.wav", std::process::id()));
    let options = fvid_control::CopyOptions {
        streams: vec![2],
        cancel: Some(flag),
        progress: Some(fvid_control::ProgressHook::new(move |event| {
            assert!(!event.done);
            if event.packets > 0 {
                trigger.cancel();
            }
        })),
        ..Default::default()
    };
    let error = fvid_media::decode_audio(&source, &output, &options).unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert!(!output.exists());
}

#[test]
fn opus_file_export_accepts_speech_hybrid_and_surround_profiles() {
    for (name, channels) in [
        ("opus-mono", 1),
        ("opus-stereo", 2),
        ("opus-silk", 1),
        ("opus-hybrid", 1),
        ("opus-surround", 6),
        ("opus-positive-first", 1),
    ] {
        let source = fixture(&format!("playback-errors/{name}.webm"));
        let expected = std::fs::read(fixture(&format!("playback-errors/{name}.f32"))).unwrap();
        let output = std::env::temp_dir().join(format!(
            "fvid-opus-profiles-{}-{name}.wav",
            std::process::id()
        ));
        let stats =
            fvid_media::decode_audio_interval(&source, &output, None, &Default::default()).unwrap();
        assert_eq!(
            (stats.sample_rate, stats.channels),
            (48000, channels),
            "{name}"
        );
        assert_eq!(stats.sample_frames, 48000, "{name}");
        let actual = wave_payload(&output);
        assert_eq!(actual.len(), expected.len(), "{name}");
        let error = actual
            .as_chunks::<4>().0.iter()
            .zip(expected.as_chunks::<4>().0.iter())
            .map(|(a, b)| {
                (f32::from_le_bytes(*a)
                    - f32::from_le_bytes(*b))
                .abs()
            })
            .fold(0.0f32, f32::max);
        assert!(error < 0.00002, "{name}: PCM max error {error}");
        std::fs::remove_file(output).unwrap();
    }
}

#[test]
fn compressed_audio_mix_and_merge_match_decoded_wave_inputs() {
    for (name, source) in [
        ("opus", fixture("playback-errors/opus-stereo.webm")),
        ("aac", fixture("audio/aac-stereo.aac")),
    ] {
        let storage =
            std::env::temp_dir().join(format!("fvid-audio-mix-{}-{name}", std::process::id()));
        std::fs::create_dir(&storage).unwrap();
        let wave = storage.join("input.wav");
        fvid_media::decode_audio(&source, &wave, &Default::default()).unwrap();
        let compressed = vec![source.clone(), source];
        let decoded = vec![wave.clone(), wave];
        assert_eq!(
            fvid_media::plan_mix_audio(&compressed, &Default::default())
                .unwrap()
                .command,
            "mix-audio"
        );
        assert_eq!(
            fvid_media::plan_merge_audio(&compressed).unwrap().command,
            "merge-audio"
        );
        for merge in [false, true] {
            let actual = storage.join(format!("actual-{merge}.wav"));
            let expected = storage.join(format!("expected-{merge}.wav"));
            if merge {
                fvid_media::merge_audio(&compressed, &actual).unwrap();
                fvid_media::merge_audio(&decoded, &expected).unwrap();
            } else {
                fvid_media::mix_audio(&compressed, &actual, &Default::default()).unwrap();
                fvid_media::mix_audio(&decoded, &expected, &Default::default()).unwrap();
            }
            assert_eq!(wave_payload(&actual), wave_payload(&expected));
        }
        std::fs::remove_dir_all(storage).unwrap();
    }
}
