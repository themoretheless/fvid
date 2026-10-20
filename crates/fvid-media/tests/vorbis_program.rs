use fvid_media::owned_ogg_vorbis::Program;
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn library_vorbis_program_preserves_chains_trims_and_exact_seek() {
    for (name, end, start) in [
        ("vorbis-timestamps-zero.ogg", 411, 0),
        ("vorbis-timestamps-initial-trim.ogg", 394, 0),
        ("vorbis-timestamps-positive-origin.ogg", 434, 23),
        ("vorbis-timestamps-chained.ogg", 805, 0),
        ("vorbis-timestamps-reconfigured.ogg", 805, 0),
        ("vorbis-timestamps-changing-blocks.ogg", 1189, 0),
    ] {
        let bytes = fixture(name);
        let mut program = Program::open(bytes.as_slice(), Default::default()).unwrap();
        assert_eq!(program.end_samples(), end);
        assert_eq!(program.sample_rate(), 8000);
        for target in [0, 19, 80, 411.min(end), 500.min(end), end, end + 1, 0] {
            assert_eq!(program.seek_to(target), target.min(end));
            let mut pcm = Vec::new();
            let stats = program.decode_pcm(&mut pcm).unwrap();
            assert_eq!(
                stats.sample_frames,
                (end - target.max(start).min(end)) as u64,
                "{name} target={target}"
            );
            assert_eq!(pcm.len(), stats.sample_frames as usize * 8);
            assert!(pcm.iter().all(|&b| b == 0));
        }
        program.rewind();
        let mut pcm = Vec::new();
        program.decode_pcm(&mut pcm).unwrap();
        assert_eq!(pcm.len(), (end - start) as usize * 8);
    }
}
#[test]
fn library_vorbis_tone_matches_native_packet_pcm_and_recovers_after_writer_failure() {
    let bytes = include_bytes!("../../../tests/fixtures/ogg/vorbis-stereo.ogg");
    let mut program = Program::open(bytes.as_slice(), Default::default()).unwrap();
    let mut decoder = program.native_decoder().unwrap();
    let mut baseline = Vec::new();
    for packet in &program.ogg().chains()[0].packets[3..] {
        for value in decoder.decode(&packet.data).unwrap() {
            baseline.extend_from_slice(&value.to_le_bytes());
        }
    }
    baseline.truncate(program.end_samples() as usize * 8);
    for target in [0, 1, 1782, 44100, 88256, 0] {
        program.seek_to(target);
        let mut pcm = Vec::new();
        let stats = program.decode_pcm(&mut pcm).unwrap();
        assert_eq!(pcm, baseline[target as usize * 8..]);
        assert_eq!(stats.sample_frames, 88256 - target as u64);
    }
    struct Failing;
    impl std::io::Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "synthetic Vorbis sink failure",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    program.rewind();
    assert!(matches!(
        program.decode_pcm(&mut Failing),
        Err(fvid_control::error::Error::Io(_))
    ));
    program.rewind();
    let mut pcm = Vec::new();
    program.decode_pcm(&mut pcm).unwrap();
    assert_eq!(pcm, baseline);
}

#[test]
fn vorbis_unbounded_input_limit_does_not_overflow_the_read_cap() {
    let bytes = fixture("vorbis-timestamps-zero.ogg");
    let mut program = Program::open(
        bytes.as_slice(),
        fvid_media::owned_ogg::Limits {
            file_bytes: usize::MAX,
            packets: usize::MAX,
        },
    )
    .unwrap();
    let mut pcm = Vec::new();
    assert_eq!(program.decode_pcm(&mut pcm).unwrap().sample_frames, 411);
}

