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
fn er_sbr_matches_non_er_control_direct_qmf_and_replays_atomically() {
    let m: serde_json::Value = serde_json::from_slice(&bytes("aac-er-sbr.json")).unwrap();
    let blob = bytes("aac-er-sbr-packets.bin");
    let gold = bytes("aac-sbr-dsp-pcm.f64le");
    let packet = |r: &serde_json::Value| {
        let at = r["offset"].as_u64().unwrap() as usize;
        &blob[at..at + r["bytes"].as_u64().unwrap() as usize]
    };
    for c in m["cases"].as_array().unwrap() {
        let mut decoder = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut control = NativeAacDecoder::new(&hex(c["control_asc"].as_str().unwrap())).unwrap();
        let mut actual = vec![];
        for (i, r) in c["frames"].as_array().unwrap().iter().enumerate() {
            let saved = decoder.checkpoint();
            let error = decoder.decode(packet(&c["bad_frames"][i])).unwrap_err();
            assert!(
                error.to_string().contains("ER AAC SBR CRC is forbidden"),
                "{error}"
            );
            let pcm = decoder.decode(packet(r)).unwrap();
            assert_eq!(
                pcm,
                control.decode(packet(&c["control_frames"][i])).unwrap(),
                "{}",
                c["name"]
            );
            decoder.restore(&saved).unwrap();
            assert_eq!(pcm, decoder.decode(packet(r)).unwrap());
            actual.extend(pcm);
        }
        let at = c["pcm_offset"].as_u64().unwrap() as usize;
        let count = c["samples"].as_u64().unwrap() as usize;
        assert_eq!(actual.len(), count);
        for (i, v) in actual.iter().enumerate() {
            let x = f64::from_le_bytes(gold[at + i * 8..at + (i + 1) * 8].try_into().unwrap());
            assert!(
                (f64::from(*v) - x).abs() < 1e-7,
                "{} {i}: {v} vs {x}",
                c["name"]
            );
        }
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
fn er_sbr_player_ranges_rewind_seek_preserve_output_clock() {
    use fvid::audio::AudioStream;
    for c in
        serde_json::from_slice::<serde_json::Value>(&bytes("aac-er-sbr.json")).unwrap()["cases"]
            .as_array()
            .unwrap()
    {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut full).unwrap();
        let stride = 4;
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
                full[from as usize
                    * (c["container_rate"].as_u64().unwrap() as usize / 1000)
                    * stride
                    ..to as usize
                        * (c["container_rate"].as_u64().unwrap() as usize / 1000)
                        * stride]
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
fn er_sbr_public_owned_export_and_crc_refusal_preserve_sample_clock() {
    let m: serde_json::Value = serde_json::from_slice(&bytes("aac-er-sbr.json")).unwrap();
    for c in m["cases"].as_array().unwrap() {
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
        assert_eq!(public.len(), c["samples"].as_u64().unwrap() as usize * 4);
        let mut rejected = vec![];
        let error = fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["bad_video"]["file"].as_str().unwrap()),
            &mut rejected,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("ER AAC SBR CRC is forbidden"),
            "{error}"
        );
        assert!(rejected.is_empty());
    }
}
