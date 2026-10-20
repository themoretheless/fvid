use std::path::{Path, PathBuf};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors")
        .join(name)
}
#[test]
fn mixed_codec_file_export_includes_every_link() {
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory =
        Directory(std::env::temp_dir().join(format!("fvid-codec-links-{}", std::process::id())));
    std::fs::create_dir(&directory.0).unwrap();
    for (name, rate, frames) in [("vorbis-opus", 8000, 1297), ("opus-vorbis", 48000, 7777)] {
        let output = directory.0.join(format!("{name}.wav"));
        fvid_media::decode_audio_transformed(
            &fixture(&format!("ogg-codec-chain-{name}.ogg")),
            &output,
            Default::default(),
            &Default::default(),
        )
        .unwrap();
        let mut file = std::fs::File::open(output).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.channel_mask),
            (rate, 6, 0x3f)
        );
        assert_eq!(
            info.data_bytes,
            frames * 24,
            "all codec links must be presented"
        );
    }
}

fn read_pcm(path: &Path) -> Vec<u8> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
    file.seek(SeekFrom::Start(info.data_offset)).unwrap();
    let mut bytes = vec![0; info.data_bytes as usize];
    file.read_exact(&mut bytes).unwrap();
    bytes
}
fn baseline(reverse: bool) -> Vec<u8> {
    use std::io::Write;
    let bytes = std::fs::read(fixture("opus-surround-presentation.opus")).unwrap();
    let mut opus =
        fvid_media::owned_ogg_opus::Program::open(bytes.as_slice(), Default::default()).unwrap();
    let mut pcm = Vec::new();
    opus.decode_pcm(&mut pcm).unwrap();
    assert_eq!(pcm.len(), 5311 * 24);
    assert!(
        pcm.chunks_exact(4)
            .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 0.001)
    );
    if reverse {
        pcm.extend(vec![0; 2466 * 24]);
        pcm
    } else {
        let mut engine =
            fvid_media::owned_resample::Resampler::new(Vec::new(), 48000, 8000, 6).unwrap();
        engine.write_all(&pcm).unwrap();
        engine.finish().unwrap();
        let mut expected = vec![0; 411 * 24];
        expected.extend(engine.take_output());
        expected
    }
}
#[test]
fn mixed_codec_native_pcm_seek_and_writer_recovery_preserve_both_sources() {
    for reverse in [false, true] {
        let name = if reverse {
            "opus-vorbis"
        } else {
            "vorbis-opus"
        };
        let expected = baseline(reverse);
        let bytes = std::fs::read(fixture(&format!("ogg-codec-chain-{name}.ogg"))).unwrap();
        let mut program = fvid_media::owned_ogg_audio_program::Program::open(
            bytes.as_slice(),
            Default::default(),
            0,
        )
        .unwrap();
        assert_eq!(program.codec(), "A_OGG/VORBIS_OPUS");
        assert_eq!(program.channels(), 6);
        assert_eq!(program.output_channel_mask(), 0x3f);
        let end = expected.len() / 24;
        let boundary = if reverse { 5311 } else { 411 };
        assert_eq!(program.end_samples() as usize, end);
        for target in [
            0,
            19,
            boundary - 1,
            boundary,
            boundary + 1,
            end - 1,
            end,
            end + 1,
            0,
        ] {
            assert_eq!(program.seek_to(target as i64), target.min(end) as i64);
            let mut actual = Vec::new();
            let mut cursor = target.min(end) as u64;
            while let Some(pcm) = program.next_pcm().unwrap() {
                assert_eq!(pcm.pts, cursor);
                assert_eq!(pcm.timebase_den, program.sample_rate());
                cursor += pcm.data.len() as u64 / 24;
                actual.extend(pcm.data);
            }
            assert_eq!(cursor, end as u64);
            assert_eq!(actual, expected[target.min(end) * 24..]);
        }
        struct Fail;
        impl std::io::Write for Fail {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("synthetic mixed codec sink failure"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        program.rewind();
        assert!(program.decode_pcm(&mut Fail).is_err());
        let mut actual = Vec::new();
        program.decode_pcm(&mut actual).unwrap();
        assert_eq!(actual, expected);
    }
}
#[test]
fn linked_codec_selection_follows_global_bos_order_and_carries_phase() {
    for (name, index, rate, channels, mask, end) in [
        ("vorbis-opus-vorbis", 0, 8000, 6, 0x3f, 1708),
        ("opus-vorbis-opus", 0, 48000, 6, 0x3f, 13088),
        ("discrete-opus", 0, 8000, 9, 0, 1297),
        ("multiplex-swap", 0, 48000, 6, 0x3f, 7777),
        ("multiplex-swap", 1, 8000, 6, 0x3f, 1297),
    ] {
        let bytes = std::fs::read(fixture(&format!("ogg-codec-chain-{name}.ogg"))).unwrap();
        let mut program = fvid_media::owned_ogg_audio_program::Program::open(
            bytes.as_slice(),
            Default::default(),
            index,
        )
        .unwrap();
        assert_eq!(
            (
                program.sample_rate(),
                program.channels(),
                program.output_channel_mask(),
                program.end_samples()
            ),
            (rate, channels, mask, end)
        );
        for target in [0, 19, 411, 777, end - 1, end, 0] {
            program.seek_to(target);
            let mut cursor = target as u64;
            while let Some(pcm) = program.next_pcm().unwrap() {
                assert_eq!(pcm.pts, cursor, "{name} index {index} target {target}");
                cursor += pcm.data.len() as u64 / (u64::from(channels) * 4);
            }
            assert_eq!(cursor, end as u64);
        }
    }
}

#[test]
fn mixed_codec_wave_ranges_and_global_source_selection_match_native_pcm() {
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory =
        Directory(std::env::temp_dir().join(format!("fvid-codec-ranges-{}", std::process::id())));
    std::fs::create_dir(&directory.0).unwrap();
    for (label, source_index, reverse) in [
        ("vorbis-opus", 0, false),
        ("opus-vorbis", 0, true),
        ("multiplex-swap", 0, true),
        ("multiplex-swap", 1, false),
    ] {
        let expected = baseline(reverse);
        let rate = if reverse { 48000u64 } else { 8000 };
        let boundary = if reverse { 5311u64 } else { 411 };
        let source = fixture(&format!("ogg-codec-chain-{label}.ogg"));
        let at = (boundary * 1_000_000 / rate) as i64;
        for (index, interval) in [
            (0, None),
            (1, Some((at - 1000, at + 5000))),
            (2, Some((at + 1000, at + 9000))),
        ] {
            let output = directory
                .0
                .join(format!("{label}-{source_index}-{index}.wav"));
            fvid_media::decode_audio_transformed(
                &source,
                &output,
                fvid_media_info::AudioDecodeTransform {
                    interval,
                    ..Default::default()
                },
                &fvid_control::CopyOptions {
                    streams: vec![source_index],
                    ..Default::default()
                },
            )
            .unwrap();
            let (from, to) = interval
                .map(|(a, b)| {
                    (
                        ((a as u64 * rate).div_ceil(1_000_000)) as usize,
                        ((b as u64 * rate).div_ceil(1_000_000)) as usize,
                    )
                })
                .unwrap_or((0, expected.len() / 24));
            assert_eq!(
                read_pcm(&output),
                expected[from * 24..to * 24],
                "{label} source {source_index} range {interval:?}"
            );
        }
    }
    let output = directory.0.join("ambiguous.wav");
    assert!(
        fvid_media::decode_audio(
            &fixture("ogg-codec-chain-multiplex-swap.ogg"),
            &output,
            &Default::default()
        )
        .unwrap_err()
        .contains("select exactly one")
    );
    assert!(!output.exists());
}
#[test]
fn mixed_codec_export_controls_bound_work_and_preserve_atomic_publication() {
    use std::{
        io::Write,
        sync::{Arc, Mutex},
    };
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory =
        Directory(std::env::temp_dir().join(format!("fvid-codec-controls-{}", std::process::id())));
    std::fs::create_dir(&directory.0).unwrap();
    let source = fixture("ogg-codec-chain-vorbis-opus.ogg");
    for (name, options, message) in [
        (
            "memory.wav",
            fvid_control::CopyOptions {
                max_controlled_bytes: Some(1),
                ..Default::default()
            },
            "controlled memory budget exceeded",
        ),
        (
            "packet.wav",
            fvid_control::CopyOptions {
                max_packet_bytes: 1,
                ..Default::default()
            },
            "packet exceeds byte limit",
        ),
    ] {
        let output = directory.0.join(name);
        let error = fvid_media::decode_audio(&source, &output, &options).unwrap_err();
        assert!(error.contains(message), "{error}");
        assert!(!output.exists());
    }
    for packet in [0, 1, 7] {
        let cancel = fvid_control::CancelFlag::new();
        if packet == 0 {
            cancel.cancel();
        }
        let flag = cancel.clone();
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = events.clone();
        let output = directory.0.join(format!("cancel-{packet}.wav"));
        let error = fvid_media::decode_audio(
            &source,
            &output,
            &fvid_control::CopyOptions {
                cancel: Some(cancel),
                progress: Some(fvid_control::ProgressHook::new(move |event| {
                    seen.lock().unwrap().push(event);
                    if event.packets >= packet {
                        flag.cancel();
                    }
                })),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert!(!output.exists());
        assert!(!events.lock().unwrap().iter().any(|e| e.done));
    }
    // Six Vorbis audio packets followed by two Opus packets; no later source
    // packet may be read, but the already decoded filter tail must be emitted.
    let bytes = std::fs::read(fixture("opus-surround-presentation.opus")).unwrap();
    let mut opus =
        fvid_media::owned_ogg_opus::Program::open(bytes.as_slice(), Default::default()).unwrap();
    let mut decoder =
        fvid_media::owned_opus::OpusDecoder::new(opus.extra_data(), 48000, 6).unwrap();
    let mut prefix = Vec::new();
    for _ in 0..2 {
        let Some(fvid_media::owned_ogg_opus::AudioStep::Encoded(packet)) =
            opus.next_step().unwrap()
        else {
            panic!("expected Opus packet")
        };
        let data = decoder.decode_bytes(&packet.data).unwrap();
        if let Some(pcm) = opus
            .present_decoded(
                fvid_media::owned_ogg_opus::AudioPacket {
                    data,
                    pts: 0,
                    timebase_num: 1,
                    timebase_den: 48000,
                },
                packet.pts,
            )
            .unwrap()
        {
            prefix.extend(pcm.data);
        }
    }
    let mut engine =
        fvid_media::owned_resample::Resampler::new(Vec::new(), 48000, 8000, 6).unwrap();
    engine.write_all(&prefix).unwrap();
    engine.finish().unwrap();
    let mut expected = vec![0; 411 * 24];
    expected.extend(engine.take_output());
    let output = directory.0.join("prefix.wav");
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let stats = fvid_media::decode_audio(
        &source,
        &output,
        &fvid_control::CopyOptions {
            max_packets: Some(8),
            max_controlled_bytes: Some(64 * 1024 * 1024),
            progress: Some(fvid_control::ProgressHook::new(move |event| {
                seen.lock().unwrap().push(event)
            })),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.decoded_frames, 8);
    assert_eq!(stats.sample_frames, expected.len() as u64 / 24);
    assert_eq!(read_pcm(&output), expected);
    assert_eq!(events.lock().unwrap().iter().filter(|e| e.done).count(), 1);
    let existing = directory.0.join("existing.wav");
    std::fs::write(&existing, b"keep existing bytes").unwrap();
    assert!(fvid_media::decode_audio(&source, &existing, &Default::default()).is_err());
    assert_eq!(std::fs::read(existing).unwrap(), b"keep existing bytes");
}

#[test]
fn mixed_codec_initial_trim_and_positive_origin_keep_the_exact_gap() {
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory =
        Directory(std::env::temp_dir().join(format!("fvid-codec-origin-{}", std::process::id())));
    std::fs::create_dir(&directory.0).unwrap();
    for (name, end, gap) in [
        ("initial-trim-opus", 1280, 0),
        ("positive-origin-opus", 1320, 23),
    ] {
        let source = fixture(&format!("ogg-codec-chain-{name}.ogg"));
        let bytes = std::fs::read(&source).unwrap();
        let mut program = fvid_media::owned_ogg_audio_program::Program::open(
            bytes.as_slice(),
            Default::default(),
            0,
        )
        .unwrap();
        assert_eq!(program.end_samples(), end);
        for target in [0, 19, 23, 24, 393, 433, 434, 500, end - 1, end, 0] {
            program.seek_to(target);
            let mut cursor = target.max(gap) as u64;
            while let Some(pcm) = program.next_pcm().unwrap() {
                assert_eq!(pcm.pts, cursor);
                cursor += pcm.data.len() as u64 / 24;
            }
            assert_eq!(cursor, end as u64);
        }
        let output = directory.0.join(format!("{name}.wav"));
        fvid_media::decode_audio(&source, &output, &Default::default()).unwrap();
        let expected = baseline(false);
        let mut raw = vec![0; ((end - 886) as usize) * 24];
        raw.extend_from_slice(&expected[411 * 24..]);
        assert_eq!(read_pcm(&output), raw);
        if gap != 0 {
            let output = directory.0.join("inside-gap.wav");
            fvid_media::decode_audio_transformed(
                &source,
                &output,
                fvid_media_info::AudioDecodeTransform {
                    interval: Some((125, 2000)),
                    ..Default::default()
                },
                &Default::default(),
            )
            .unwrap();
            assert_eq!(read_pcm(&output), vec![0; 15 * 24]);
        }
    }
}

#[test]
fn both_nonzero_codecs_preserve_pcm_through_44100_48000_changes() {
    use std::io::Write;
    let bytes = std::fs::read(fixture("vorbis-rate-tone-44100.ogg")).unwrap();
    let mut vorbis =
        fvid_media::owned_ogg_vorbis::Program::open(bytes.as_slice(), Default::default()).unwrap();
    let mut stereo = Vec::new();
    vorbis.decode_pcm(&mut stereo).unwrap();
    let mut six = Vec::new();
    for frame in stereo.chunks_exact(8) {
        six.extend_from_slice(frame);
        six.extend([0; 16]);
    }
    let bytes = std::fs::read(fixture("opus-surround-presentation.opus")).unwrap();
    let mut opus =
        fvid_media::owned_ogg_opus::Program::open(bytes.as_slice(), Default::default()).unwrap();
    let mut surround = Vec::new();
    opus.decode_pcm(&mut surround).unwrap();
    for reverse in [false, true] {
        let (name, rate, input, second, boundary, first) = if reverse {
            (
                "tone-opus-vorbis",
                48000,
                44100,
                &six,
                5311,
                surround.clone(),
            )
        } else {
            (
                "tone-vorbis-opus",
                44100,
                48000,
                &surround,
                88256,
                six.clone(),
            )
        };
        let mut engine =
            fvid_media::owned_resample::Resampler::new(Vec::new(), input, rate, 6).unwrap();
        engine.write_all(second).unwrap();
        engine.finish().unwrap();
        let mut expected = first;
        expected.extend(engine.take_output());
        assert!(
            expected[..boundary * 24]
                .chunks_exact(4)
                .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 0.01)
        );
        assert!(
            expected[boundary * 24..]
                .chunks_exact(4)
                .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 0.001)
        );
        let bytes = std::fs::read(fixture(&format!("ogg-codec-chain-{name}.ogg"))).unwrap();
        let mut program = fvid_media::owned_ogg_audio_program::Program::open(
            bytes.as_slice(),
            Default::default(),
            0,
        )
        .unwrap();
        let end = expected.len() / 24;
        for target in [0, boundary - 1, boundary, boundary + 1, end - 1, end, 0] {
            program.seek_to(target as i64);
            let mut actual = Vec::new();
            program.decode_pcm(&mut actual).unwrap();
            assert_eq!(actual, expected[target * 24..], "{name} target {target}");
        }
    }
}