#[test]
fn public_vorbis_wave_export_preserves_pcm_clock_and_exact_intervals() {
    use std::io::{Read, Seek, SeekFrom};
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory =
        Directory(std::env::temp_dir().join(format!("fvid-vorbis-export-{}", std::process::id())));
    std::fs::create_dir(&directory.0).unwrap();
    for name in [
        "vorbis-timestamps-zero.ogg",
        "vorbis-timestamps-initial-trim.ogg",
        "vorbis-timestamps-positive-origin.ogg",
        "vorbis-timestamps-chained.ogg",
        "vorbis-timestamps-reconfigured.ogg",
        "vorbis-timestamps-changing-blocks.ogg",
    ] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors")
            .join(name);
        let bytes = std::fs::read(&source).unwrap();
        let mut program = Program::open(bytes.as_slice(), Default::default()).unwrap();
        let mut baseline = Vec::new();
        program.decode_pcm(&mut baseline).unwrap();
        if name.contains("positive-origin") {
            baseline.splice(..0, vec![0; 23 * 8]);
        }
        let mut intervals = vec![None, Some((12_375, 48_125))];
        if program.end_samples() > 700 {
            intervals.push(Some((51_125, 91_375)));
        }
        if name.contains("positive-origin") {
            intervals.push(Some((125, 2000)));
        }
        for (index, interval) in intervals.into_iter().enumerate() {
            let output = directory.0.join(format!("{name}-{index}.wav"));
            fvid_media::decode_audio_transformed(
                &source,
                &output,
                fvid_media_info::AudioDecodeTransform {
                    interval,
                    ..Default::default()
                },
                &Default::default(),
            )
            .unwrap();
            let mut file = std::fs::File::open(output).unwrap();
            let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
            assert!(info.float);
            assert_eq!(info.channels, 2);
            assert_eq!(info.sample_rate, 8000);
            file.seek(SeekFrom::Start(info.data_offset)).unwrap();
            let mut pcm = vec![0; info.data_bytes as usize];
            file.read_exact(&mut pcm).unwrap();
            let (from, to) = interval
                .map(|(a, b)| {
                    (
                        (a as usize * 8000).div_ceil(1_000_000),
                        (b as usize * 8000).div_ceil(1_000_000),
                    )
                })
                .unwrap_or((0, baseline.len() / 8));
            assert_eq!(
                pcm,
                baseline[from * 8..to * 8],
                "{name} interval={interval:?}"
            );
        }
    }
}

