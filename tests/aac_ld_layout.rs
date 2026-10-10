use fvid_media::owned_aac::{NativeAacDecoder, config::AacConfig};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("aac-ld-layout.json")).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn ld_indexed_explicit_rates_all_layouts_and_final_bands_match_scalar_pcm() {
    let m = manifest();
    let blob = bytes("aac-ld-layout-packets.bin");
    let gold = bytes("aac-ld-layout-reference.f32le");
    let names: std::collections::BTreeSet<_> = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_owned())
        .collect();
    let mut expected = std::collections::BTreeSet::new();
    for n in [480, 512] {
        for rate in [22050, 24000, 32000, 44100, 48000, 27713, 37566, 46009] {
            for config in [1, 2, 3, 4, 5, 6, 7, 11, 12, 14] {
                expected.insert(format!("{n}-{rate}-{config}"));
            }
        }
    }
    assert_eq!(names, expected);
    for c in m["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let config = AacConfig::parse(&asc).unwrap();
        let rate = c["rate"].as_u64().unwrap() as u32;
        let width = c["channels"].as_u64().unwrap() as usize;
        assert_eq!(
            (
                config.object_type,
                config.sample_rate,
                config.channels as usize,
                config.frame_samples as u64,
                config.channel_configuration as u64
            ),
            (
                23,
                rate,
                width,
                c["n"].as_u64().unwrap(),
                c["configuration"].as_u64().unwrap()
            )
        );
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        assert_eq!(decoder.channel_mask(), c["mask"].as_u64().unwrap() as u32);
        let retained = decoder.retained_payload_bytes().unwrap();
        let mut all = vec![];
        for (index, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let size = row["bytes"].as_u64().unwrap() as usize;
            let raw = &blob[at..at + size];
            let saved = decoder.checkpoint();
            assert!(decoder.decode(&raw[..raw.len() / 2]).is_err());
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
                (*actual - expected).abs() < 1e-10,
                "{} sample {i}: {actual} vs {expected}",
                c["name"]
            );
        }
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut public = vec![];
        let mut owned = vec![];
        let stats = fvid::native_media::decode_mp4_aac_pcm(&data, &mut public).unwrap();
        assert_eq!(
            (
                stats.channels as usize,
                stats.sample_rate,
                stats.sample_frames
            ),
            (width, rate, c["samples"].as_u64().unwrap())
        );
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
fn ld_rate_layout_player_ranges_rewind_seek_preserve_channel_order_and_sample_clock() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut full).unwrap();
        let rate = c["rate"].as_u64().unwrap();
        let width = c["channels"].as_u64().unwrap() as usize;
        let stride = width * 4;
        for (from, to) in [(10, 50), (2, 15), (10, 50)] {
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
            let begin = (rate * from).div_ceil(1000) as usize;
            let end = (rate * to).div_ceil(1000) as usize;
            assert_eq!(out, full[begin * stride..end * stride], "{}", c["name"]);
        }
        let mut reader = fvid::playback_mp4_audio::Mp4AudioReader::open(
            std::io::Cursor::new(&data),
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            (reader.channels() as usize, reader.sample_rate() as u64),
            (width, rate)
        );
        assert_eq!(play(&mut reader), full);
        reader.rewind();
        assert_eq!(play(&mut reader), full);
        for target in [13, 512, 2500, (full.len() / stride) as i64] {
            let landed = reader.seek_to(target);
            assert_eq!(
                play(&mut reader),
                full[landed as usize * stride..],
                "{} target={target}",
                c["name"]
            );
        }
    }
}
