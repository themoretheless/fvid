use fvid_media::owned_aac::{
    aac_eld_cldfb, aac_eld_sbr::Decoder, aac_ld_mps_transport::Assembler,
    aac_pce::read_eld_spatial_extensions, bits::BitReader, config::AudioSpecificConfig,
};
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-eld-spatial-sbr")
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn configuration(case: &Value) -> AudioSpecificConfig {
    AudioSpecificConfig::parse(&hex(case["asc"].as_str().unwrap())).unwrap()
}
fn number(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn input(case: &Value) -> Vec<Vec<f32>> {
    let channels = number(&case["channels"]);
    let raw: Vec<f32> = std::fs::read(root().join(case["input"].as_str().unwrap()))
        .unwrap()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    case["mapping"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            raw.chunks_exact(channels)
                .map(|row| row[number(c)])
                .collect()
        })
        .collect()
}
#[test]
fn spatial_ld_sbr_subbands_match_independent_convolution_hf_noise_and_gain_controls() {
    let r = root();
    let m: Value =
        serde_json::from_slice(&std::fs::read(r.join("manifest.json")).unwrap()).unwrap();
    let packets = std::fs::read(r.join("packets.bin")).unwrap();
    let expected = std::fs::read(r.join("expected.f64le")).unwrap();
    let mut maximum = 0.0f64;
    let mut wrong_bank_gap = 0.0f64;
    let mut high_energy = 0.0f64;
    let mut scalars = 0;
    let mut programmes = 0;
    for case in m["cases"].as_array().unwrap() {
        let parsed = configuration(case);
        let config = parsed.eld_sbr.clone().unwrap();
        let cfg = parsed.core.channel_configuration;
        let n = usize::from(parsed.core.frame_samples);
        let channels = number(&case["channels"]);
        let slots = number(&case["slots"]);
        let bands = number(&case["bands"]);
        let stored = number(&case["expected_bands"]);
        let offset = number(&case["expected_offset"]);
        let pcm = input(case);
        let fresh = Decoder::new_spatial(
            config,
            parsed.core.sample_rate,
            parsed.core.frame_samples,
            cfg,
        )
        .unwrap();
        let mut decoder = fresh.clone();
        let mut spatial_actual = fvid_media::owned_aac::aac_ld_mps_pcm::Decoder::new_qmf(
            parsed.ld_mps.as_ref().unwrap(),
            fvid_media::owned_aac::aac_ld_mps_decor::Mode::Complex,
        )
        .unwrap();
        let mut spatial_expected = spatial_actual.clone();
        let mut whole_session = fvid_media::owned_aac::aac_ld_mps_session::Session::new_qmf(
            parsed.ld_mps.as_ref().unwrap(),
            fvid_media::owned_aac::aac_ld_mps_decor::Mode::Complex,
        )
        .unwrap();
        let mut split_session = whole_session.clone();
        let mut delayed_session = whole_session.clone();
        let mut delayed_messages = Vec::new();
        let mut delayed_expected = Vec::new();
        let mut old_banks: Vec<_> = (0..channels)
            .map(|_| aac_eld_cldfb::Analysis::default())
            .collect();
        let mut assembler = Assembler::default();
        let mut oracle_pcm = Vec::new();
        assert!(decoder.is_spatial());
        for (frame, row) in case["frames"].as_array().unwrap().iter().enumerate() {
            let at = number(&row["offset"]);
            let size = number(&row["bytes"]);
            let packet = &packets[at..at + size];
            let start = number(&row["core_bits"]);
            let core: Vec<Vec<f32>> = pcm
                .iter()
                .map(|p| p[frame * n..(frame + 1) * n].to_vec())
                .collect();
            let saved = decoder.clone();
            let (actual, used) = decoder
                .process_prefix_qmf(packet, start, packet.len() * 8, &core)
                .unwrap_or_else(|e| panic!("{} frame {frame}: {e}", case["name"]));
            assert_eq!(used, number(&row["sbr_bits"]));
            assert_eq!(actual.len(), channels);
            for ch in 0..channels {
                assert_eq!(actual[ch].len(), slots);
                let canonical = number(&case["mapping"][ch]);
                let old = old_banks[ch].process(&core[ch]).unwrap();
                for t in 0..slots {
                    assert_eq!(actual[ch][t].len(), bands);
                    for b in 0..bands {
                        let want = if b < stored {
                            let at = offset
                                + ((canonical * 12 * slots + frame * slots + t) * stored + b) * 16;
                            [
                                f64::from_le_bytes(expected[at..at + 8].try_into().unwrap()),
                                f64::from_le_bytes(expected[at + 8..at + 16].try_into().unwrap()),
                            ]
                        } else {
                            [0., 0.]
                        };
                        let got = actual[ch][t][b];
                        let delta = (got.re - want[0]).abs().max((got.im - want[1]).abs());
                        maximum = maximum.max(delta);
                        scalars += 2;
                        assert!(
                            delta < 2e-10,
                            "{} frame {frame} channel {ch} slot {t} band {b}: {:?} != {:?}; delta {delta}",
                            case["name"],
                            got,
                            want
                        );
                        if b < 10 {
                            wrong_bank_gap = wrong_bank_gap.max(
                                (got.re - old[t][b].re)
                                    .abs()
                                    .max((got.im - old[t][b].im).abs()),
                            );
                        }
                        if b >= 32 && case["mode"] == "wide" {
                            high_energy += got.re * got.re + got.im * got.im;
                        }
                    }
                }
            }
            if frame == 3 {
                let mut replay = saved;
                assert_eq!(
                    replay
                        .process_prefix_qmf(packet, start, packet.len() * 8, &core)
                        .unwrap(),
                    (actual.clone(), used)
                );
                assert_eq!(replay, decoder);
            }
            let mut tail = BitReader::new(packet);
            tail.skip(start + used).unwrap();
            let messages = read_eld_spatial_extensions(&mut tail, &mut assembler).unwrap();
            assert_eq!(messages.len(), 1);
            let mut qmf = vec![vec![vec![Default::default(); bands]; channels]; slots];
            let mut oracle = qmf.clone();
            for ch in 0..channels {
                let canonical = number(&case["mapping"][ch]);
                for t in 0..slots {
                    qmf[t][canonical] = actual[ch][t].clone();
                    for b in 0..bands {
                        if b < stored {
                            let at = offset
                                + ((canonical * 12 * slots + frame * slots + t) * stored + b) * 16;
                            oracle[t][canonical][b] = fvid_media::owned_aac::aac_sbr_qmf::Complex {
                                re: f64::from_le_bytes(expected[at..at + 8].try_into().unwrap()),
                                im: f64::from_le_bytes(
                                    expected[at + 8..at + 16].try_into().unwrap(),
                                ),
                            };
                        }
                    }
                }
            }
            let start = -11 + (frame * slots * bands * 1000 / number(&case["rate"])) as i64;
            let end = -11 + ((frame + 1) * slots * bands * 1000 / number(&case["rate"])) as i64;
            let duration = (end - start) as u64;
            let stamp = fvid_media::owned_aac::aac_ld_mps_session::Stamp {
                pts: start,
                duration,
                timescale: 1000,
            };
            let complete = whole_session
                .submit_qmf(Some((&qmf, stamp)), &messages)
                .unwrap();
            let count = slots / 2;
            let ticks = duration * count as u64 / slots as u64;
            let first = fvid_media::owned_aac::aac_ld_mps_session::Stamp {
                duration: ticks,
                ..stamp
            };
            assert!(
                split_session
                    .submit_qmf(Some((&qmf[..count], first)), &[])
                    .unwrap()
                    .is_empty()
            );
            let saved = split_session.clone();
            let mut invalid = qmf[count..].to_vec();
            invalid[0][0][0].re = f64::NAN;
            let second = fvid_media::owned_aac::aac_ld_mps_session::Stamp {
                pts: start + ticks as i64,
                duration: duration - ticks,
                ..stamp
            };
            assert!(
                split_session
                    .submit_qmf(Some((&invalid, second)), &messages)
                    .is_err()
            );
            assert_eq!(split_session, saved);
            let parts = split_session
                .submit_qmf(Some((&qmf[count..], second)), &messages)
                .unwrap();
            assert_eq!(parts, complete, "{} fragmented QMF clock", case["name"]);
            assert_eq!(complete[0].stamp, stamp);
            delayed_messages.extend(messages.clone());
            delayed_expected.extend(complete.clone());
            let ready = if frame % 3 == 2 {
                delayed_session
                    .submit_qmf(Some((&qmf, stamp)), &delayed_messages)
                    .unwrap()
            } else {
                delayed_session
                    .submit_qmf(Some((&qmf, stamp)), &[])
                    .unwrap()
            };
            if frame % 3 == 2 {
                assert_eq!(ready, delayed_expected, "{} late parameters", case["name"]);
                delayed_messages.clear();
                delayed_expected.clear();
                delayed_session.finish().unwrap();
            } else {
                assert!(ready.is_empty());
                assert_eq!(
                    delayed_session.queued_samples(),
                    (frame % 3 + 1) * slots * bands
                );
                assert!(delayed_session.finish().is_err());
            }

            split_session.finish().unwrap();
            let spatial = parsed.ld_mps.as_ref().unwrap();
            let residuals =
                vec![None; spatial.tree.ott_lfe.len() + usize::from(spatial.tree.ttt_boxes)];
            let a = spatial_actual
                .decode_qmf(&mut BitReader::new(&messages[0].payload), &qmf, &residuals)
                .unwrap();
            let b = spatial_expected
                .decode_qmf(
                    &mut BitReader::new(&messages[0].payload),
                    &oracle,
                    &residuals,
                )
                .unwrap();
            assert_eq!(
                a.samples.len(),
                slots * bands * usize::from(spatial.tree.output_channels)
            );
            for (&a, &b) in a.samples.iter().zip(&b.samples) {
                assert!(
                    (a - b).abs() < 2e-9,
                    "spatial handoff {} frame {frame}: {a} != {b}",
                    case["name"]
                );
            }
            let output_channels = usize::from(spatial.tree.output_channels);
            for row in b.samples.chunks_exact(output_channels) {
                for c in 0..output_channels {
                    let c = if matches!(spatial.tree.code, 4 | 6) {
                        [0, 1, 2, 3, 6, 7, 4, 5][c]
                    } else {
                        c
                    };
                    oracle_pcm.push(row[c]);
                }
            }
            assert_eq!(tail.remaining(), 0);
        }
        let movie =
            std::fs::read(r.join(format!("{}.mp4", case["name"].as_str().unwrap()))).unwrap();
        let mut native = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie, &mut native).unwrap();
        assert_eq!(native.len(), oracle_pcm.len() * 4);
        for (raw, expected) in native.chunks_exact(4).zip(oracle_pcm) {
            let actual = f64::from(f32::from_le_bytes(raw.try_into().unwrap()));
            assert!(
                (actual - expected).abs() < 2e-7,
                "{} native PCM: {actual} != {expected}",
                case["name"]
            );
        }
        decoder.reset();
        assert_eq!(decoder, fresh);
        programmes += 1;
    }
    assert_eq!(programmes, 152);
    assert!(wrong_bank_gap > 1e-8);
    assert!(high_energy > 1e-8);
    assert!(scalars > 1000000);
    eprintln!(
        "spatial LD-SBR: {programmes} programmes, {scalars} QMF scalars, maximum difference {maximum}; CLDFB negative-control gap {wrong_bank_gap}; upper-band energy {high_energy}"
    );
}
#[test]
fn malformed_crc_and_truncated_spatial_sbr_preserve_all_group_histories_and_cursor() {
    let r = root();
    let m: Value =
        serde_json::from_slice(&std::fs::read(r.join("manifest.json")).unwrap()).unwrap();
    let packets = std::fs::read(r.join("packets.bin")).unwrap();
    for case in m["errors"].as_array().unwrap() {
        let parsed = configuration(case);
        let mut decoder = Decoder::new_spatial(
            parsed.eld_sbr.clone().unwrap(),
            parsed.core.sample_rate,
            parsed.core.frame_samples,
            parsed.core.channel_configuration,
        )
        .unwrap();
        let pcm = input(case);
        let n = usize::from(parsed.core.frame_samples);
        for (frame, row) in case["frames"]
            .as_array()
            .unwrap()
            .iter()
            .take(4)
            .enumerate()
        {
            let at = number(&row["offset"]);
            let size = number(&row["bytes"]);
            let mut bits = BitReader::new(&packets[at..at + size]);
            let start = number(&row["core_bits"]);
            bits.skip(start).unwrap();
            let core: Vec<_> = pcm
                .iter()
                .map(|p| p[frame * n..(frame + 1) * n].to_vec())
                .collect();
            let saved = decoder.clone();
            let end = bits.position() + bits.remaining();
            if frame == 3 {
                let error = decoder
                    .process_qmf(&mut bits, end, &core)
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.contains(case["error"].as_str().unwrap()),
                    "{}: {error}",
                    case["name"]
                );
                assert_eq!(bits.position(), start);
                assert_eq!(decoder, saved);
            } else {
                decoder.process_qmf(&mut bits, end, &core).unwrap();
            }
        }
    }
}
#[test]
fn cldfb_and_spatial_api_domains_cannot_silently_fall_back_to_each_other() {
    let m: Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let case = &m["cases"][0];
    let parsed = configuration(case);
    let config = parsed.eld_sbr.unwrap();
    let mut cldfb = Decoder::new(config.clone(), 24000, parsed.core.frame_samples, 1).unwrap();
    let mut spatial = Decoder::new_spatial(config, 24000, parsed.core.frame_samples, 1).unwrap();
    let original = cldfb.clone();
    let saved = spatial.clone();
    let core = vec![vec![0.; usize::from(parsed.core.frame_samples)]];
    let mut bits = BitReader::new(&[]);
    assert!(
        cldfb
            .process_qmf(&mut bits, 0, &core)
            .unwrap_err()
            .to_string()
            .contains("CLDFB")
    );
    assert_eq!(cldfb, original);
    assert_eq!(bits.position(), 0);
    assert!(
        spatial
            .process(&mut bits, 0, &core)
            .unwrap_err()
            .to_string()
            .contains("requires QMF")
    );
    assert_eq!(spatial, saved);
    assert_eq!(bits.position(), 0);
}
#[test]
fn coupled_native_movies_play_instead_of_returning_only_the_core_downmix() {
    let m: Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in m["cases"].as_array().unwrap() {
        let movie =
            std::fs::read(root().join(format!("{}.mp4", case["name"].as_str().unwrap()))).unwrap();
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie, &mut pcm).unwrap();
        let parsed = configuration(case);
        assert_eq!(
            pcm.len(),
            number(&case["samples"]) * usize::from(parsed.output_channels()) * 4
        );
    }
}

