use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-ep1-grouped-sbr")
}
fn manifest() -> serde_json::Value {
    let mut base: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for corpus in ["mp4-ep1-sbr-batches", "mp4-ep1-grouped-core"] {
        let mut extra: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root().join(format!("../{corpus}/manifest.json"))).unwrap(),
        )
        .unwrap();
        for c in extra["cases"].as_array_mut().unwrap() {
            for key in ["video", "control", "golden"] {
                let path = c[key].as_str().unwrap();
                if !path.starts_with("../") {
                    c[key] = format!("../{corpus}/{path}").into();
                }
            }
            for bad in c["malformed"].as_array_mut().unwrap() {
                bad["video"] = format!("../{corpus}/{}", bad["video"].as_str().unwrap()).into();
            }
            base["cases"].as_array_mut().unwrap().push(c.clone());
        }
    }
    base
}
#[test]
fn grouped_ld_sbr_pairs_decode_to_independent_pcm() {
    use fvid_media::{
        owned_aac::NativeAacDecoder, owned_codec_config::aac_stream_descriptor,
        owned_mp4::Mp4Reader, owned_mp4_ep1::Group,
    };
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 722);
    for c in m["cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(data), Default::default()).unwrap();
        let selected = Group::automatic_audio_tracks(&reader).unwrap();
        assert_eq!(selected.len(), 1);
        let group = Group::new(reader.tracks(), selected[0]).unwrap();
        assert_eq!(
            group.stream_count(),
            c["streams"].as_u64().unwrap() as usize
        );
        let mut decoder = NativeAacDecoder::new(
            aac_stream_descriptor(&reader.tracks()[selected[0]].configuration)
                .unwrap()
                .asc,
        )
        .unwrap();
        let mut pcm = Vec::new();
        for i in 0..6 {
            let units = group.read_frame(&mut reader, i).unwrap();
            let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
            let checkpoint = decoder.checkpoint();
            let frame = decoder
                .decode_esc(&input)
                .unwrap_or_else(|e| panic!("{}: {e}", c["name"]))
                .unwrap();
            decoder.restore(&checkpoint).unwrap();
            assert_eq!(Some(&frame), decoder.decode_esc(&input).unwrap().as_ref());
            pcm.extend(frame.samples);
        }
        let gold = std::fs::read(root().join(c["golden"].as_str().unwrap())).unwrap();
        assert_eq!(pcm.len() * 4, gold.len());
        for (a, b) in pcm.iter().zip(gold.chunks_exact(4)) {
            assert!((*a - f32::from_le_bytes(b.try_into().unwrap())).abs() < 1e-7);
        }
    }
}

