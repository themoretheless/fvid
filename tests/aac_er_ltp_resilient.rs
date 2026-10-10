use std::path::Path;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}

use fvid_media::owned_aac::NativeAacDecoder;
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("aac-er-ltp-resilient.json")).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn er_ltp_resilient_native_transitions_prediction_and_stereo_match_scalar_pcm() {
    let blob = bytes("aac-er-ltp-resilient-packets.bin");
    let gold = bytes("aac-er-ltp-resilient-reference.f32le");
    let m = manifest();
    let actual: std::collections::BTreeSet<_> = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_owned())
        .collect();
    let mut expected = std::collections::BTreeSet::new();
    for n in [960, 1024] {
        for flags in 1..8 {
            for mode in ["mono", "independent", "common-0", "common-1", "common-2"] {
                expected.insert(format!("{n}-{flags}-{mode}"));
            }
        }
    }
    assert_eq!(
        actual, expected,
        "the complete resilience combination corpus must remain present"
    );
    for c in m["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        assert_eq!(
            fvid_media::owned_aac::config::AacConfig::parse(&asc)
                .unwrap()
                .object_type,
            19
        );
        let config = fvid_media::owned_aac::config::AacConfig::parse(&asc).unwrap();
        let flags = c["flags"].as_u64().unwrap();
        assert_eq!(
            (
                config.section_data_resilience,
                config.scalefactor_data_resilience,
                config.spectral_data_resilience
            ),
            (flags & 4 != 0, flags & 2 != 0, flags & 1 != 0)
        );
        let mut first = vec![];
        for pass in 0..2 {
            if pass != 0 {
                decoder.reset();
            }
            let mut out = vec![];
            for row in c["frames"].as_array().unwrap() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let len = row["bytes"].as_u64().unwrap() as usize;
                let raw = &blob[at..at + len];
                let saved = decoder.checkpoint();
                let mut bad = raw.to_vec();
                bad.push(0);
                assert!(
                    decoder
                        .decode(&bad)
                        .err()
                        .unwrap()
                        .to_string()
                        .contains("trailing bytes after ER AAC block")
                );
                assert!(decoder.decode(&raw[..raw.len() - 1]).is_err());
                let pcm = decoder.decode(raw).unwrap();
                decoder.restore(&saved).unwrap();
                assert_eq!(decoder.decode(raw).unwrap(), pcm);
                out.extend(pcm);
            }
            let at = c["reference_offset"].as_u64().unwrap() as usize;
            let count = c["reference_bytes"].as_u64().unwrap() as usize;
            assert_eq!(out.len() * 4, count);
            for (i, (a, b)) in out
                .iter()
                .zip(gold[at..at + count].chunks_exact(4))
                .enumerate()
            {
                let b = f32::from_le_bytes(b.try_into().unwrap());
                assert!((a - b).abs() < 1e-7, "{} sample {i}: {a} vs {b}", c["name"]);
            }
            if pass == 0 {
                first = out;
            } else {
                assert_eq!(out, first);
            }
        }
    }
}
#[test]
fn er_ltp_resilient_public_mp4_acceptance_matches_owned_pcm() {
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut out = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut out).unwrap();
        let mut owned = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&data),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(out, owned);
        assert_eq!(out.len(), c["reference_bytes"].as_u64().unwrap() as usize);
        let gold = bytes("aac-er-ltp-resilient-reference.f32le");
        let at = c["reference_offset"].as_u64().unwrap() as usize;
        for (a, b) in out
            .chunks_exact(4)
            .zip(gold[at..at + out.len()].chunks_exact(4))
        {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7
            );
        }
    }
}

