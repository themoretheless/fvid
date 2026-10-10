use fvid_media::owned_aac::NativeAacDecoder;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn er_large_sbr_region_matches_individually_bounded_fil_and_truncation_rolls_back() {
    let m: serde_json::Value = serde_json::from_slice(&bytes("aac-er-large-sbr.json")).unwrap();
    let blob = bytes("aac-er-large-sbr-packets.bin");
    let packet = |r: &serde_json::Value| {
        let at = r["offset"].as_u64().unwrap() as usize;
        &blob[at..at + r["bytes"].as_u64().unwrap() as usize]
    };
    for c in m["cases"].as_array().unwrap() {
        let mut control = NativeAacDecoder::new(&hex(c["control_asc"].as_str().unwrap())).unwrap();
        let expected: Vec<_> = c["control_frames"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| control.decode(packet(r)).unwrap())
            .collect();
        assert!(expected.iter().any(|v| v.abs() > 1e-6));
        let mut decoder = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut actual = vec![];
        for (i, r) in c["frames"].as_array().unwrap().iter().enumerate() {
            let saved = decoder.checkpoint();
            let pcm = decoder.decode(packet(r)).unwrap();
            decoder.restore(&saved).unwrap();
            let error = decoder.decode(packet(&c["bad_frames"][i])).unwrap_err();
            assert!(error.to_string().contains("truncated SBR"), "{error}");
            assert_eq!(pcm, decoder.decode(packet(r)).unwrap());
            actual.extend(pcm);
        }
        assert_eq!(actual, expected, "{}", c["name"]);
        let mut out = vec![];
        fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["video"]["file"].as_str().unwrap()),
            &mut out,
        )
        .unwrap();
        assert_eq!(
            out,
            actual
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
        );
        let mut owned = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(bytes(c["video"]["file"].as_str().unwrap())),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, out);
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
fn large_er_sbr_player_ranges_rewind_seek_restore_state() {
    use fvid::audio::AudioStream;
    for c in serde_json::from_slice::<serde_json::Value>(&bytes("aac-er-large-sbr.json")).unwrap()["cases"].as_array().unwrap() {
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
                full[from as usize * 48 * stride..to as usize * 48 * stride]
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