#[test]
fn vorbis_file_controls_and_dsp_keep_pcm_and_publish_only_completed_results() {
    use std::io::{Read, Seek, SeekFrom};
    use std::sync::{Arc, Mutex};
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = Directory(
        std::env::temp_dir().join(format!("fvid-vorbis-controls-{}", std::process::id())),
    );
    std::fs::create_dir(&directory.0).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/ogg/vorbis-stereo.ogg");
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
    let cancel = fvid_control::CancelFlag::new();
    cancel.cancel();
    let output = directory.0.join("cancelled.wav");
    assert!(
        fvid_media::decode_audio(
            &source,
            &output,
            &fvid_control::CopyOptions {
                cancel: Some(cancel),
                ..Default::default()
            }
        )
        .unwrap_err()
        .contains("cancelled")
    );
    assert!(!output.exists());
    let cancel = fvid_control::CancelFlag::new();
    let captured = cancel.clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let output = directory.0.join("during.wav");
    assert!(
        fvid_media::decode_audio(
            &source,
            &output,
            &fvid_control::CopyOptions {
                cancel: Some(cancel),
                progress: Some(fvid_control::ProgressHook::new(move |event| {
                    seen.lock().unwrap().push(event);
                    if event.packets >= 1 {
                        captured.cancel();
                    }
                })),
                ..Default::default()
            }
        )
        .unwrap_err()
        .contains("cancelled")
    );
    assert!(!output.exists());
    assert!(!events.lock().unwrap().iter().any(|e| e.done));
    let read = |path: &std::path::Path| {
        let mut file = std::fs::File::open(path).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        file.seek(SeekFrom::Start(info.data_offset)).unwrap();
        let mut pcm = vec![0; info.data_bytes as usize];
        file.read_exact(&mut pcm).unwrap();
        pcm
    };
    let baseline = directory.0.join("baseline.wav");
    fvid_media::decode_audio(&source, &baseline, &Default::default()).unwrap();
    let raw = std::fs::read(&source).unwrap();
    let mut program = Program::open(raw.as_slice(), Default::default()).unwrap();
    let mut expected = Vec::new();
    program.decode_pcm(&mut expected).unwrap();
    assert_eq!(read(&baseline), expected);
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let output = directory.0.join("prefix.wav");
    let stats = fvid_media::decode_audio(
        &source,
        &output,
        &fvid_control::CopyOptions {
            max_packets: Some(3),
            max_controlled_bytes: Some(64 * 1024 * 1024),
            progress: Some(fvid_control::ProgressHook::new(move |event| {
                seen.lock().unwrap().push(event)
            })),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.decoded_frames, 3);
    assert_eq!(stats.sample_frames, 2048);
    assert_eq!(read(&output), expected[..2048 * 8]);
    let events = events.lock().unwrap();
    assert!(events.last().unwrap().done);
    assert_eq!(events.iter().filter(|e| e.done).count(), 1);
    drop(events);
    for rate in [32000, 48000] {
        let transform = fvid_media_info::AudioDecodeTransform {
            interval: Some((37_123, 191_789)),
            sample_rate: Some(rate),
            channels: Some(1),
            volume: Some(0.375),
        };
        let actual = directory.0.join(format!("ogg-{rate}.wav"));
        let expected = directory.0.join(format!("wave-{rate}.wav"));
        let options = fvid_control::CopyOptions {
            metadata_set: vec![("title".into(), "Owned export".into())],
            ..Default::default()
        };
        fvid_media::decode_audio_transformed(&source, &actual, transform, &options).unwrap();
        fvid_media::decode_audio_transformed(&baseline, &expected, transform, &options).unwrap();
        assert_eq!(read(&actual), read(&expected), "rate={rate}");
        let mut file = std::fs::File::open(actual).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        let tags = fvid_media::owned_wave_inspect::info_chunks(&mut file, &info, None).unwrap();
        assert!(tags.windows(12).any(|s| s == b"Owned export"));
    }
    let existing = directory.0.join("existing.wav");
    std::fs::write(&existing, b"preserve existing bytes").unwrap();
    assert!(fvid_media::decode_audio(&source, &existing, &Default::default()).is_err());
    assert_eq!(std::fs::read(existing).unwrap(), b"preserve existing bytes");
}

#[test]
fn parallel_vorbis_wave_uses_selected_source_stream() {
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = Directory(
        std::env::temp_dir().join(format!("fvid-vorbis-parallel-{}", std::process::id())),
    );
    std::fs::create_dir(&directory.0).unwrap();
    for (name, track, end) in [
        ("vorbis-timestamps-parallel.ogg", 1, 394),
        ("vorbis-timestamps-skeleton-parallel.ogg", 2, 394),
        ("vorbis-timestamps-parallel-chained.ogg", 1, 788),
    ] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors")
            .join(name);
        let output = directory.0.join(format!("{name}.wav"));
        let stats = fvid_media::decode_audio(
            &source,
            &output,
            &fvid_control::CopyOptions {
                streams: vec![track],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.sample_frames, end);
    }
}

#[test]
fn parallel_vorbis_library_selects_tracks_and_rejects_ambiguous_file_requests() {
    for (name, base) in [
        ("vorbis-timestamps-parallel.ogg", 0),
        ("vorbis-timestamps-skeleton-parallel.ogg", 1),
        ("vorbis-timestamps-parallel-chained.ogg", 0),
    ] {
        let bytes = fixture(name);
        for (nth, end) in [(0, 411), (1, 394)] {
            let end = if name.contains("parallel-chained") {
                end * 2
            } else {
                end
            };
            let mut program =
                Program::open_track(bytes.as_slice(), Default::default(), nth).unwrap();
            assert_eq!(program.audio_tracks().len(), 2);
            assert_eq!(program.audio_track_index(), nth);
            assert_eq!(program.end_samples(), end);
            for target in [0, 19, 80, 394.min(end), 411.min(end), end, 0] {
                program.seek_to(target);
                let mut pcm = Vec::new();
                assert_eq!(
                    program.decode_pcm(&mut pcm).unwrap().sample_frames,
                    (end - target) as u64
                );
            }
            let program =
                Program::open_source_track(bytes.as_slice(), Default::default(), Some(base + nth))
                    .unwrap();
            assert_eq!(program.audio_track_index(), nth);
        }
        assert!(
            Program::open_source_track(bytes.as_slice(), Default::default(), None)
                .err()
                .unwrap()
                .to_string()
                .contains("select exactly one")
        );
    }
}

#[test]
fn vorbis_discrete_channels_reach_the_library_and_wave_export() {
    use std::io::{Read, Seek, SeekFrom};
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory =
        Directory(std::env::temp_dir().join(format!("fvid-vorbis-wide-{}", std::process::id())));
    std::fs::create_dir(&directory.0).unwrap();
    for channels in [1, 9, 33, 255] {
        let name = format!("vorbis-channels-{channels}.ogg");
        let bytes = fixture(&name);
        let mut program = Program::open(bytes.as_slice(), Default::default()).unwrap();
        assert_eq!(program.channels(), channels);
        for target in [0, 19, 390, 411, 0] {
            assert_eq!(program.seek_to(target), target);
            let mut pcm = Vec::new();
            let stats = program.decode_pcm(&mut pcm).unwrap();
            assert_eq!(stats.sample_frames, (411 - target) as u64);
            assert_eq!(pcm.len(), (411 - target) as usize * channels as usize * 4);
            assert!(pcm.iter().all(|&b| b == 0));
        }
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors")
            .join(name);
        let output = directory.0.join(format!("{channels}.wav"));
        fvid_media::decode_audio_transformed(
            &source,
            &output,
            fvid_media_info::AudioDecodeTransform::default(),
            &Default::default(),
        )
        .unwrap();
        let mut file = std::fs::File::open(output).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        assert_eq!(info.channels, channels);
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.data_bytes, 411 * u32::from(channels) * 4);
        assert_eq!(info.channel_mask, if channels == 1 { 4 } else { 0 });
        file.seek(SeekFrom::Start(info.data_offset)).unwrap();
        let mut pcm = vec![0; info.data_bytes as usize];
        file.read_exact(&mut pcm).unwrap();
        assert!(pcm.iter().all(|&b| b == 0));
    }
}

