use fvid_media::owned_aac::aac_ps_native::NativePsAacDecoder;
use serde_json::Value;
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-ep1-ps")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn reference(case: &Value) -> Vec<u8> {
    let asc = hex(case["plain_asc"].as_str().unwrap());
    let mut d = if case["signal"] == "implicit" {
        NativePsAacDecoder::new_with_in_band_ps(&asc, case["rate"].as_u64().unwrap() as u32)
            .unwrap()
    } else {
        NativePsAacDecoder::new(&asc).unwrap()
    };
    let data = std::fs::read(root().join(case["logical"].as_str().unwrap())).unwrap();
    let mut pcm = Vec::new();
    for row in case["frames"].as_array().unwrap() {
        let a = row["offset"].as_u64().unwrap() as usize;
        let b = a + row["bytes"].as_u64().unwrap() as usize;
        if let Some(frame) = d.decode(&data[a..b]).unwrap() {
            pcm.extend(frame.pcm);
        }
    }
    while let Some(frame) = d.finish().unwrap() {
        pcm.extend(frame.pcm);
    }
    assert!(d.finish().unwrap().is_none());
    pcm.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn presentation(case: &Value, raw: &[u8]) -> Vec<u8> {
    if case["mode"] != "edits" {
        return raw.to_vec();
    }
    let rate = case["rate"].as_u64().unwrap() as usize;
    let frames = rate / 30;
    let mut pcm = vec![0; frames * 8];
    pcm.extend_from_slice(&raw[(7 * rate / 1000) * 8..(7 * rate / 1000 + frames) * 8]);
    pcm.extend_from_slice(&raw[(rate / 1000) * 8..(rate / 1000 + frames) * 8]);
    pcm
}
#[test]
fn ep1_ps_movies_export_all_delayed_frames_and_exact_edited_windows() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 72);
    for case in m["cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let raw = reference(case);
        let expected = presentation(case, &raw);
        assert_eq!(
            raw.len(),
            6 * case["n"].as_u64().unwrap() as usize * case["rate"].as_u64().unwrap() as usize
                / 24000
                * 8
        );
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
        assert_eq!(owned, expected, "{} complete PS export", case["name"]);
        let mut public = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut public).unwrap();
        assert_eq!(owned, public);
        let rate = case["rate"].as_u64().unwrap();
        for (from, to) in [(7u64, 31u64), (35, 67), (7, 31)] {
            let window = Some((
                std::time::Duration::from_millis(from),
                std::time::Duration::from_millis(to),
            ));
            let mut cropped = Vec::new();
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&data),
                &mut cropped,
                window,
                &Default::default(),
            )
            .unwrap();
            let a = ((from * rate).div_ceil(1000) as usize * 8).min(expected.len());
            let b = ((to * rate).div_ceil(1000) as usize * 8).min(expected.len());
            assert_eq!(cropped, expected[a..b], "{} PS interval", case["name"]);
        }
    }
}
fn decoder(asc: &[u8], case: &Value) -> NativePsAacDecoder {
    if case["signal"] == "implicit" {
        NativePsAacDecoder::new_with_in_band_ps(asc, case["rate"].as_u64().unwrap() as u32).unwrap()
    } else {
        NativePsAacDecoder::new(asc).unwrap()
    }
}
#[test]
fn real_ps_groups_preserve_delayed_identity_checkpoints_and_last_eof_frame() {
    for case in manifest()["cases"].as_array().unwrap() {
        let movie = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid_media::owned_mp4::Mp4Reader::open_indexed(Cursor::new(movie), Default::default())
                .unwrap();
        let index = fvid_media::owned_mp4_ep1::Group::automatic_audio_tracks(&reader).unwrap();
        assert_eq!(index.len(), 1);
        let group = fvid_media::owned_mp4_ep1::Group::new(reader.tracks(), index[0]).unwrap();
        let mut d = decoder(&hex(case["asc"].as_str().unwrap()), case);
        let mut raw = decoder(&hex(case["plain_asc"].as_str().unwrap()), case);
        let data = std::fs::read(root().join(case["logical"].as_str().unwrap())).unwrap();
        let mut emitted = 0u64;
        for (i, row) in case["frames"].as_array().unwrap().iter().enumerate() {
            let units = group.read_frame(&mut reader, i).unwrap();
            let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
            let state = d.checkpoint();
            let actual = d.decode_esc(&input).unwrap();
            d.restore(&state).unwrap();
            assert_eq!(actual, d.decode_esc(&input).unwrap());
            let a = row["offset"].as_u64().unwrap() as usize;
            let b = a + row["bytes"].as_u64().unwrap() as usize;
            let expected = raw.decode(&data[a..b]).unwrap();
            assert_eq!(actual, expected, "{} PS identity {i}", case["name"]);
            if i == 0 {
                assert!(actual.is_none());
            }
            if let Some(frame) = actual {
                assert_eq!(frame.frame_index, emitted);
                emitted += 1;
            }
        }
        let state = d.checkpoint();
        let last = d.finish().unwrap().unwrap();
        d.restore(&state).unwrap();
        assert_eq!(Some(last.clone()), d.finish().unwrap());
        assert_eq!(Some(last.clone()), raw.finish().unwrap());
        assert_eq!(last.frame_index, emitted);
        emitted += 1;
        assert_eq!(emitted, 6);
        assert!(d.finish().unwrap().is_none());
        assert!(raw.finish().unwrap().is_none());
    }
}
#[test]
fn ps_transport_pcm_agrees_with_independent_core_and_separate_ps_stage() {
    use fvid_media::owned_aac::{aac_sbr_dsp::OutputRate, aac_sbr_ps, bits::BitReader};
    for case in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["mode"] == "indexed")
    {
        let core: Vec<f32> = std::fs::read(root().join(case["core_reference"].as_str().unwrap()))
            .unwrap()
            .chunks_exact(4)
            .map(|p| f32::from_le_bytes(p.try_into().unwrap()))
            .collect();
        let n = case["n"].as_u64().unwrap() as usize;
        let mut stage = aac_sbr_ps::Decoder::default();
        let mut expected = Vec::new();
        let append = |out: &mut Vec<f32>, frame: aac_sbr_ps::Frame| {
            out.extend(
                frame.pcm[0]
                    .iter()
                    .zip(&frame.pcm[1])
                    .flat_map(|(l, r)| [*l as f32, *r as f32]),
            );
        };
        for (i, payload) in case["payloads"].as_array().unwrap().iter().enumerate() {
            let data = hex(payload.as_str().unwrap());
            let mut bits = BitReader::new(&data);
            assert_eq!(bits.read(4).unwrap(), 13);
            if let Some(frame) = stage
                .read(
                    &mut bits,
                    data.len() * 8,
                    false,
                    &core[i * n..(i + 1) * n],
                    48000,
                    (n / 64) as u8,
                    if case["rate"] == 24000 {
                        OutputRate::Core
                    } else {
                        OutputRate::Double
                    },
                )
                .unwrap()
            {
                append(&mut expected, frame);
            }
        }
        while let Some(frame) = stage.finish().unwrap() {
            append(&mut expected, frame);
        }
        let actual = reference(case);
        assert_eq!(actual.len(), expected.len() * 4);
        let worst = actual
            .chunks_exact(4)
            .zip(&expected)
            .map(|(a, b)| (f32::from_le_bytes(a.try_into().unwrap()) - b).abs())
            .fold(0f32, f32::max);
        assert!(
            worst < 2e-6,
            "{} independent-core/shared-PS error {worst}",
            case["name"]
        );
    }
}
#[cfg(feature = "player")]
fn play(
    source: &mut dyn fvid::audio::AudioStream,
    decoder: &mut Box<dyn fvid::audio::AudioDecode>,
) -> Vec<u8> {
    use fvid::audio::AudioStep;
    let mut pcm = Vec::new();
    while let Some(step) = source.next_step().unwrap() {
        match step {
            AudioStep::ResetDecoder => source.reset_decoder(decoder).unwrap(),
            AudioStep::ReconfigureDecoder => panic!("unexpected PS setup change"),
            AudioStep::Pcm(p) => pcm.extend(p.data),
            AudioStep::Encoded(p) => {
                let stamp = source.decoder_stamp(&p);
                if let Some(f) = source
                    .decode_source_packet(decoder.as_mut(), &p, stamp)
                    .unwrap()
                {
                    if let Some(p) = source.present_decoded(f.packet, f.source_pts).unwrap() {
                        pcm.extend(p.data);
                    }
                }
            }
        }
    }
    if source.drain_decoded_at_eof() {
        while let Some(f) = decoder.finish_packet().unwrap() {
            if let Some(p) = source.present_decoded(f.packet, f.source_pts).unwrap() {
                pcm.extend(p.data);
            }
        }
    }
    source.validate_eof().unwrap();
    pcm
}
#[cfg(feature = "player")]
#[test]
fn ps_audiostream_playback_seek_rewind_and_delayed_checkpoint_restore_match_pcm() {
    use fvid::audio::AudioStream;
    for case in manifest()["cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let raw = reference(case);
        let expected = presentation(case, &raw);
        let mut source =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(data), Default::default())
                .unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
        assert_eq!(source.audio_tracks().len(), 1);
        assert_eq!(source.channels(), 2);
        let mut decoder = source.make_decoder().unwrap();
        assert_eq!(
            play(&mut source, &mut decoder),
            expected,
            "{} playback",
            case["name"]
        );
        let total = expected.len() / 8;
        let frame =
            case["n"].as_u64().unwrap() as usize * case["rate"].as_u64().unwrap() as usize / 24000;
        for target in [
            0,
            1,
            frame - 1,
            frame,
            frame + 1,
            total,
            total + frame,
            7,
            7,
            0,
        ] {
            let ticks = (target as u128 * u128::from(source.timescale())
                / u128::from(source.sample_rate())) as i64;
            let landed = source.seek_to(ticks);
            source.reset_decoder(&mut decoder).unwrap();
            let actual = play(&mut source, &mut decoder);
            let begin = (u128::try_from(landed).unwrap() * u128::from(source.sample_rate()))
                .div_ceil(u128::from(source.timescale())) as usize
                * 8;
            assert_eq!(
                actual,
                expected[begin.min(expected.len())..],
                "{} seek {target}",
                case["name"]
            );
        }
        source.rewind();
        source.reset_decoder(&mut decoder).unwrap();
        let p = source.next_packet().unwrap().unwrap();
        let stamp = source.decoder_stamp(&p);
        assert!(source
            .decode_source_packet(decoder.as_mut(), &p, stamp)
            .unwrap()
            .is_none());
        let saved = decoder.checkpoint().unwrap();
        let p = source.next_packet().unwrap().unwrap();
        let stamp = source.decoder_stamp(&p);
        let a = source
            .decode_source_packet(decoder.as_mut(), &p, stamp)
            .unwrap()
            .unwrap();
        decoder.restore(&saved).unwrap();
        let b = source
            .decode_source_packet(decoder.as_mut(), &p, stamp)
            .unwrap()
            .unwrap();
        assert_eq!(a.packet.data, b.packet.data);
        assert_eq!(a.source_pts, b.source_pts);
        source.rewind();
        source.reset_decoder(&mut decoder).unwrap();
        assert_eq!(play(&mut source, &mut decoder), expected);
    }
}
#[cfg(feature = "player")]
#[test]
fn ps_typed_bridge_restores_pending_source_identity_after_bad_esc_clock() {
    use fvid::audio::AudioStream;
    for case in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["mode"] == "indexed")
    {
        let movie = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid_media::owned_mp4::Mp4Reader::open_indexed(Cursor::new(&movie), Default::default())
                .unwrap();
        let track = fvid_media::owned_mp4_ep1::Group::automatic_audio_tracks(&reader).unwrap()[0];
        let group = fvid_media::owned_mp4_ep1::Group::new(reader.tracks(), track).unwrap();
        let source =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&movie), Default::default())
                .unwrap();
        let mut d = source.make_decoder().unwrap();
        let first = group.read_frame(&mut reader, 0).unwrap();
        let input: Vec<_> = first.iter().map(|u| u.borrowed()).collect();
        assert!(d.decode_esc_packet(&input, -13, 77).unwrap().is_none());
        let saved = d.checkpoint().unwrap();
        let second = group.read_frame(&mut reader, 1).unwrap();
        let input: Vec<_> = second.iter().map(|u| u.borrowed()).collect();
        let mut bad = input.clone();
        bad[1].pts += 1;
        let error = match d.decode_esc_packet(&bad, 123, 99) {
            Ok(_) => panic!("mismatched ESC clock accepted"),
            Err(e) => e,
        };
        assert!(error.to_string().contains("same source time"), "{error}");
        assert!(d
            .finish_packet()
            .err()
            .unwrap()
            .to_string()
            .contains("requires reset"));
        d.restore(&saved).unwrap();
        let frame = d.decode_esc_packet(&input, 123, 99).unwrap().unwrap();
        assert_eq!(frame.source_pts, -13);
        assert_eq!(frame.source_duration, 77);
        let raw = reference(case);
        assert_eq!(frame.packet.data, &raw[..frame.packet.data.len()]);
    }
}
#[test]
fn ps_ep1_file_export_preserves_complete_pcm_and_interval_gain() {
    let dir = std::env::temp_dir().join(format!("fvid-ep1-ps-files-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    for case in manifest()["cases"].as_array().unwrap() {
        let raw = reference(case);
        let expected = presentation(case, &raw);
        let rate = case["rate"].as_u64().unwrap();
        let source = root().join(case["video"].as_str().unwrap());
        for crop in [false, true] {
            let output = dir.join(format!("{}-{crop}.wav", case["name"].as_str().unwrap()));
            let transform = if crop {
                fvid_media_info::AudioDecodeTransform {
                    interval: Some((7000, 31000)),
                    volume: Some(0.5),
                    ..Default::default()
                }
            } else {
                Default::default()
            };
            fvid_media::decode_audio_transformed(
                &source,
                &output,
                transform,
                &fvid_control::CopyOptions {
                    max_controlled_bytes: Some(128 << 20),
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
            let bytes = std::fs::read(output).unwrap();
            let info =
                fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&bytes), None).unwrap();
            let actual = &bytes
                [info.data_offset as usize..info.data_offset as usize + info.data_bytes as usize];
            let expected = if crop {
                &expected[(7 * rate).div_ceil(1000) as usize * 8
                    ..(31 * rate).div_ceil(1000) as usize * 8]
            } else {
                &expected
            };
            assert_eq!(actual.len(), expected.len());
            let worst = actual
                .chunks_exact(4)
                .zip(expected.chunks_exact(4))
                .map(|(a, b)| {
                    (f32::from_le_bytes(a.try_into().unwrap())
                        - f32::from_le_bytes(b.try_into().unwrap()) * if crop { 0.5 } else { 1.0 })
                    .abs()
                })
                .fold(0f32, f32::max);
            assert!(worst < 1e-7, "{} WAVE crop {crop}: {worst}", case["name"]);
        }
    }
}

#[test]
fn standard_file_bindings_use_track_ids_and_reject_malformed_dependency_tables() {
    use fvid_media::{
        owned_codec_config::aac_stream_descriptor, owned_mp4::Mp4Reader, owned_mp4_ep1::Group,
    };
    let m = manifest();
    for case in m["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(bytes), Default::default()).unwrap();
        let roots = Group::automatic_audio_tracks(&reader).unwrap();
        assert_eq!(roots.len(), 1);
        let at = roots[0];
        for track in reader.tracks().iter().filter(|t| t.handler == *b"soun") {
            let d = aac_stream_descriptor(&track.configuration).unwrap();
            assert_eq!(d.es_id, 0);
            assert_eq!(d.depends_on, None);
            assert_eq!(d.ocr_es_id, None);
            assert_eq!(
                track.decoder_setup(1).unwrap().dependencies,
                track.dependencies
            );
        }
        let group = Group::new(reader.tracks(), at).unwrap();
        let units = group.read_frame(&mut reader, 0).unwrap();
        assert_eq!(units.len(), 4);
        for unit in units {
            assert!(reader
                .tracks()
                .iter()
                .any(|t| t.id == u32::from(unit.stream_id)));
        }
    }
    for case in m["malformed"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let result = Mp4Reader::open_indexed(Cursor::new(bytes), Default::default());
        let error = if case["parser"] == true {
            result
                .err()
                .expect("malformed dpnd must fail parsing")
                .to_string()
        } else {
            let reader = result.unwrap();
            let at = reader
                .tracks()
                .iter()
                .position(|t| t.handler == *b"soun" && t.dependencies.is_empty())
                .unwrap();
            Group::new(reader.tracks(), at)
                .err()
                .expect("invalid EP1 graph must refuse")
                .to_string()
        };
        assert!(
            error.contains(case["error"].as_str().unwrap()),
            "{}: {error}",
            case["video"]
        );
    }
}

#[cfg(feature = "player")]
#[test]
fn standard_ld_sbr_dependencies_preserve_independent_pcm_playback_and_seek() {
    use fvid::audio::AudioStream;
    let m = manifest();
    assert_eq!(m["ld_cases"].as_array().unwrap().len(), 4);
    for case in m["ld_cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let gold = std::fs::read(root().join(case["golden"].as_str().unwrap())).unwrap();
        let mut exported = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut exported,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(exported.len(), gold.len());
        for (actual, expected) in exported.chunks_exact(4).zip(gold.chunks_exact(4)) {
            assert!(
                (f32::from_le_bytes(actual.try_into().unwrap())
                    - f32::from_le_bytes(expected.try_into().unwrap()))
                .abs()
                    < 1e-7
            );
        }
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(reader.audio_tracks().len(), 1);
        let mut decoder = reader.make_decoder().unwrap();
        let full = play(&mut reader, &mut decoder);
        assert_eq!(full, exported);
        let stride = usize::from(reader.channels()) * 4;
        let frames = full.len() / stride;
        for sample in [
            1,
            frames / 2 - 1,
            frames / 2,
            frames / 2 + 1,
            frames,
            frames + 1,
            7,
            7,
            0,
        ] {
            let ticks = (sample as u128 * u128::from(reader.timescale())
                / u128::from(reader.sample_rate())) as i64;
            let landed = reader.seek_to(ticks);
            reader.reset_decoder(&mut decoder).unwrap();
            let actual = play(&mut reader, &mut decoder);
            let begin = (u128::try_from(landed).unwrap() * u128::from(reader.sample_rate()))
                .div_ceil(u128::from(reader.timescale())) as usize
                * stride;
            assert_eq!(
                actual,
                full[begin.min(full.len())..],
                "{} seek {sample}",
                case["video"]
            );
        }
        reader.rewind();
        reader.reset_decoder(&mut decoder).unwrap();
        assert_eq!(play(&mut reader, &mut decoder), full);
    }
}