#[test]
fn grouped_pair_truncation_extra_bits_and_late_crc_errors_preserve_history() {
    use fvid_media::{
        owned_aac::NativeAacDecoder, owned_codec_config::aac_stream_descriptor,
        owned_mp4::Mp4Reader, owned_mp4_ep1::Group,
    };
    for c in manifest()["cases"].as_array().unwrap() {
        for bad in c["malformed"].as_array().unwrap() {
            let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
            let control = std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap();
            let broken = std::fs::read(root().join(bad["video"].as_str().unwrap())).unwrap();
            let mut reader =
                Mp4Reader::open_indexed(Cursor::new(data), Default::default()).unwrap();
            let selected = Group::automatic_audio_tracks(&reader).unwrap()[0];
            let group = Group::new(reader.tracks(), selected).unwrap();
            let asc = aac_stream_descriptor(&reader.tracks()[selected].configuration)
                .unwrap()
                .asc
                .to_vec();
            let mut decoder = NativeAacDecoder::new(&asc).unwrap();
            let mut reference =
                Mp4Reader::open_indexed(Cursor::new(control), Default::default()).unwrap();
            let index = Group::automatic_audio_tracks(&reference).unwrap()[0];
            let reference_group = Group::new(reference.tracks(), index).unwrap();
            let mut baseline = NativeAacDecoder::new(&asc).unwrap();
            let mut broken =
                Mp4Reader::open_indexed(Cursor::new(broken), Default::default()).unwrap();
            let index = Group::automatic_audio_tracks(&broken).unwrap()[0];
            let broken_group = Group::new(broken.tracks(), index).unwrap();
            for i in 0..6 {
                let units = group.read_frame(&mut reader, i).unwrap();
                let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
                let original = reference_group.read_frame(&mut reference, i).unwrap();
                let reference_input: Vec<_> = original.iter().map(|u| u.borrowed()).collect();
                if i == 1 {
                    let saved = decoder.checkpoint();
                    let invalid = broken_group.read_frame(&mut broken, 1).unwrap();
                    let invalid: Vec<_> = invalid.iter().map(|u| u.borrowed()).collect();
                    let error = decoder
                        .decode_esc(&invalid)
                        .err()
                        .expect("malformed grouped payload must refuse")
                        .to_string();
                    assert!(
                        error.contains(bad["error"].as_str().unwrap()),
                        "{} {}: {error}",
                        c["name"],
                        bad["video"]
                    );
                    let actual = decoder.decode_esc(&input).unwrap();
                    let expected = baseline.decode_esc(&reference_input).unwrap();
                    assert_eq!(actual, expected, "{} failure rollback", c["name"]);
                    decoder.restore(&saved).unwrap();
                    assert_eq!(
                        decoder.decode_esc(&input).unwrap(),
                        expected,
                        "checkpoint replay after error"
                    );
                } else {
                    assert_eq!(
                        decoder.decode_esc(&input).unwrap(),
                        baseline.decode_esc(&reference_input).unwrap()
                    );
                }
            }
        }
    }
}

