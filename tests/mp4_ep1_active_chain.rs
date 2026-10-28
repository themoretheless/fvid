use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-ep1-active-chain")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
#[test]
fn active_chain_tracks_follow_configuration_epochs_and_independent_native_pcm() {
    use fvid_media::{
        owned_aac::NativeAacDecoder, owned_codec_config::aac_stream_descriptor,
        owned_mp4::Mp4Reader, owned_mp4_ep1::Group,
    };
    for c in manifest()["cases"].as_array().unwrap() {
        for video in ["video", "equivalent"] {
            let data = std::fs::read(root().join(c[video].as_str().unwrap())).unwrap();
            let mut reader =
                Mp4Reader::open_indexed(Cursor::new(data), Default::default()).unwrap();
            let selected = Group::automatic_audio_tracks(&reader).unwrap();
            assert_eq!(selected.len(), 1);
            let group = Group::new(reader.tracks(), selected[0]).unwrap();
            assert_eq!(group.stream_count(), 27);
            assert!(
                Group::admission_upper_bound(reader.tracks(), selected[0]).unwrap()
                    >= group.retained_payload_bytes().unwrap()
            );
            for epoch in 0..3 {
                let sample = reader.tracks()[selected[0]]
                    .description_for_sample(epoch * 6)
                    .unwrap();
                let mut decoder = NativeAacDecoder::new(
                    aac_stream_descriptor(&sample.configuration).unwrap().asc,
                )
                .unwrap();
                let mut pcm = Vec::new();
                for i in epoch * 6..epoch * 6 + 6 {
                    let units = group.read_frame(&mut reader, i).unwrap();
                    assert_eq!(units.len(), c["active"][epoch].as_u64().unwrap() as usize);
                    let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
                    let frame = decoder.decode_esc(&input).unwrap().unwrap();
                    pcm.extend(frame.samples);
                }
                let gold = std::fs::read(
                    root().join(c["golden"][usize::from(epoch == 1)].as_str().unwrap()),
                )
                .unwrap();
                assert_eq!(pcm.len() * 4, gold.len());
                for (a, b) in pcm.iter().zip(gold.chunks_exact(4)) {
                    assert!((*a - f32::from_le_bytes(b.try_into().unwrap())).abs() < 1e-7);
                }
            }
        }
    }
}

