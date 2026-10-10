use fvid_media::owned_aac::{
    NativeAacDecoder, aac_pair::ChannelPair, bits::BitReader, config::AacConfig,
};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("aac-ld-stereo.json")).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn ld_stereo_resilience_native_public_pcm_and_history_rollback_match_scalar() {
    let m = manifest();
    let blob = bytes("aac-ld-stereo-packets.bin");
    let gold = bytes("aac-ld-stereo-reference.f32le");
    let names: std::collections::BTreeSet<_> = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_owned())
        .collect();
    let mut expected = std::collections::BTreeSet::new();
    for n in [480, 512] {
        for flags in 0..8 {
            for mode in ["independent", "common-0", "common-1", "common-2"] {
                expected.insert(format!("{n}-{flags}-{mode}"));
            }
        }
    }
    assert_eq!(names, expected);
    for c in m["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let config = AacConfig::parse(&asc).unwrap();
        let flags = c["flags"].as_u64().unwrap();
        assert_eq!(
            (
                config.object_type,
                config.channels,
                config.frame_samples as u64
            ),
            (23, 2, c["n"].as_u64().unwrap())
        );
        assert_eq!(
            (
                config.section_data_resilience,
                config.scalefactor_data_resilience,
                config.spectral_data_resilience
            ),
            (flags & 4 != 0, flags & 2 != 0, flags & 1 != 0)
        );
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        let mut all = vec![];
        let retained = decoder.retained_payload_bytes().unwrap();
        for (index, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let size = row["bytes"].as_u64().unwrap() as usize;
            let raw = &blob[at..at + size];
            let saved = decoder.checkpoint();
            let mut pair_bits = BitReader::new(raw);
            pair_bits.skip(4).unwrap();
            ChannelPair::read_ld(&mut pair_bits, &config).unwrap();
            if let Some(bad) = row.get("malformed") {
                let at = bad["offset"].as_u64().unwrap() as usize;
                let size = bad["bytes"].as_u64().unwrap() as usize;
                let error = decoder
                    .decode(&blob[at..at + size])
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.contains(bad["error"].as_str().unwrap()),
                    "{}: {error}",
                    c["name"]
                );
                if flags != 0 {
                    let mut bits = BitReader::new(&blob[at..at + size]);
                    bits.skip(4).unwrap();
                    assert!(ChannelPair::read_ld(&mut bits, &config).is_err());
                    assert_eq!(bits.position(), 4);
                }
                for end in 0..raw.len() {
                    let mut bits = BitReader::new(&raw[..end]);
                    if bits.skip(4).is_ok() {
                        assert!(ChannelPair::read_ld(&mut bits, &config).is_err());
                        assert_eq!(bits.position(), 4);
                    }
                }
            }
            let pts = index as i64 * i64::from(config.frame_samples);
            let frame = decoder
                .decode_timed(raw, pts, u64::from(config.frame_samples))
                .unwrap()
                .unwrap();
            assert_eq!(
                (frame.pts, frame.duration),
                (pts, u64::from(config.frame_samples))
            );
            decoder.restore(&saved).unwrap();
            assert_eq!(frame.samples, decoder.decode(raw).unwrap());
            all.extend(frame.samples);
            assert_eq!(decoder.retained_payload_bytes().unwrap(), retained);
        }
        let start = c["reference_offset"].as_u64().unwrap() as usize;
        assert_eq!(
            all.len() * 4,
            c["reference_bytes"].as_u64().unwrap() as usize
        );
        for (i, (actual, raw)) in all
            .iter()
            .zip(gold[start..start + all.len() * 4].chunks_exact(4))
            .enumerate()
        {
            let expected = f32::from_le_bytes(raw.try_into().unwrap());
            assert!(
                (*actual - expected).abs() < 2e-7,
                "{} sample {i}: {actual} vs {expected}",
                c["name"]
            );
        }
        decoder.reset();
        let first = &c["frames"][0];
        let at = first["offset"].as_u64().unwrap() as usize;
        let size = first["bytes"].as_u64().unwrap() as usize;
        assert_eq!(
            &all[..usize::from(config.frame_samples) * 2],
            decoder.decode(&blob[at..at + size]).unwrap()
        );
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut public = vec![];
        let mut owned = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut public).unwrap();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&data),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(public, owned);
        assert_eq!(
            public,
            all.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>()
        );
    }
}
#[test]
fn ld_stereo_malformed_videos_refuse_the_specific_right_channel_failure() {
    let m = manifest();
    assert_eq!(m["malformed"].as_array().unwrap().len(), 64);
    for c in m["malformed"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut out = vec![];
        let error = fvid::native_media::decode_mp4_aac_pcm(&data, &mut out)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(c["error"].as_str().unwrap()),
            "{}: {error}",
            c["video"]["file"]
        );
    }
}
#[cfg(feature = "player")]
fn play(reader: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut output = vec![];
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
#[cfg(feature = "player")]
#[test]
fn ld_stereo_player_ranges_rewind_seek_and_channel_order_preserve_pcm() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut full).unwrap();
        for (from, to) in [(30, 120), (10, 60), (30, 120)] {
            let mut out = vec![];
            fvid::native_media::decode_mp4_aac_pcm_interval(
                &data,
                &mut out,
                Some((
                    std::time::Duration::from_millis(from),
                    std::time::Duration::from_millis(to),
                )),
            )
            .unwrap();
            assert_eq!(out, full[from as usize * 24 * 8..to as usize * 24 * 8]);
        }
        let mut reader = fvid::playback_mp4_audio::Mp4AudioReader::open(
            std::io::Cursor::new(&data),
            Default::default(),
        )
        .unwrap();
        assert_eq!((reader.channels(), reader.sample_rate()), (2, 24000));
        assert_eq!(play(&mut reader), full);
        reader.rewind();
        assert_eq!(play(&mut reader), full);
        for target in [13, 512, 2500, (full.len() / 8) as i64] {
            let landed = reader.seek_to(target);
            assert_eq!(play(&mut reader), full[landed as usize * 8..]);
        }
    }
}