#[test]
fn owned_and_public_grouped_pair_exports_preserve_exact_control_pcm_and_ranges() {
    let m = manifest();
    for c in m["cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap();
        let mut expected = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(control),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        let rate = c["rate"].as_u64().unwrap();
        let stride = c["channels"].as_u64().unwrap() as usize * 4;
        let mut public = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut public).unwrap();
        assert!(public == expected, "{} public PCM", c["name"]);
        for range in [None, Some((7u64, 31u64)), Some((35, 67)), Some((7, 31))] {
            let mut actual = Vec::new();
            let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&data),
                &mut actual,
                range.map(|(a, b)| {
                    (
                        std::time::Duration::from_millis(a),
                        std::time::Duration::from_millis(b),
                    )
                }),
                &fvid_control::CopyOptions {
                    max_controlled_bytes: Some(128 << 20),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                (stats.sample_rate, stats.channels),
                (rate as u32, c["channels"].as_u64().unwrap() as u16)
            );
            let wanted = if let Some((a, b)) = range {
                &expected[(a * rate / 1000) as usize * stride..(b * rate / 1000) as usize * stride]
            } else {
                &expected
            };
            assert!(actual == wanted, "{} crop {range:?}", c["name"]);
        }
    }
}
#[cfg(feature = "player")]
fn play(
    source: &mut dyn fvid::audio::AudioStream,
    decoder: &mut Box<dyn fvid::audio::AudioDecode>,
) -> Vec<u8> {
    use fvid::audio::AudioStep;
    fn present(
        source: &mut dyn fvid::audio::AudioStream,
        f: fvid::audio::DecodedAudio,
        out: &mut Vec<u8>,
    ) {
        if let Some(p) = source.present_decoded(f.packet, f.source_pts).unwrap() {
            out.extend(p.data);
        }
    }
    let mut out = Vec::new();
    while let Some(step) = source.next_step().unwrap() {
        match step {
            AudioStep::ResetDecoder => source.reset_decoder(decoder).unwrap(),
            AudioStep::ReconfigureDecoder => {
                panic!("constant grouped transport unexpectedly changed config")
            }
            AudioStep::Pcm(p) => out.extend(p.data),
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
#[test]
fn grouped_pair_playback_frame_edge_seek_eof_and_rewind_match_pcm() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut source =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(source.audio_tracks().len(), 1);
        assert_eq!(source.sample_rate(), c["rate"].as_u64().unwrap() as u32);
        assert_eq!(source.channels(), c["channels"].as_u64().unwrap() as u16);
        let mut decoder = source.make_decoder().unwrap();
        let full = play(&mut source, &mut decoder);
        let mut expected = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        assert!(full == expected);
        let stride = usize::from(source.channels()) * 4;
        let frame = c["n"].as_u64().unwrap() as usize * source.sample_rate() as usize / 24000;
        let end = full.len() / stride;
        for sample in [
            1,
            frame - 1,
            frame,
            frame + 1,
            3 * frame - 1,
            3 * frame,
            3 * frame + 1,
            end,
            end + 1,
            7,
            7,
            0,
        ] {
            let ticks = (sample as u128 * u128::from(source.timescale())
                / u128::from(source.sample_rate())) as i64;
            let landed = source.seek_to(ticks);
            source.reset_decoder(&mut decoder).unwrap();
            let actual = play(&mut source, &mut decoder);
            let begin = (u128::try_from(landed).unwrap() * u128::from(source.sample_rate()))
                .div_ceil(u128::from(source.timescale())) as usize
                * stride;
            assert!(
                actual == full[begin.min(full.len())..],
                "{} seek {sample}",
                c["name"]
            );
        }
        source.rewind();
        source.reset_decoder(&mut decoder).unwrap();
        assert!(play(&mut source, &mut decoder) == full);
    }
}

#[test]
fn grouped_pair_wave_gain_crop_and_resampling_match_separate_transport() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-grouped-sbr-wave-{}-{}",
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
    for (i, c) in manifest()["cases"].as_array().unwrap().iter().enumerate() {
        let mut output = Vec::new();
        for (j, key) in ["video", "control"].iter().enumerate() {
            let path = dir.join(format!("{i}-{j}.wav"));
            fvid_media::decode_audio_transformed(
                &root().join(c[key].as_str().unwrap()),
                &path,
                fvid_media::AudioDecodeTransform {
                    sample_rate: Some(32000),
                    channels: Some(2),
                    volume: Some(0.5),
                    interval: Some((7000, 31000)),
                    ..Default::default()
                },
                &fvid_control::CopyOptions {
                    max_controlled_bytes: Some(128 << 20),
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
            let bytes = std::fs::read(path).unwrap();
            let info =
                fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&bytes), None).unwrap();
            assert_eq!((info.sample_rate, info.channels), (32000, 2));
            output.push(
                bytes[info.data_offset as usize
                    ..info.data_offset as usize + info.data_bytes as usize]
                    .to_vec(),
            );
        }
        assert!(!output[0].is_empty());
        assert!(output[0] == output[1], "{} transformed PCM", c["name"]);
    }
}