use fvid::audio::AudioStream;
fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
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
fn er_ltp_resilient_player_ranges_rewind_seek_preserve_pcm_and_channels() {
    use std::{io::Cursor, time::Duration};
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut full).unwrap();
        let stride = c["channels"].as_u64().unwrap() as usize * 4;
        for (from, to) in [(130, 230), (10, 60), (130, 230)] {
            let mut out = vec![];
            fvid::native_media::decode_mp4_aac_pcm_interval(
                &data,
                &mut out,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
            )
            .unwrap();
            assert_eq!(
                out,
                full[from as usize * 24 * stride..to as usize * 24 * stride]
            );
        }
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(
            (reader.channels(), reader.sample_rate()),
            (c["channels"].as_u64().unwrap() as u16, 24000)
        );
        assert_eq!(play(&mut reader), full);
        reader.rewind();
        assert_eq!(play(&mut reader), full);
        for target in [1100, 5800, 9000, (full.len() / stride) as i64] {
            let landed = reader.seek_to(target);
            assert_eq!(play(&mut reader), full[landed as usize * stride..]);
        }
    }
}
#[test]
fn er_ltp_resilient_malformed_predictors_restore_history_and_pair_cursor() {
    use fvid_media::owned_aac::{
        aac_channel::ChannelData, aac_pair::ChannelPair, bits::BitReader, config::AacConfig,
    };
    let blob = bytes("aac-er-ltp-resilient-packets.bin");
    for c in manifest()["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let config = AacConfig::parse(&asc).unwrap();
        let mut control = NativeAacDecoder::new(&asc).unwrap();
        let mut trial = NativeAacDecoder::new(&asc).unwrap();
        for row in c["frames"].as_array().unwrap() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            let raw = &blob[at..at + len];
            if let Some(bad) = row.get("malformed") {
                let at = bad["offset"].as_u64().unwrap() as usize;
                let len = bad["bytes"].as_u64().unwrap() as usize;
                let data = &blob[at..at + len];
                let e = trial
                    .decode(data)
                    .err()
                    .unwrap_or_else(|| panic!("accepted malformed {}", c["name"]));
                assert!(
                    e.to_string().contains(bad["error"].as_str().unwrap()),
                    "{}: {e}",
                    c["name"]
                );
                let mut bits = BitReader::new(data);
                bits.skip(4).unwrap();
                let start = bits.position();
                let rejected = if config.channels == 1 {
                    ChannelData::read_ltp(&mut bits, &config).is_err()
                } else {
                    ChannelPair::read_ltp(&mut bits, &config).is_err()
                };
                assert!(rejected);
                assert_eq!(bits.position(), start);
            }
            assert_eq!(
                trial.decode(raw).unwrap(),
                control.decode(raw).unwrap(),
                "{}",
                c["name"]
            );
        }
    }
    for c in manifest()["malformed"].as_array().unwrap() {
        let mut out = vec![];
        let e = fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["video"]["file"].as_str().unwrap()),
            &mut out,
        )
        .unwrap_err();
        assert!(e.to_string().contains(c["error"].as_str().unwrap()), "{e}");
    }
}
#[test]
fn er_ltp_resilient_deferred_predictors_match_each_channel_and_truncation_is_transactional() {
    use fvid_media::owned_aac::{
        aac_channel::ChannelData, aac_pair::ChannelPair, bits::BitReader, config::AacConfig,
    };
    let blob = bytes("aac-er-ltp-resilient-packets.bin");
    for c in manifest()["cases"].as_array().unwrap() {
        let config = AacConfig::parse(&hex(c["asc"].as_str().unwrap())).unwrap();
        for row in c["frames"].as_array().unwrap() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            let raw = &blob[at..at + len];
            let mut bits = BitReader::new(raw);
            bits.skip(4).unwrap();
            let present: Vec<_> = if config.channels == 1 {
                vec![
                    ChannelData::read_ltp(&mut bits, &config)
                        .unwrap()
                        .1
                        .is_some(),
                ]
            } else {
                ChannelPair::read_ltp(&mut bits, &config)
                    .unwrap()
                    .1
                    .into_iter()
                    .map(|x| x.is_some())
                    .collect()
            };
            assert_eq!(
                present,
                row["active"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_bool().unwrap())
                    .collect::<Vec<_>>(),
                "{}",
                c["name"]
            );
            assert!(bits.remaining() <= 7);
            for end in 1..raw.len() {
                let mut bits = BitReader::new(&raw[..end]);
                bits.skip(4).unwrap();
                let failed = if config.channels == 1 {
                    ChannelData::read_ltp(&mut bits, &config).is_err()
                } else {
                    ChannelPair::read_ltp(&mut bits, &config).is_err()
                };
                assert!(failed, "{} prefix={end}", c["name"]);
                assert_eq!(bits.position(), 4);
            }
        }
    }
}