fn play_with(
    reader: &mut dyn fvid::audio::AudioStream,
    decoder: &mut dyn fvid::audio::AudioDecode,
) -> Vec<u8> {
    let mut output = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
            .unwrap()
        {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                output.extend(pcm.data);
            }
        }
    }
    while let Some(frame) = decoder.finish_packet().unwrap() {
        if let Some(pcm) = reader
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            output.extend(pcm.data);
        }
    }
    assert!(decoder.finish_packet().unwrap().is_none());
    output
}
#[test]
fn coupled_playback_rewind_live_reset_seek_and_ranges_match_full_pcm() {
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let movie = std::fs::read(root().join(format!("{name}.mp4"))).unwrap();
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie, &mut full).unwrap();
        assert!(full.iter().any(|&b| b != 0), "{name}");
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&movie),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, full, "{name} owned");
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&movie), Default::default())
                .unwrap();
        let mut live = reader.make_decoder().unwrap();
        let width = usize::from(reader.channels()) * 4;
        assert_eq!(
            play_with(&mut reader, live.as_mut()),
            full,
            "{name} playback"
        );
        reader.rewind();
        live.reset();
        for _ in 0..2 {
            let packet = reader.next_packet().unwrap().unwrap();
            live.decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap();
        }
        reader.rewind();
        live.reset();
        assert_eq!(
            play_with(&mut reader, live.as_mut()),
            full,
            "{name} rewind/reset"
        );
        for target in [13, 512, 7, (full.len() / width) as i64] {
            let ticks = (i128::from(target) * i128::from(reader.timescale())
                / i128::from(reader.sample_rate())) as i64;
            let landed = reader.seek_to(ticks);
            let first = ((landed.max(0) as u128 * u128::from(reader.sample_rate()))
                .div_ceil(u128::from(reader.timescale())) as usize
                * width)
                .min(full.len());
            live.reset();
            assert_eq!(
                play_with(&mut reader, live.as_mut()),
                full[first..],
                "{name} seek {target}"
            );
        }
        let rate = u64::from(reader.sample_rate());
        for (from, to) in [(7, 23), (2, 19), (7, 23)] {
            let mut range = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm_interval(
                &movie,
                &mut range,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
            )
            .unwrap();
            let first = ((from * rate).div_ceil(1000) as usize * width).min(full.len());
            let last = ((to * rate).div_ceil(1000) as usize * width).min(full.len());
            assert_eq!(range, full[first..last], "{name} range");
        }
    }
}