#[test]
fn vorbis_channel_chains_keep_a_stable_output_and_exact_seek() {
    for (first, second, output, mask) in [
        (1, 2, 3, 7),
        (2, 1, 3, 7),
        (4, 6, 6, 0x3f),
        (7, 8, 9, 0x73f),
        (9, 33, 33, 0),
        (33, 9, 33, 0),
        (2, 255, 255, 0),
    ] {
        let name = format!("vorbis-channel-chain-{first}-{second}.ogg");
        let bytes = fixture(&name);
        let mut program = Program::open(bytes.as_slice(), Default::default()).unwrap();
        assert_eq!(program.channels(), output);
        assert_eq!(program.output_channel_mask(), mask);
        assert_eq!(program.end_samples(), 822);
        for target in [0, 19, 410, 411, 430, 821, 822, 0] {
            program.seek_to(target);
            let mut pcm = Vec::new();
            let stats = program.decode_pcm(&mut pcm).unwrap();
            assert_eq!(stats.channels, output);
            assert_eq!(stats.sample_frames, (822 - target) as u64);
            assert_eq!(pcm.len(), (822 - target) as usize * output as usize * 4);
            assert!(pcm.iter().all(|&b| b == 0));
        }
    }
}

#[test]
fn vorbis_channel_chain_routes_preserve_source_bytes_and_silence_only_missing_positions() {
    use fvid_media::owned_ogg_opus::{AudioPacket, AudioStep};
    for (first, second, routes) in [
        (1, 2, vec![vec![1], vec![0, 2]]),
        (2, 1, vec![vec![0, 2], vec![1]]),
        (4, 6, vec![vec![0, 2, 3, 4], vec![0, 1, 2, 3, 4, 5]]),
        (
            7,
            8,
            vec![vec![0, 2, 1, 7, 8, 6, 3], vec![0, 2, 1, 7, 8, 4, 5, 3]],
        ),
        (9, 33, vec![(0..9).collect(), (0..33).collect()]),
        (33, 9, vec![(0..33).collect(), (0..9).collect()]),
        (2, 255, vec![(0..2).collect(), (0..255).collect()]),
    ] {
        let bytes = fixture(&format!("vorbis-channel-chain-{first}-{second}.ogg"));
        let mut program = Program::open(bytes.as_slice(), Default::default()).unwrap();
        for target in [0, 19, 411, 430, 0] {
            program.seek_to(target);
            let mut checked = 0;
            while let Some(step) = program.next_step().unwrap() {
                if let AudioStep::Encoded(packet) = step {
                    let link = usize::from(packet.pts >= 411);
                    let route = &routes[link];
                    // Distinct NaN payload bit patterns prove byte routing
                    // without float arithmetic or value canonicalization.
                    let source: Vec<u8> = (0..route.len())
                        .flat_map(|ch| (0x7fc00001u32.wrapping_add(ch as u32)).to_le_bytes())
                        .collect();
                    let mut expected = vec![0u8; program.channels() as usize * 4];
                    for (ch, &position) in route.iter().enumerate() {
                        expected[position * 4..position * 4 + 4]
                            .copy_from_slice(&source[ch * 4..ch * 4 + 4]);
                    }
                    let data = source.repeat(packet.duration as usize);
                    if let Some(pcm) = program
                        .present_decoded(
                            AudioPacket {
                                data,
                                pts: 0,
                                timebase_num: 1,
                                timebase_den: 8000,
                            },
                            packet.pts,
                        )
                        .unwrap()
                    {
                        assert!(pcm.pts >= target as u64);
                        for frame in pcm.data.chunks_exact(expected.len()) {
                            assert_eq!(frame, expected);
                        }
                        checked += pcm.data.len() / expected.len();
                    }
                }
            }
            assert_eq!(checked, (822 - target) as usize);
        }
    }
}