#[test]
fn independent_window_cpe_uses_pair_hcr_lengths_above_6144_and_matches_pcm() {
    use fvid_media::owned_aac::{
        aac_channel::ChannelData, aac_pair::ChannelPair, bits::BitReader, config::AacConfig,
    };
    use std::io::Cursor;
    let blob = bytes("aac-er-ltp-resilient-packets.bin");
    let gold = bytes("aac-er-ltp-resilient-reference.f32le");
    for c in manifest()["large_cpe"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let config = AacConfig::parse(&asc).unwrap();
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        let mut decoded = vec![];
        for row in c["frames"].as_array().unwrap() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            let raw = &blob[at..at + len];
            let mut bits = BitReader::new(raw);
            bits.skip(4).unwrap();
            let (_, prediction) = ChannelPair::read_ltp(&mut bits, &config).unwrap();
            assert!(prediction.iter().all(Option::is_none));
            assert!(bits.remaining() <= 7);
            if row["sequence"].as_u64().unwrap() == 2 {
                assert!(raw.len() * 8 <= 12288, "CPE total buffer budget");
                let large = c["large_channel"].as_u64().unwrap() as usize;
                assert_eq!(row["hcr_lengths"][large].as_u64().unwrap(), 10192);
                assert!(row["hcr_lengths"][1 - large].as_u64().unwrap() < 6144);
                // Reproduce the former CPE path by parsing its channels as SCEs.
                let mut former = BitReader::new(raw);
                former.skip(5).unwrap();
                let failure = match ChannelData::read_ltp(&mut former, &config) {
                    Err(error) => error,
                    Ok(_) => ChannelData::read_ltp(&mut former, &config).err().unwrap(),
                };
                assert!(
                    failure
                        .to_string()
                        .contains("AAC HCR incomplete nonpriority codeword"),
                    "{failure}"
                );
                assert!(
                    row["hcr_lengths"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|x| x.as_u64().unwrap() > 6144)
                );
                for end in [raw.len() / 2, raw.len() - 1] {
                    let mut bad = BitReader::new(&raw[..end]);
                    bad.skip(4).unwrap();
                    assert!(ChannelPair::read_ltp(&mut bad, &config).is_err());
                    assert_eq!(bad.position(), 4);
                }
            }
            decoded.extend(decoder.decode(raw).unwrap());
        }
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut exported = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut exported).unwrap();
        let mut owned = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(exported, owned);
        let at = c["reference_offset"].as_u64().unwrap() as usize;
        let len = c["reference_bytes"].as_u64().unwrap() as usize;
        assert_eq!(decoded.len() * 4, len);
        assert_eq!(exported.len(), len);
        for (i, ((actual, raw), expected)) in decoded
            .iter()
            .zip(exported.chunks_exact(4))
            .zip(gold[at..at + len].chunks_exact(4))
            .enumerate()
        {
            let expected = f32::from_le_bytes(expected.try_into().unwrap());
            assert_eq!(*actual, f32::from_le_bytes(raw.try_into().unwrap()));
            assert!(
                (actual - expected).abs() < 1e-7,
                "{} sample={i} {actual} vs {expected}",
                c["name"]
            );
        }
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(play(&mut reader), exported);
        reader.rewind();
        assert_eq!(play(&mut reader), exported);
        for target in [1100, 2100] {
            let landed = reader.seek_to(target);
            assert_eq!(play(&mut reader), exported[landed as usize * 8..]);
        }
    }
}
