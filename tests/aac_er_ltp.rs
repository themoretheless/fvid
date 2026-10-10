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
    serde_json::from_slice(&bytes("aac-er-ltp.json")).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn er_ltp_native_transitions_prediction_and_stereo_match_scalar_pcm() {
    let blob = bytes("aac-er-ltp-packets.bin");
    let gold = bytes("aac-er-ltp-reference.f32le");
    for c in manifest()["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        assert_eq!(
            fvid_media::owned_aac::config::AacConfig::parse(&asc)
                .unwrap()
                .object_type,
            19
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
                let mut padded = raw.to_vec();
                padded.push(0); // Valid ER EXT_FILL, rather than trailing garbage.
                let padded_pcm = decoder.decode(&padded).unwrap();
                decoder.restore(&saved).unwrap();
                assert!(decoder.decode(&raw[..raw.len() - 1]).is_err());
                let pcm = decoder.decode(raw).unwrap();
                assert_eq!(padded_pcm, pcm);
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
fn er_ltp_public_mp4_acceptance_matches_owned_pcm() {
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
        let gold = bytes("aac-er-ltp-reference.f32le");
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
fn er_ltp_player_ranges_rewind_seek_preserve_pcm_and_channels() {
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
fn er_ltp_malformed_predictors_restore_history_and_pair_cursor() {
    use fvid_media::owned_aac::{
        aac_channel::ChannelData, aac_pair::ChannelPair, bits::BitReader, config::AacConfig,
    };
    let blob = bytes("aac-er-ltp-packets.bin");
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
fn er_ltp_deferred_predictors_match_each_channel_and_truncation_is_transactional() {
    use fvid_media::owned_aac::{
        aac_channel::ChannelData, aac_pair::ChannelPair, bits::BitReader, config::AacConfig,
    };
    let blob = bytes("aac-er-ltp-packets.bin");
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