#[test]
fn vorbis_channel_chain_wave_exports_keep_union_masks_and_cross_boundary_ranges() {
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = Directory(
        std::env::temp_dir().join(format!("fvid-vorbis-chain-export-{}", std::process::id())),
    );
    std::fs::create_dir(&directory.0).unwrap();
    for (first, second, channels, mask) in [
        (1, 2, 3, 7),
        (2, 1, 3, 7),
        (4, 6, 6, 0x3f),
        (7, 8, 9, 0x73f),
        (9, 33, 33, 0),
        (33, 9, 33, 0),
        (2, 255, 255, 0),
    ] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors")
            .join(format!("vorbis-channel-chain-{first}-{second}.ogg"));
        for (index, interval, frames) in [(0, None, 822), (1, Some((51_125, 60_125)), 72)] {
            let output = directory.0.join(format!("{first}-{second}-{index}.wav"));
            fvid_media::decode_audio_transformed(
                &source,
                &output,
                fvid_media_info::AudioDecodeTransform {
                    interval,
                    ..Default::default()
                },
                &Default::default(),
            )
            .unwrap();
            let mut file = std::fs::File::open(output).unwrap();
            let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
            assert_eq!(info.channels, channels);
            assert_eq!(info.channel_mask, mask);
            assert_eq!(info.sample_rate, 8000);
            assert_eq!(info.data_bytes, frames * u32::from(channels) * 4);
        }
    }
}

#[test]
fn vorbis_rate_changes_keep_the_first_output_clock_and_exact_frame_boundaries() {
    for (first, second, offset) in [
        (8000u32, 12000u32, 0i64),
        (12000, 8000, 0),
        (44100, 48000, 0),
        (48000, 44100, 0),
        (8000, 12000, -17),
        (8000, 12000, 23),
    ] {
        let bytes = fixture(&format!(
            "vorbis-rate-chain-{first}-{second}-offset-{offset}.ogg"
        ));
        let mut program = Program::open(bytes.as_slice(), Default::default()).unwrap();
        let end =
            411 + ((411 + offset) as u64 * u64::from(first)).div_ceil(u64::from(second)) as i64;
        assert_eq!(program.sample_rate(), first);
        assert_eq!(program.end_samples(), end);
        for target in [0, 19, 410, 411, 430, end - 1, end, 0] {
            program.seek_to(target);
            let mut pcm = Vec::new();
            let stats = program.decode_pcm(&mut pcm).unwrap();
            let gap = if offset > 0 {
                (offset as u64 * u64::from(first)).div_ceil(u64::from(second)) as i64
            } else {
                0
            };
            let skipped_gap = if target <= 411 {
                gap
            } else {
                (411 + gap - target).max(0)
            };
            assert_eq!(
                stats.sample_frames,
                (end - target - skipped_gap) as u64,
                "rates {first}/{second} offset {offset} target {target}"
            );
            assert_eq!(pcm.len(), stats.sample_frames as usize * 8);
            assert!(pcm.iter().all(|&b| b == 0));
        }
    }
}