#[test]
fn ep1_pair_reassembly_is_bit_exact_and_packetization_can_change_without_dsp_reset() {
    use fvid_media::owned_aac::{
        NativeAacDecoder, aac_eld_sbr, aac_ep1, aac_esc, config::AudioSpecificConfig,
    };
    use fvid_media::{
        owned_codec_config::aac_stream_descriptor, owned_mp4::Mp4Reader, owned_mp4_ep1::Group,
    };
    for c in manifest()["cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let original = std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(data), Default::default()).unwrap();
        let mut reference =
            Mp4Reader::open_indexed(Cursor::new(original), Default::default()).unwrap();
        let selected = Group::automatic_audio_tracks(&reader).unwrap()[0];
        let other = Group::automatic_audio_tracks(&reference).unwrap()[0];
        let group = Group::new(reader.tracks(), selected).unwrap();
        let control = Group::new(reference.tracks(), other).unwrap();
        assert!(
            Group::admission_upper_bound(reader.tracks(), selected).unwrap()
                >= group.retained_payload_bytes().unwrap()
        );
        let asc = aac_stream_descriptor(&reader.tracks()[selected].configuration)
            .unwrap()
            .asc
            .to_vec();
        let parsed = AudioSpecificConfig::parse(&asc).unwrap();
        let context = aac_eld_sbr::Decoder::new(
            parsed.eld_sbr.clone().unwrap(),
            parsed.core.sample_rate,
            parsed.core.frame_samples,
            parsed.core.channel_configuration,
        )
        .unwrap();
        let mut mixed = NativeAacDecoder::new(&asc).unwrap();
        let mut baseline = NativeAacDecoder::new(&asc).unwrap();
        for i in 0..6 {
            let units = group.read_frame(&mut reader, i).unwrap();
            let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
            let original = control.read_frame(&mut reference, i).unwrap();
            let reference_input: Vec<_> = original.iter().map(|u| u.borrowed()).collect();
            if i == 0 {
                let joined = aac_ep1::associate(&input).unwrap();
                let split = aac_ep1::associate(&reference_input).unwrap();
                let groups = c["groups"].as_u64().unwrap() as usize;
                let core = c["original_streams"].as_u64().unwrap() as usize - 2 * groups;
                let mut partitions: Vec<Vec<usize>> = (0..core).map(|i| vec![i]).collect();
                match c["mode"].as_str().unwrap() {
                    "batch" => partitions.push((core..core + 2 * groups).collect()),
                    "bridge" => {
                        partitions.push(vec![core]);
                        partitions.extend(
                            (core + 1..core + 2 * groups - 1)
                                .step_by(2)
                                .map(|i| vec![i, i + 1]),
                        );
                        partitions.push(vec![core + 2 * groups - 1]);
                    }
                    mode => {
                        for g in 0..groups {
                            let index = core + 2 * g;
                            if mode == "all" || g + 1 == groups {
                                partitions.push(vec![index, index + 1]);
                            } else {
                                partitions.push(vec![index]);
                                partitions.push(vec![index + 1]);
                            }
                        }
                    }
                }
                if let Some(declared) = c.get("partitions") {
                    partitions = declared
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| {
                            p.as_array()
                                .unwrap()
                                .iter()
                                .map(|i| i.as_u64().unwrap() as usize)
                                .collect()
                        })
                        .collect();
                }
                assert_eq!(joined.payload.classes.len(), partitions.len());
                for (actual, indices) in joined.payload.classes.iter().zip(partitions) {
                    let expected: Vec<_> = indices
                        .into_iter()
                        .flat_map(|i| split.payload.classes[i].bits.iter().copied())
                        .collect();
                    assert!(
                        actual.bits == expected,
                        "{} authored AU membership",
                        c["name"]
                    );
                }
                let expected =
                    aac_esc::reassemble_eld(&parsed.core, &split.payload, Some(&context)).unwrap();
                let actual =
                    aac_esc::reassemble_eld_ep1(&parsed.core, &joined.payload, Some(&context))
                        .unwrap();
                assert!(actual == expected, "reassembled logical AU changed");
                assert!(
                    aac_esc::reassemble_eld(&parsed.core, &joined.payload, Some(&context)).is_err(),
                    "strict recovered-class entrypoint must retain declared boundaries"
                );
            }
            let actual = mixed
                .decode_esc(if i % 2 == 0 { &reference_input } else { &input })
                .unwrap();
            let expected = baseline.decode_esc(&reference_input).unwrap();
            assert_eq!(
                actual, expected,
                "{} mixed packetization frame {i}",
                c["name"]
            );
            if i == 2 {
                let future = control.read_frame(&mut reference, 3).unwrap();
                let future: Vec<_> = future.iter().map(|u| u.borrowed()).collect();
                let saved = baseline.checkpoint();
                baseline.restore(&mixed.checkpoint()).unwrap();
                let replay = baseline.decode_esc(&future).unwrap();
                baseline.restore(&saved).unwrap();
                assert_eq!(
                    replay,
                    baseline.decode_esc(&future).unwrap(),
                    "transport-independent checkpoint"
                );
                baseline.restore(&saved).unwrap();
            }
        }
    }
}

