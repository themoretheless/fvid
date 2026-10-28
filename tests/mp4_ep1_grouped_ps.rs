use fvid_media::owned_aac::aac_ps_native::NativePsAacDecoder;
use serde_json::Value;
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-ep1-grouped-ps")
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
    assert_eq!(m["cases"].as_array().unwrap().len(), 432);
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
        assert!(
            source
                .decode_source_packet(decoder.as_mut(), &p, stamp)
                .unwrap()
                .is_none()
        );
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
        bad[0].timescale = 0;
        let error = match d.decode_esc_packet(&bad, 123, 99) {
            Ok(_) => panic!("mismatched ESC clock accepted"),
            Err(e) => e,
        };
        assert!(
            error.to_string().contains("zero EP1 source timescale"),
            "{error}"
        );
        assert!(
            d.finish_packet()
                .err()
                .unwrap()
                .to_string()
                .contains("requires reset")
        );
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
    let dir =
        std::env::temp_dir().join(format!("fvid-grouped-ep1-ps-files-{}", std::process::id()));
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
fn grouped_ps_membership_and_every_boundary_match_raw_decode_and_discovery() {
    use fvid_media::{
        owned_aac::{aac_ep1, aac_ps_native::InBandPsProbe},
        owned_mp4::Mp4Reader,
        owned_mp4_ep1::Group,
    };
    let m = manifest();
    let mut layouts = 0;
    for c in m["cases"].as_array().unwrap() {
        let mut original = Mp4Reader::open_indexed(
            Cursor::new(std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap()),
            Default::default(),
        )
        .unwrap();
        let selected = Group::automatic_audio_tracks(&original).unwrap()[0];
        let original_group = Group::new(original.tracks(), selected).unwrap();
        let mut reader = Mp4Reader::open_indexed(
            Cursor::new(std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap()),
            Default::default(),
        )
        .unwrap();
        let selected = Group::automatic_audio_tracks(&reader).unwrap()[0];
        let group = Group::new(reader.tracks(), selected).unwrap();
        for i in 0..6 {
            let source = original_group.read_frame(&mut original, i).unwrap();
            let input: Vec<_> = source.iter().map(|u| u.borrowed()).collect();
            let source = aac_ep1::associate(&input).unwrap();
            let units = group.read_frame(&mut reader, i).unwrap();
            let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
            let actual = aac_ep1::associate(&input).unwrap();
            let parts = c["partitions"].as_array().unwrap();
            assert_eq!(actual.payload.classes.len(), parts.len());
            for (actual, indices) in actual.payload.classes.iter().zip(parts) {
                let expected: Vec<_> = indices
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|j| {
                        source.payload.classes[j.as_u64().unwrap() as usize]
                            .bits
                            .iter()
                            .copied()
                    })
                    .collect();
                assert!(
                    actual.bits == expected,
                    "{} frame {i} authored membership",
                    c["name"]
                );
            }
        }
        if c["mode"] != "indexed" || c["grouping"] != "whole" {
            continue;
        }
        let raw = std::fs::read(root().join(c["logical"].as_str().unwrap())).unwrap();
        for mask in 0..8usize {
            layouts += 1;
            let mut decoder = decoder(&hex(c["asc"].as_str().unwrap()), c);
            let mut reference = decoder_for_raw(c);
            let mut probe = InBandPsProbe::new(
                &hex(c["asc"].as_str().unwrap()),
                c["rate"].as_u64().unwrap() as u32,
            )
            .unwrap();
            let mut raw_probe = InBandPsProbe::new(
                &hex(c["plain_asc"].as_str().unwrap()),
                c["rate"].as_u64().unwrap() as u32,
            )
            .unwrap();
            for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
                let source = original_group.read_frame(&mut original, i).unwrap();
                let input: Vec<_> = source.iter().map(|u| u.borrowed()).collect();
                let frame = aac_ep1::associate(&input).unwrap();
                assert_eq!(frame.payload.classes.len(), 4);
                let mut parts = vec![Vec::new()];
                for (j, class) in frame.payload.classes.iter().enumerate() {
                    parts.last_mut().unwrap().extend_from_slice(&class.bits);
                    if j < 3 && mask & (1 << j) != 0 {
                        parts.push(Vec::new());
                    }
                }
                let bytes: Vec<Vec<u8>> = parts
                    .iter()
                    .map(|bits| {
                        let mut out = vec![0xff; bits.len().div_ceil(8)];
                        for (j, &bit) in bits.iter().enumerate() {
                            let shift = 7 - j % 8;
                            out[j / 8] = (out[j / 8] & !(1 << shift)) | (bit << shift);
                        }
                        out
                    })
                    .collect();
                let input: Vec<_> = bytes
                    .iter()
                    .zip(&parts)
                    .enumerate()
                    .map(|(j, (data, bits))| aac_ep1::AccessUnit {
                        stream_id: j as u16 + 1,
                        depends_on: (j > 0).then_some(j as u16),
                        data,
                        bit_len: bits.len(),
                        pts: frame.pts,
                        duration: frame.duration,
                        timescale: frame.timescale,
                    })
                    .collect();
                let a = row["offset"].as_u64().unwrap() as usize;
                let b = a + row["bytes"].as_u64().unwrap() as usize;
                assert_eq!(
                    decoder.decode_esc(&input).unwrap(),
                    reference.decode(&raw[a..b]).unwrap()
                );
                assert_eq!(
                    probe.read_esc(&input).unwrap(),
                    raw_probe.read(&raw[a..b]).unwrap()
                );
            }
            assert!(probe.ps_detected());
            assert_eq!(decoder.finish().unwrap(), reference.finish().unwrap());
            assert!(decoder.finish().unwrap().is_none());
        }
    }
    assert_eq!(layouts, 384);
}
fn decoder_for_raw(c: &Value) -> NativePsAacDecoder {
    decoder(&hex(c["plain_asc"].as_str().unwrap()), c)
}