#[test]
fn vorbis_three_links_carry_fractional_duration_without_one_sample_drift() {
    let bytes = fixture("vorbis-rate-chain-fractional-three.ogg");
    let mut program = Program::open(bytes.as_slice(), Default::default()).unwrap();
    assert_eq!(program.sample_rate(), 12000);
    assert_eq!(program.end_samples(), 1644);
    for target in [0, 410, 411, 1027, 1028, 1200, 1644, 0] {
        program.seek_to(target);
        let mut pcm = Vec::new();
        let stats = program.decode_pcm(&mut pcm).unwrap();
        assert_eq!(stats.sample_frames, (1644 - target) as u64);
        assert_eq!(pcm.len(), (1644 - target) as usize * 8);
    }
}

#[test]
fn vorbis_rate_tone_chains_match_native_pcm_resampling_and_exact_wave_ranges() {
    use std::io::{Read, Seek, SeekFrom, Write};
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = Directory(
        std::env::temp_dir().join(format!("fvid-vorbis-rate-tone-{}", std::process::id())),
    );
    std::fs::create_dir(&directory.0).unwrap();
    for (first, second) in [(44100u32, 32000u32), (32000, 44100)] {
        let decode = |rate| {
            let bytes = fixture(&format!("vorbis-rate-tone-{rate}.ogg"));
            let mut program = Program::open(bytes.as_slice(), Default::default()).unwrap();
            let mut pcm = Vec::new();
            program.decode_pcm(&mut pcm).unwrap();
            assert_eq!(pcm.len(), 88256 * 8);
            pcm
        };
        let mut baseline = decode(first);
        let mut resampler =
            fvid_media::owned_resample::Resampler::new(Vec::new(), second, first, 2).unwrap();
        resampler.write_all(&decode(second)).unwrap();
        resampler.finish().unwrap();
        baseline.extend(resampler.take_output());
        assert!(
            baseline
                .chunks_exact(4)
                .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 0.05)
        );
        let name = format!("vorbis-rate-tone-chain-{first}-{second}.ogg");
        let bytes = fixture(&name);
        let mut program = Program::open(bytes.as_slice(), Default::default()).unwrap();
        let end = baseline.len() / 8;
        assert_eq!(program.end_samples() as usize, end);
        for target in [0, 19, 88255, 88256, 88257, end - 1, end, 0] {
            program.seek_to(target as i64);
            let mut pcm = Vec::new();
            program.decode_pcm(&mut pcm).unwrap();
            assert_eq!(
                pcm,
                baseline[target * 8..],
                "rates {first}/{second} target {target}"
            );
        }
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors")
            .join(name);
        let boundary_us = (88256u64 * 1_000_000 / u64::from(first)) as i64;
        for (index, interval) in [
            (0, None),
            (1, Some((37_123, 191_789))),
            (2, Some((boundary_us - 5000, boundary_us + 5000))),
        ] {
            let output = directory.0.join(format!("{first}-{second}-{index}.wav"));
            fvid_media::decode_audio_transformed(
                &source,
                &output,
                fvid_media_info::AudioDecodeTransform {
                    interval,
                    ..Default::default()
                },
                &Default::default(),
            )
            .unwrap();
            let mut file = std::fs::File::open(output).unwrap();
            let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
            assert_eq!(
                (info.channels, info.sample_rate, info.channel_mask),
                (2, first, 3)
            );
            file.seek(SeekFrom::Start(info.data_offset)).unwrap();
            let mut pcm = vec![0; info.data_bytes as usize];
            file.read_exact(&mut pcm).unwrap();
            let (from, to) = interval
                .map(|(a, b)| {
                    (
                        ((a as u64 * u64::from(first)).div_ceil(1_000_000)) as usize,
                        ((b as u64 * u64::from(first)).div_ceil(1_000_000)) as usize,
                    )
                })
                .unwrap_or((0, end));
            assert_eq!(
                pcm,
                baseline[from * 8..to * 8],
                "WAVE rates {first}/{second} range {interval:?}"
            );
        }
    }
}