#[test]
fn consecutive_core_and_sbr_partitions_preserve_complete_native_history() {
    use fvid_media::{
        owned_aac::{NativeAacDecoder, aac_ep1},
        owned_codec_config::aac_stream_descriptor,
        owned_mp4::Mp4Reader,
        owned_mp4_ep1::Group,
    };
    let mut layouts = 0;
    // Batch cases identify the multichannel SBR controls; selected whole cases
    // add combined core tools and empty categories. All boundary masks include
    // separate, paired and whole-region AUs, without parser-derived partitions.
    for c in manifest()["cases"].as_array().unwrap().iter().filter(|c| {
        c["mode"] == "batch"
            || c["mode"] == "whole" && {
                let name = c["name"].as_str().unwrap();
                name.contains("tns-resilience")
                    || name.contains("empty-core")
                    || matches!(
                        name,
                        "n480-r24000-c2-resilience-f7-x2-crc1-whole"
                            | "n512-r24000-c2-resilience-f7-x2-crc1-whole"
                    )
            }
    }) {
        let bytes = std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(bytes), Default::default()).unwrap();
        let selected = Group::automatic_audio_tracks(&reader).unwrap()[0];
        let group = Group::new(reader.tracks(), selected).unwrap();
        let asc = aac_stream_descriptor(&reader.tracks()[selected].configuration)
            .unwrap()
            .asc
            .to_vec();
        let mut baseline = NativeAacDecoder::new(&asc).unwrap();
        let mut frames = Vec::new();
        let mut expected = Vec::new();
        for i in 0..6 {
            let units = group.read_frame(&mut reader, i).unwrap();
            let borrowed: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
            expected.push(baseline.decode_esc(&borrowed).unwrap());
            frames.push(aac_ep1::associate(&borrowed).unwrap());
        }
        let core_count = if c["mode"] == "whole" {
            0
        } else {
            frames[0].payload.classes.len() - c["groups"].as_u64().unwrap() as usize * 2
        };
        let sbr_count = frames[0].payload.classes.len() - core_count;
        for mask in 0..1usize << (sbr_count - 1) {
            layouts += 1;
            let mut decoder = NativeAacDecoder::new(&asc).unwrap();
            for (i, frame) in frames.iter().enumerate() {
                let mut parts: Vec<Vec<u8>> = frame.payload.classes[..core_count]
                    .iter()
                    .map(|c| c.bits.clone())
                    .collect();
                parts.push(Vec::new());
                for (index, class) in frame.payload.classes[core_count..].iter().enumerate() {
                    parts.last_mut().unwrap().extend_from_slice(&class.bits);
                    if index + 1 < sbr_count && mask & (1 << index) != 0 {
                        parts.push(Vec::new());
                    }
                }
                let packed: Vec<Vec<u8>> = parts
                    .iter()
                    .map(|bits| {
                        let mut bytes = vec![0xff; bits.len().div_ceil(8)];
                        for (index, &bit) in bits.iter().enumerate() {
                            let shift = 7 - index % 8;
                            bytes[index / 8] = (bytes[index / 8] & !(1 << shift)) | (bit << shift);
                        }
                        bytes
                    })
                    .collect();
                let input: Vec<_> = packed
                    .iter()
                    .zip(&parts)
                    .enumerate()
                    .map(|(j, (bytes, bits))| aac_ep1::AccessUnit {
                        stream_id: j as u16 + 1,
                        depends_on: (j > 0).then_some(j as u16),
                        data: bytes,
                        bit_len: bits.len(),
                        pts: frame.pts,
                        duration: frame.duration,
                        timescale: frame.timescale,
                    })
                    .collect();
                assert_eq!(
                    decoder.decode_esc(&input).unwrap(),
                    expected[i],
                    "{} partition {mask:#x} frame {i}",
                    c["name"]
                );
            }
        }
    }
    assert_eq!(layouts, 2512);
}