#[test]
fn missing_active_tail_and_internal_gap_remain_specific_refusals() {
    use fvid_media::{owned_mp4::Mp4Reader, owned_mp4_ep1::Group};
    for c in manifest()["cases"].as_array().unwrap() {
        for (key, index, error) in [
            ("malformed", 11, "missing simultaneous ESC access unit"),
            ("gap", 6, "active dependency chain has a gap"),
        ] {
            let data = std::fs::read(root().join(c[key].as_str().unwrap())).unwrap();
            let mut reader =
                Mp4Reader::open_indexed(Cursor::new(data), Default::default()).unwrap();
            let selected = Group::automatic_audio_tracks(&reader).unwrap()[0];
            let group = Group::new(reader.tracks(), selected).unwrap();
            for i in 0..6 {
                assert_eq!(group.read_frame(&mut reader, i).unwrap().len(), 4);
            }
            let e = group
                .read_frame(&mut reader, index)
                .err()
                .unwrap()
                .to_string();
            assert!(e.contains(error), "{key}: {e}");
            assert_eq!(group.read_frame(&mut reader, 0).unwrap().len(), 4);
        }
    }
}
#[cfg(feature = "player")]
fn play(
    source: &mut dyn fvid::audio::AudioStream,
    decoder: &mut Box<dyn fvid::audio::AudioDecode>,
) -> Vec<(fvid::audio::AudioPacket, u16)> {
    use fvid::audio::AudioStep;
    fn present(
        source: &mut dyn fvid::audio::AudioStream,
        f: fvid::audio::DecodedAudio,
        out: &mut Vec<(fvid::audio::AudioPacket, u16)>,
    ) {
        if let Some(p) = source.present_decoded(f.packet, f.source_pts).unwrap() {
            out.push((p, source.channels()));
        }
    }
    let mut out = Vec::new();
    while let Some(step) = source.next_step().unwrap() {
        match step {
            AudioStep::ResetDecoder => source.reset_decoder(decoder).unwrap(),
            AudioStep::ReconfigureDecoder => {
                while let Some(f) = decoder.finish_packet().unwrap() {
                    present(source, f, &mut out);
                }
                source.advance_configuration().unwrap();
                *decoder = source.make_decoder().unwrap();
            }
            AudioStep::Pcm(p) => out.push((p, source.channels())),
            AudioStep::Encoded(p) => {
                let stamp = source.decoder_stamp(&p);
                if let Some(f) = source
                    .decode_source_packet(decoder.as_mut(), &p, stamp)
                    .unwrap()
                {
                    present(source, f, &mut out);
                }
            }
        }
    }
    if source.drain_decoded_at_eof() {
        while let Some(f) = decoder.finish_packet().unwrap() {
            present(source, f, &mut out);
        }
    }
    source.validate_eof().unwrap();
    out
}
#[cfg(feature = "player")]
fn check(packets: &[(fvid::audio::AudioPacket, u16)], start: usize, gold: &[Vec<u8>]) {
    let mut cursor = start.min(17664);
    for (p, channels) in packets {
        assert_eq!(
            u128::from(p.pts) * u128::from(p.timebase_num) * 48000,
            cursor as u128 * u128::from(p.timebase_den)
        );
        let phase = if cursor < 5760 {
            0
        } else if cursor < 11904 {
            1
        } else {
            2
        };
        let origin = [0, 5760, 11904][phase];
        let end = [5760, 11904, 17664][phase];
        assert_eq!(*channels, [1, 8, 1][phase]);
        let stride = usize::from(*channels) * 4;
        assert!(p.data.len().is_multiple_of(stride));
        let frames = p.data.len() / stride;
        assert!(frames > 0 && cursor + frames <= end);
        let expected = &gold[usize::from(phase == 1)]
            [(cursor - origin) * stride..(cursor - origin + frames) * stride];
        for (a, b) in p.data.chunks_exact(4).zip(expected.chunks_exact(4)) {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7
            );
        }
        cursor += frames;
    }
    assert_eq!(cursor, 17664);
}
#[cfg(feature = "player")]
#[test]
fn active_chain_playback_and_native_geometry_survive_epoch_edge_seek_and_rewind() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        for key in ["video", "equivalent"] {
            let bytes = std::fs::read(root().join(c[key].as_str().unwrap())).unwrap();
            let mut source = fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(bytes),
                Default::default(),
            )
            .unwrap();
            assert_eq!(source.audio_tracks().len(), 1);
            let mut decoder = source.make_decoder().unwrap();
            let gold: Vec<Vec<u8>> = c["golden"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| std::fs::read(root().join(p.as_str().unwrap())).unwrap())
                .collect();
            check(&play(&mut source, &mut decoder), 0, &gold);
            for sample in [
                1, 5759, 5760, 5761, 11903, 11904, 11905, 17664, 17665, 7, 7, 0,
            ] {
                let ticks = (sample as u128 * u128::from(source.timescale()) / 48000) as i64;
                let landed = source.seek_to(ticks);
                source.reset_decoder(&mut decoder).unwrap();
                let begin = (u128::try_from(landed).unwrap() * 48000)
                    .div_ceil(u128::from(source.timescale())) as usize;
                check(&play(&mut source, &mut decoder), begin, &gold);
            }
            source.rewind();
            source.reset_decoder(&mut decoder).unwrap();
            check(&play(&mut source, &mut decoder), 0, &gold);
        }
    }
}
#[test]
fn normalized_active_chain_exports_match_destination_controls_and_cross_epoch_ranges() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-active-chain-export-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    fn wave(path: &std::path::Path, channels: u16) -> Vec<u8> {
        let bytes = std::fs::read(path).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&bytes), None).unwrap();
        assert_eq!((info.sample_rate, info.channels), (48000, channels));
        bytes[info.data_offset as usize..info.data_offset as usize + info.data_bytes as usize]
            .to_vec()
    }
    fn controls_agree(actual: &[u8], expected: &[u8]) {
        assert_eq!(actual.len(), expected.len());
        // Static exports use the established f32 routing path; changing geometry
        // uses a f64 matrix. Verify numerical equivalence across those paths.
        for (i, (a, b)) in actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .enumerate()
        {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!(
                (a - b).abs() < 1e-9,
                "static/epoch matrix sample {i}: {a} vs {b}"
            );
        }
    }
    for c in manifest()["cases"].as_array().unwrap() {
        for destination in [1u16, 2] {
            let mut controls = Vec::new();
            for (i, control) in c["controls"].as_array().unwrap().iter().enumerate() {
                let output = dir.join(format!("control-{i}-{destination}.wav"));
                fvid_media::decode_audio_transformed(
                    &root().join(control.as_str().unwrap()),
                    &output,
                    fvid_media::AudioDecodeTransform {
                        channels: Some(i32::from(destination)),
                        sample_rate: Some(48000),
                        ..Default::default()
                    },
                    &Default::default(),
                )
                .unwrap();
                controls.push(wave(&output, destination));
            }
            let expected = [
                controls[0].clone(),
                controls[1].clone(),
                controls[0].clone(),
            ]
            .concat();
            let stride = usize::from(destination) * 4;
            let mut variant_reference: Option<Vec<u8>> = None;
            for key in ["video", "equivalent"] {
                let source = root().join(c[key].as_str().unwrap());
                let data = std::fs::read(&source).unwrap();
                let full_path = dir.join(format!("{key}-{destination}-full.wav"));
                fvid_media::decode_audio_transformed(
                    &source,
                    &full_path,
                    fvid_media::AudioDecodeTransform {
                        channels: Some(i32::from(destination)),
                        sample_rate: Some(48000),
                        ..Default::default()
                    },
                    &fvid_control::CopyOptions {
                        max_controlled_bytes: Some(128 << 20),
                        ..Default::default()
                    },
                )
                .unwrap();
                let full = wave(&full_path, destination);
                controls_agree(&full, &expected);
                if let Some(reference) = &variant_reference {
                    assert!(full == *reference, "unused descriptions changed PCM");
                } else {
                    variant_reference = Some(full.clone());
                }
                if destination == 1 {
                    let mut public = Vec::new();
                    fvid::native_media::decode_mp4_aac_pcm(&data, &mut public).unwrap();
                    assert!(public == full, "public/owned default output differs");
                }
                for (crop, range) in [
                    None,
                    Some((119u64, 121u64)),
                    Some((247, 249)),
                    Some((119, 121)),
                ]
                .into_iter()
                .enumerate()
                {
                    let interval = range.map(|(a, b)| {
                        (
                            std::time::Duration::from_millis(a),
                            std::time::Duration::from_millis(b),
                        )
                    });
                    let wanted = if let Some((a, b)) = range {
                        &full[a as usize * 48 * stride..b as usize * 48 * stride]
                    } else {
                        &full
                    };
                    if destination == 1 {
                        let mut output = Vec::new();
                        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                            Cursor::new(&data),
                            &mut output,
                            interval,
                            &Default::default(),
                        )
                        .unwrap();
                        assert_eq!((stats.sample_rate, stats.channels), (48000, 1));
                        assert!(output == wanted, "exact interval/default PCM differs");
                    }
                    let output = dir.join(format!("{key}-{destination}-{crop}.wav"));
                    fvid_media::decode_audio_transformed(
                        &source,
                        &output,
                        fvid_media::AudioDecodeTransform {
                            channels: Some(i32::from(destination)),
                            sample_rate: Some(48000),
                            interval: range.map(|(a, b)| ((a * 1000) as i64, (b * 1000) as i64)),
                            ..Default::default()
                        },
                        &fvid_control::CopyOptions {
                            max_controlled_bytes: Some(128 << 20),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    assert!(
                        wave(&output, destination) == wanted,
                        "exact WAV crop differs"
                    );
                }
            }
        }
    }
}
