use fvid_media::{
    owned_aac::NativeAacDecoder, owned_codec_config::aac_stream_descriptor, owned_mp4::Mp4Reader,
    owned_mp4_ep1::Group,
};
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-ep1")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn selected<R>(reader: &Mp4Reader<R>) -> usize
where
    R: std::io::Read + std::io::Seek,
{
    reader
        .tracks()
        .iter()
        .position(|t| {
            t.handler == *b"soun"
                && aac_stream_descriptor(&t.configuration)
                    .unwrap()
                    .depends_on
                    .is_none()
        })
        .unwrap()
}
#[test]
fn real_mp4_ep1_groups_decode_exact_bit_ends_rational_clocks_and_independent_pcm() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 8);
    for case in m["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(&bytes), Default::default()).unwrap();
        let track = selected(&reader);
        let group = Group::new(reader.tracks(), track).unwrap();
        assert_eq!(group.root_track(), track);
        assert_eq!(
            group.stream_count(),
            case["streams"].as_u64().unwrap() as usize
        );
        let asc = aac_stream_descriptor(&reader.tracks()[track].configuration)
            .unwrap()
            .asc;
        let mut decoder = NativeAacDecoder::new(asc).unwrap();
        let golden = std::fs::read(root().join(case["golden"].as_str().unwrap())).unwrap();
        let gold: Vec<f32> = golden
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        let mut full = Vec::new();
        let mut nonbyte = false;
        let mut nonzero_excluded = false;
        for i in 0..3 {
            let units = group.read_frame(&mut reader, i).unwrap();
            for unit in &units {
                if unit.bit_len % 8 != 0 {
                    nonbyte = true;
                    nonzero_excluded |=
                        unit.data.last().unwrap() & ((1 << (8 - unit.bit_len % 8)) - 1) != 0;
                }
            }
            let mut input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
            input.reverse();
            let frame = decoder.decode_esc(&input).unwrap().unwrap();
            assert_eq!(
                frame.pts,
                reader.tracks()[track].samples.get(i).unwrap().pts
            );
            full.extend(frame.samples);
        }
        assert!(
            nonbyte && nonzero_excluded,
            "{} did not exercise ignored transport bits",
            case["name"]
        );
        assert_eq!(full.len(), gold.len());
        let error = full
            .iter()
            .zip(&gold)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(error < 1e-7, "{} PCM error {error}", case["name"]);
        assert!(group.read_frame(&mut reader, 3).is_err());
        // Stateless container navigation plus real decoder preroll. This is
        // group-read acceptance, not AudioStream/device playback qualification.
        let frame_samples = full.len() / 3;
        for target in [2, 1, 0, 2, 0] {
            decoder.reset();
            let mut suffix = Vec::new();
            for i in 0..3 {
                let units = group.read_frame(&mut reader, i).unwrap();
                let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
                let frame = decoder.decode_esc(&input).unwrap().unwrap();
                if i >= target {
                    suffix.extend(frame.samples);
                }
            }
            assert_eq!(
                suffix,
                full[target * frame_samples..],
                "{} read/rewind {target}",
                case["name"]
            );
        }
    }
}
#[test]
fn missing_mistimed_forked_or_ambiguous_es_groups_have_specific_errors() {
    for case in manifest()["malformed"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(bytes), Default::default()).unwrap();
        let track = selected(&reader);
        let expected = case["error"].as_str().unwrap();
        match Group::new(reader.tracks(), track) {
            Err(error) => assert!(
                error.to_string().contains(expected),
                "{}: {error}",
                case["mode"]
            ),
            Ok(group) => {
                let index = if case["mode"] == "missing" { 2 } else { 0 };
                let error = match group.read_frame(&mut reader, index) {
                    Ok(_) => panic!("{} accepted", case["mode"]),
                    Err(e) => e,
                };
                assert!(
                    error.to_string().contains(expected),
                    "{}: {error}",
                    case["mode"]
                );
            }
        }
    }
}
#[test]
fn ep1_mp4_exports_use_complete_dependency_groups_and_exact_intervals() {
    use std::time::Duration;
    let m = manifest();
    let cases = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| !c["name"].as_str().unwrap().ends_with("-signed"))
        .chain(m["exports"].as_array().unwrap());
    for case in cases {
        let bytes = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let indexed = Mp4Reader::open_indexed(Cursor::new(&bytes), Default::default()).unwrap();
        let track = selected(&indexed);
        let golden = std::fs::read(root().join(case["golden"].as_str().unwrap())).unwrap();
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        let mut public = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut public).unwrap();
        assert_eq!(owned, public, "{} bindings", case["name"]);
        assert_eq!(owned.len(), golden.len());
        let error = owned
            .chunks_exact(4)
            .zip(golden.chunks_exact(4))
            .map(|(a, b)| {
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
            })
            .fold(0f32, f32::max);
        assert!(error < 1e-7, "{} export PCM {error}", case["name"]);
        for (from, to) in [(1u64, 7u64), (0, 13), (35, 67), (40, 83), (1, 7)] {
            let interval = Some((Duration::from_millis(from), Duration::from_millis(to)));
            let rate = case["rate"].as_u64().unwrap();
            let stride = case["channels"].as_u64().unwrap() as usize * 4;
            let begin = ((from * rate).div_ceil(1000) as usize * stride).min(owned.len());
            let end = ((to * rate).div_ceil(1000) as usize * stride).min(owned.len());
            let mut range = Vec::new();
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&bytes),
                &mut range,
                interval,
                &fvid_control::CopyOptions {
                    streams: vec![track],
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(range, owned[begin..end], "{} interval", case["name"]);
            let mut public = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm_interval(&bytes, &mut public, interval).unwrap();
            assert_eq!(range, public);
        }
    }
}
#[test]
fn ep1_signed_source_clocks_crop_exact_presentation_intervals() {
    use std::time::Duration;
    for case in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"].as_str().unwrap().ends_with("-signed"))
    {
        let bytes = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let golden = std::fs::read(root().join(case["golden"].as_str().unwrap())).unwrap();
        let rate = case["rate"].as_u64().unwrap();
        let stride = case["channels"].as_u64().unwrap() as usize * 4;
        let skip = (37 * rate / 24000) as usize;
        for (from, to) in [(5u64, 37u64), (0, 17), (5, 37)] {
            let interval = Some((Duration::from_millis(from), Duration::from_millis(to)));
            let mut owned = Vec::new();
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&bytes),
                &mut owned,
                interval,
                &Default::default(),
            )
            .unwrap();
            let begin = ((from * rate).div_ceil(1000) as usize + skip) * stride;
            let end = ((to * rate).div_ceil(1000) as usize + skip) * stride;
            let expected = &golden[begin..end];
            assert_eq!(owned.len(), expected.len());
            let error = owned
                .chunks_exact(4)
                .zip(expected.chunks_exact(4))
                .map(|(a, b)| {
                    (f32::from_le_bytes(a.try_into().unwrap())
                        - f32::from_le_bytes(b.try_into().unwrap()))
                    .abs()
                })
                .fold(0f32, f32::max);
            assert!(error < 1e-7, "{} signed interval {error}", case["name"]);
            let mut public = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm_interval(&bytes, &mut public, interval).unwrap();
            assert_eq!(owned, public);
        }
    }
}
#[test]
fn ep1_export_controls_admit_the_complete_group_and_count_logical_aus() {
    use fvid_control::{CancelFlag, CopyOptions, ProgressHook};
    use std::sync::{Arc, Mutex};
    for case in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| !c["name"].as_str().unwrap().ends_with("-signed"))
    {
        let bytes = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(&bytes), Default::default()).unwrap();
        let track = selected(&reader);
        let group = Group::new(reader.tracks(), track).unwrap();
        let bound = Group::admission_upper_bound(reader.tracks(), track).unwrap();
        assert!(bound >= group.retained_payload_bytes().unwrap());
        let mut checks = 0;
        let error = match group.read_frame_checked(&mut reader, 0, || {
            checks += 1;
            if checks == 3 {
                Err("synthetic mid-group cancellation")
            } else {
                Ok(())
            }
        }) {
            Ok(_) => panic!("cancelled group was returned"),
            Err(e) => e,
        };
        assert!(error.to_string().contains("mid-group cancellation"));
        assert_eq!(
            group.read_frame(&mut reader, 0).unwrap().len(),
            group.stream_count()
        );
        let mut expected_bytes = 0u64;
        for i in 0..3 {
            expected_bytes += group
                .read_frame(&mut reader, i)
                .unwrap()
                .iter()
                .map(|u| u.data.len() as u64)
                .sum::<u64>();
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let options = CopyOptions {
            max_controlled_bytes: Some(128 << 20),
            progress: Some(ProgressHook::new(move |e| captured.lock().unwrap().push(e))),
            ..Default::default()
        };
        let mut full = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut full,
            None,
            &options,
        )
        .unwrap();
        let events = events.lock().unwrap();
        let last = events.last().unwrap();
        assert!(events.iter().all(|e| !e.done)); // Raw decoding does not publish output.
        assert_eq!(last.packets, 3);
        assert_eq!(last.payload_bytes, expected_bytes);
        drop(events);
        let mut prefix = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut prefix,
            None,
            &CopyOptions {
                max_packets: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(prefix, full[..full.len() / 3]);
        for options in [
            CopyOptions {
                max_controlled_bytes: Some(1),
                ..Default::default()
            },
            CopyOptions {
                max_packet_bytes: 1,
                ..Default::default()
            },
        ] {
            let mut output = Vec::new();
            let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&bytes),
                &mut output,
                None,
                &options,
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("budget"), "{error}");
            assert!(output.is_empty());
        }
        let cancel = CancelFlag::default();
        let hook_cancel = cancel.clone();
        let mut output = Vec::new();
        let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut output,
            None,
            &CopyOptions {
                cancel: Some(cancel),
                progress: Some(ProgressHook::new(move |e| {
                    if e.packets == 1 {
                        hook_cancel.cancel();
                    }
                })),
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("cancelled"), "{error}");
    }
}
#[test]
fn ep1_file_exports_preserve_group_pcm_edits_and_interval_gain() {
    let dir = std::env::temp_dir().join(format!("fvid-ep1-file-export-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(std::path::PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    let m = manifest();
    let cases = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| !c["name"].as_str().unwrap().ends_with("-signed"))
        .chain(m["exports"].as_array().unwrap());
    for case in cases {
        let source = root().join(case["video"].as_str().unwrap());
        let gold = std::fs::read(root().join(case["golden"].as_str().unwrap())).unwrap();
        let rate = case["rate"].as_u64().unwrap();
        let stride = case["channels"].as_u64().unwrap() as usize * 4;
        for interval in [false, true] {
            let path = dir.join(format!("{}-{interval}.wav", case["name"].as_str().unwrap()));
            let transform = if interval {
                fvid_media_info::AudioDecodeTransform {
                    interval: Some((1000, 17000)),
                    volume: Some(0.5),
                    ..Default::default()
                }
            } else {
                Default::default()
            };
            let stats = fvid_media::decode_audio_transformed(
                &source,
                &path,
                transform,
                &fvid_control::CopyOptions {
                    max_controlled_bytes: Some(128 << 20),
                    ..Default::default()
                },
            )
            .unwrap();
            let bytes = std::fs::read(path).unwrap();
            let info =
                fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&bytes), None).unwrap();
            assert_eq!(stats.sample_rate, i32::try_from(rate).unwrap());
            assert_eq!(stats.channels, case["channels"].as_u64().unwrap() as i32);
            let expected = if interval {
                &gold[(rate.div_ceil(1000) as usize) * stride
                    ..((17 * rate).div_ceil(1000) as usize) * stride]
            } else {
                &gold[..]
            };
            let actual = &bytes
                [info.data_offset as usize..info.data_offset as usize + info.data_bytes as usize];
            assert_eq!(actual.len(), expected.len());
            let error = actual
                .chunks_exact(4)
                .zip(expected.chunks_exact(4))
                .map(|(a, b)| {
                    (f32::from_le_bytes(a.try_into().unwrap())
                        - f32::from_le_bytes(b.try_into().unwrap())
                            * if interval { 0.5 } else { 1.0 })
                    .abs()
                })
                .fold(0f32, f32::max);
            assert!(
                error < 1e-7,
                "{} file export interval {interval}: {error}",
                case["name"]
            );
        }
    }
}
#[cfg(feature = "player")]
fn play_stream(
    reader: &mut dyn fvid::audio::AudioStream,
    decoder: &mut Box<dyn fvid::audio::AudioDecode>,
) -> Vec<u8> {
    use fvid::audio::AudioStep;
    let mut pcm = Vec::new();
    while let Some(step) = reader.next_step().unwrap() {
        match step {
            AudioStep::ResetDecoder => reader.reset_decoder(decoder).unwrap(),
            AudioStep::ReconfigureDecoder => panic!("unexpected changing EP1 setup"),
            AudioStep::Pcm(packet) => pcm.extend(packet.data),
            AudioStep::Encoded(packet) => {
                let stamp = reader.decoder_stamp(&packet);
                if let Some(frame) = reader
                    .decode_source_packet(decoder.as_mut(), &packet, stamp)
                    .unwrap()
                {
                    if let Some(p) = reader
                        .present_decoded(frame.packet, frame.source_pts)
                        .unwrap()
                    {
                        pcm.extend(p.data);
                    }
                }
            }
        }
    }
    if reader.drain_decoded_at_eof() {
        while let Some(frame) = decoder.finish_packet().unwrap() {
            if let Some(p) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                pcm.extend(p.data);
            }
        }
    }
    reader.validate_eof().unwrap();
    pcm
}
#[cfg(feature = "player")]
#[test]
fn grouped_ep1_audiostream_playback_rewind_and_seek_preserve_the_complete_pcm() {
    use fvid::audio::AudioStream;
    let m = manifest();
    let cases = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| !c["name"].as_str().unwrap().ends_with("-signed"))
        .chain(m["exports"].as_array().unwrap());
    for case in cases {
        let bytes = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        assert_eq!(
            reader.audio_tracks().len(),
            1,
            "{} dependent ES listed as a program",
            case["name"]
        );
        let mut decoder = reader.make_decoder().unwrap();
        let first = reader.next_packet().unwrap().unwrap();
        let error = match decoder.decode_packet(&first.data, first.pts, first.duration as u64) {
            Ok(_) => panic!("bare root AU accepted as a complete EP1 group"),
            Err(e) => e,
        };
        assert!(error.to_string().contains("separately associated ESC"));
        reader.reset_decoder(&mut decoder).unwrap();
        let mut altered = fvid::audio::EncodedPacket {
            data: first.data.clone(),
            pts: first.pts,
            duration: first.duration,
        };
        altered.data[0] ^= 1;
        let stamp = reader.decoder_stamp(&first);
        let error = match reader.decode_source_packet(decoder.as_mut(), &altered, stamp) {
            Ok(_) => panic!("mismatched root AU accepted"),
            Err(e) => e,
        };
        assert!(error.to_string().contains("disagrees with indexed AU"));
        assert!(
            reader
                .decode_source_packet(decoder.as_mut(), &first, stamp)
                .unwrap()
                .is_some()
        );
        reader.rewind();
        reader.reset_decoder(&mut decoder).unwrap();
        let full = play_stream(&mut reader, &mut decoder);
        let mut expected = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(full, expected, "{} initial playback", case["name"]);
        let width = case["channels"].as_u64().unwrap() as usize * 4;
        for sample in [13, 0, 7, 7, (full.len() / width) as i64, 13, 0] {
            let ticks = (i128::from(sample) * i128::from(reader.timescale())
                / i128::from(reader.sample_rate())) as i64;
            let landed = reader.seek_to(ticks);
            reader.reset_decoder(&mut decoder).unwrap();
            let actual = play_stream(&mut reader, &mut decoder);
            let begin = (u128::try_from(landed).unwrap() * u128::from(reader.sample_rate()))
                .div_ceil(u128::from(reader.timescale())) as usize
                * width;
            assert_eq!(
                actual,
                full[begin.min(full.len())..],
                "{} seek {sample}",
                case["name"]
            );
        }
        reader.rewind();
        reader.reset_decoder(&mut decoder).unwrap();
        assert_eq!(play_stream(&mut reader, &mut decoder), full);
    }
}
