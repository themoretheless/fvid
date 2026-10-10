use fvid_media::owned_aac::NativeAacDecoder;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("aac-opaque-extensions.json")).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn reference(c: &serde_json::Value, raw: &[u8]) -> Vec<f32> {
    let start = c["reference_offset"].as_u64().unwrap() as usize;
    let size = c["reference_bytes"].as_u64().unwrap() as usize;
    raw[start..start + size]
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect()
}
#[test]
fn opaque_extensions_preserve_pcm_and_malformed_ancillary_is_transactional() {
    let m = manifest();
    let blob = bytes("aac-opaque-extensions-packets.bin");
    let gold = bytes("aac-opaque-extensions-reference.f32le");
    assert_eq!(m["cases"].as_array().unwrap().len(), 10);
    let packet = |row: &serde_json::Value| {
        let at = row["offset"].as_u64().unwrap() as usize;
        &blob[at..at + row["bytes"].as_u64().unwrap() as usize]
    };
    for c in m["cases"].as_array().unwrap() {
        let mut decoder = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut actual = vec![];
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let saved = decoder.checkpoint();
            let error = decoder.decode(packet(&c["bad_frames"][i])).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("AAC ancillary data exceeds fill payload"),
                "{}: {error}",
                c["name"]
            );
            let sac_error = decoder.decode(packet(&c["sac_frames"][i])).unwrap_err();
            assert!(
                sac_error
                    .to_string()
                    .contains("AAC fill extension tool is not implemented"),
                "{sac_error}"
            );
            let pcm = decoder.decode(packet(row)).unwrap();
            decoder.restore(&saved).unwrap();
            assert_eq!(
                pcm,
                decoder.decode(packet(&c["control_frames"][i])).unwrap()
            );
            decoder.restore(&saved).unwrap();
            assert_eq!(pcm, decoder.decode(packet(row)).unwrap());
            actual.extend(pcm);
        }
        let expected = reference(c, &gold);
        assert_eq!(actual.len(), expected.len());
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(a, b)| (*a - b).abs() < 1e-8),
            "{}",
            c["name"]
        );
        let mut output = vec![];
        let error = fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["bad_video"]["file"].as_str().unwrap()),
            &mut output,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("AAC ancillary data exceeds fill payload"),
            "{error}"
        );
        assert!(output.is_empty());
    }
}
#[test]
fn opaque_extensions_public_videos_match_controls_and_owned_export() {
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let control = bytes(c["control_video"]["file"].as_str().unwrap());
        let mut out = vec![];
        let mut expected = vec![];
        let mut owned = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut out).unwrap();
        fvid::native_media::decode_mp4_aac_pcm(&control, &mut expected).unwrap();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&data),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(out, expected, "{}", c["name"]);
        assert_eq!(out, owned);
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
fn opaque_extensions_player_ranges_rewind_seek_restore_noise_and_predictor_state() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut full).unwrap();
        let stride = c["channels"].as_u64().unwrap() as usize * 4;
        for (from, to) in [(10, 80), (1, 20), (10, 80)] {
            let mut range = vec![];
            fvid::native_media::decode_mp4_aac_pcm_interval(
                &data,
                &mut range,
                Some((
                    std::time::Duration::from_millis(from),
                    std::time::Duration::from_millis(to),
                )),
            )
            .unwrap();
            assert_eq!(
                range,
                full[from as usize * 24 * stride..to as usize * 24 * stride]
            );
        }
        let mut reader = fvid::playback_mp4_audio::Mp4AudioReader::open(
            std::io::Cursor::new(&data),
            Default::default(),
        )
        .unwrap();
        assert_eq!(play(&mut reader), full);
        reader.rewind();
        assert_eq!(play(&mut reader), full);
        for target in [13, 1400, 3500, (full.len() / stride) as i64] {
            let landed = reader.seek_to(target);
            assert_eq!(
                play(&mut reader),
                full[landed as usize * stride..],
                "{}",
                c["name"]
            );
        }
    }
}

#[test]
fn recognized_mpeg_surround_remains_a_specific_refusal() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut output = vec![];
        let error = fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["sac_video"]["file"].as_str().unwrap()),
            &mut output,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("AAC fill extension tool is not implemented"),
            "{}: {error}",
            c["name"]
        );
        assert!(output.is_empty());
    }
}