#[test]
fn malformed_coupled_movies_reach_the_specific_sbr_error() {
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["errors"].as_array().unwrap() {
        let movie =
            std::fs::read(root().join(format!("{}.mp4", case["name"].as_str().unwrap()))).unwrap();
        let mut pcm = Vec::new();
        let error = fvid::native_media::decode_mp4_aac_pcm(&movie, &mut pcm)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(case["error"].as_str().unwrap()),
            "{}: {error}",
            case["name"]
        );
    }
}

#[test]
fn coupled_native_crc_truncation_and_checkpoint_retry_preserve_whole_decoder() {
    use fvid_media::owned_aac::NativeAacDecoder;
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for bad in manifest["errors"].as_array().unwrap() {
        let good = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| {
                c["n"] == bad["n"]
                    && c["configuration"] == 1
                    && c["mode"] == "base"
                    && c["bands"] == 64
                    && c["name"].as_str().unwrap().ends_with("crc1")
            })
            .unwrap();
        let asc = hex(good["asc"].as_str().unwrap());
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        let mut reference = NativeAacDecoder::new(&asc).unwrap();
        let n = number(&good["n"]) * 2;
        for i in 0..12 {
            let row = &good["frames"][i];
            let at = number(&row["offset"]);
            let packet = &blob[at..at + number(&row["bytes"])];
            let expected = reference
                .decode_timed(packet, (i * n) as i64, n as u64)
                .unwrap();
            if i == 3 {
                let row = &bad["frames"][i];
                let at = number(&row["offset"]);
                let saved = decoder.checkpoint();
                let before = decoder.retained_payload_bytes().unwrap();
                assert!(
                    decoder
                        .retained_payload_bytes_with_checkpoint(Some(&saved))
                        .unwrap()
                        >= before
                );
                let error = decoder
                    .decode_timed(
                        &blob[at..at + number(&row["bytes"])],
                        (i * n) as i64,
                        n as u64,
                    )
                    .unwrap_err()
                    .to_string();
                assert!(error.contains(bad["error"].as_str().unwrap()), "{error}");
                assert_eq!(decoder.retained_payload_bytes().unwrap(), before);
                assert_eq!(
                    decoder
                        .decode_timed(packet, (i * n) as i64, n as u64)
                        .unwrap(),
                    expected
                );
                decoder.restore(&saved).unwrap();
            }
            assert_eq!(
                decoder
                    .decode_timed(packet, (i * n) as i64, n as u64)
                    .unwrap(),
                expected
            );
        }
        assert_eq!(decoder.finish().unwrap(), reference.finish().unwrap());
        assert!(decoder.finish().unwrap().is_none());
    }
}