#[test]
fn vorbis_rate_limited_export_flushes_only_the_decoded_prefix() {
    use std::io::{Read, Seek, SeekFrom, Write};
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = Directory(
        std::env::temp_dir().join(format!("fvid-vorbis-rate-limit-{}", std::process::id())),
    );
    std::fs::create_dir(&directory.0).unwrap();
    let bytes = fixture("vorbis-rate-tone-44100.ogg");
    let mut first = Program::open(bytes.as_slice(), Default::default()).unwrap();
    let first_packets = first.ogg().chains()[0].packets.len() - 3;
    let mut expected = Vec::new();
    first.decode_pcm(&mut expected).unwrap();
    let bytes = fixture("vorbis-rate-tone-32000.ogg");
    let mut second = Program::open(bytes.as_slice(), Default::default()).unwrap();
    let mut pcm = Vec::new();
    second.decode_pcm(&mut pcm).unwrap();
    let mut resampler =
        fvid_media::owned_resample::Resampler::new(Vec::new(), 32000, 44100, 2).unwrap();
    resampler.write_all(&pcm[..2048 * 8]).unwrap();
    resampler.finish().unwrap();
    expected.extend(resampler.take_output());
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/vorbis-rate-tone-chain-44100-32000.ogg");
    let output = directory.0.join("limited.wav");
    fvid_media::decode_audio_transformed(
        &source,
        &output,
        Default::default(),
        &fvid_control::CopyOptions {
            max_packets: Some(first_packets as u64 + 3),
            ..Default::default()
        },
    )
    .unwrap();
    let mut file = std::fs::File::open(output).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
    file.seek(SeekFrom::Start(info.data_offset)).unwrap();
    let mut actual = vec![0; info.data_bytes as usize];
    file.read_exact(&mut actual).unwrap();
    assert_eq!(
        actual.len(),
        expected.len(),
        "all decoded prefix samples must be published"
    );
    assert_eq!(actual, expected);
}

#[test]
fn vorbis_rate_and_channel_changes_combine_in_pcm_and_wave() {
    let bytes = fixture("vorbis-rate-channel-chain.ogg");
    let mut program = Program::open(bytes.as_slice(), Default::default()).unwrap();
    assert_eq!(
        (
            program.sample_rate(),
            program.channels(),
            program.output_channel_mask()
        ),
        (8000, 3, 7)
    );
    assert_eq!(program.end_samples(), 685);
    for target in [0, 19, 410, 411, 430, 684, 685, 0] {
        program.seek_to(target);
        let mut pcm = Vec::new();
        let stats = program.decode_pcm(&mut pcm).unwrap();
        assert_eq!(stats.sample_frames, (685 - target) as u64);
        assert_eq!(pcm.len(), (685 - target) as usize * 12);
        assert!(pcm.iter().all(|&b| b == 0));
    }
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory =
        Directory(std::env::temp_dir().join(format!("fvid-vorbis-shape-{}", std::process::id())));
    std::fs::create_dir(&directory.0).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/vorbis-rate-channel-chain.ogg");
    for (index, interval, frames) in [(0, None, 685), (1, Some((51_125, 60_125)), 72)] {
        let output = directory.0.join(format!("{index}.wav"));
        fvid_media::decode_audio_transformed(
            &source,
            &output,
            fvid_media_info::AudioDecodeTransform {
                interval,
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        let mut file = std::fs::File::open(output).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.channel_mask),
            (8000, 3, 7)
        );
        assert_eq!(info.data_bytes, frames * 12);
    }
}

#[test]
fn vorbis_rate_change_packet_durations_use_the_program_timescale() {
    let bytes = fixture("vorbis-rate-chain-8000-12000-offset-0.ogg");
    let program = Program::open(bytes.as_slice(), Default::default()).unwrap();
    assert_eq!(program.timescale(), 8000);
    assert_eq!(
        program.packet_times().skip(6).collect::<Vec<_>>(),
        [
            (411, 0),
            (411, 53),
            (464, 53),
            (517, 54),
            (571, 85),
            (656, 53)
        ]
    );
}