#[test]
fn absent_grouped_pce_ps_tag_preserves_delayed_decode_and_discovery_history() {
    use fvid_media::{
        owned_aac::aac_ps_native::InBandPsProbe, owned_mp4::Mp4Reader, owned_mp4_ep1::Group,
    };
    let m = manifest();
    let mut cases = 0;
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c.get("malformed_tag").is_some())
    {
        cases += 1;
        let open = |key: &str| {
            let reader = Mp4Reader::open_indexed(
                Cursor::new(std::fs::read(root().join(c[key].as_str().unwrap())).unwrap()),
                Default::default(),
            )
            .unwrap();
            let at = Group::automatic_audio_tracks(&reader).unwrap()[0];
            let group = Group::new(reader.tracks(), at).unwrap();
            (reader, group)
        };
        let (mut good, group) = open("video");
        let (mut bad, bad_group) = open("malformed_tag");
        let asc = hex(c["asc"].as_str().unwrap());
        let mut d = decoder(&asc, c);
        let mut baseline = decoder_for_raw(c);
        let mut probe = InBandPsProbe::new(&asc, c["rate"].as_u64().unwrap() as u32).unwrap();
        let data = std::fs::read(root().join(c["logical"].as_str().unwrap())).unwrap();
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let units = group.read_frame(&mut good, i).unwrap();
            let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
            let checkpoint = d.checkpoint();
            let before = probe.clone();
            if i == 1 {
                let units = bad_group.read_frame(&mut bad, i).unwrap();
                let bad: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
                assert!(
                    d.decode_esc(&bad)
                        .unwrap_err()
                        .to_string()
                        .contains("PS AAC mono tag is not configured by PCE")
                );
                assert!(
                    probe
                        .read_esc(&bad)
                        .unwrap_err()
                        .to_string()
                        .contains("PS AAC mono tag is not configured by PCE")
                );
                assert_eq!(probe.ps_detected(), before.ps_detected());
            }
            let a = row["offset"].as_u64().unwrap() as usize;
            let b = a + row["bytes"].as_u64().unwrap() as usize;
            let expected = baseline.decode(&data[a..b]).unwrap();
            let actual = d.decode_esc(&input).unwrap();
            assert_eq!(actual, expected);
            let detected = probe.read_esc(&input).unwrap();
            if i == 1 {
                d.restore(&checkpoint).unwrap();
                assert_eq!(d.decode_esc(&input).unwrap(), expected);
                let mut before = before;
                assert_eq!(before.read_esc(&input).unwrap(), detected);
            }
        }
        assert_eq!(d.finish().unwrap(), baseline.finish().unwrap());
        assert!(d.finish().unwrap().is_none());
    }
    assert_eq!(cases, 24);
}
